use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::{process::Command, time::timeout};

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
const READINESS_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_DETAIL_BYTES: usize = 2_048;
const MAX_READINESS_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub required: bool,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub install_supported: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadinessSnapshot {
    pub scanned_at_unix_seconds: u64,
    pub platform: &'static str,
    pub tools_ready: bool,
    pub core: Option<Value>,
    pub core_error: Option<String>,
    pub tools: Vec<ToolStatus>,
}

pub async fn scan(app: &AppHandle) -> ReadinessSnapshot {
    // These probes are independent and each has its own bounded timeout. Run
    // them together so first-launch readiness is limited by the slowest local
    // tool instead of the sum of every probe.
    let (bundled_version, uv, connector, rust) = tokio::join!(
        sidecar_output(app, &["--version"], VERSION_TIMEOUT),
        tool_status("uv", "uv", &["uv"], true, uv_install_supported()),
        connector_status(),
        tool_status("rust", "Rust toolchain", &["rustc"], false, false),
    );
    let cortana = if let Ok(version) = &bundled_version {
        ToolStatus {
            id: "cortana",
            label: "Cortana runtime",
            required: true,
            available: true,
            path: Some("bundled sidecar".into()),
            version: Some(bounded_output(&version.stdout)),
            install_supported: false,
            detail: "Cryptographically bound to this desktop release.".into(),
        }
    } else {
        tool_status("cortana", "Cortana runtime", &["cortana"], true, false).await
    };
    let python = python_status(uv.available).await;
    let tools = vec![cortana.clone(), uv, python, connector, rust];
    let (core, core_error) = if bundled_version.is_ok() {
        match sidecar_readiness(app).await {
            Ok(report) => (Some(report), None),
            Err(error) => (None, Some(error)),
        }
    } else if let Some(path) = cortana.path.as_deref() {
        match core_readiness(path).await {
            Ok(report) => (Some(report), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (
            None,
            Some("Install the Cortana runtime before running production checks.".into()),
        )
    };

    ReadinessSnapshot {
        scanned_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        platform: std::env::consts::OS,
        tools_ready: tools
            .iter()
            .filter(|tool| tool.required)
            .all(|tool| tool.available),
        core,
        core_error,
        tools,
    }
}

async fn sidecar_readiness(app: &AppHandle) -> Result<Value, String> {
    let output = sidecar_output(app, &["readiness"], READINESS_TIMEOUT).await?;
    parse_readiness_output(&output.stdout, &output.stderr)
}

async fn sidecar_output(
    app: &AppHandle,
    args: &[&str],
    duration: Duration,
) -> Result<tauri_plugin_shell::process::Output, String> {
    let command = app
        .shell()
        .sidecar("cortana")
        .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
        .args(args);
    timeout(duration, command.output())
        .await
        .map_err(|_| "bundled Cortana command timed out".to_string())?
        .map_err(|error| format!("run bundled Cortana runtime: {error}"))
}

async fn tool_status(
    id: &'static str,
    label: &'static str,
    candidates: &[&str],
    required: bool,
    install_supported: bool,
) -> ToolStatus {
    let path = candidates
        .iter()
        .find_map(|candidate| find_executable(candidate));
    let version = match path.as_deref() {
        Some(path) => command_version(path).await,
        None => None,
    };
    ToolStatus {
        id,
        label,
        required,
        available: path.is_some(),
        path: path.as_ref().map(|path| path.display().to_string()),
        version,
        install_supported,
        detail: if let Some(path) = &path {
            format!("Found {}", path.display())
        } else if required {
            "Required for the local ingestion runtime.".into()
        } else {
            "Optional; only needed to build Cortana from source.".into()
        },
    }
}

async fn connector_status() -> ToolStatus {
    let path = connector_candidates()
        .into_iter()
        .find(|candidate| is_executable(candidate));
    let version = match path.as_deref() {
        Some(path) => command_version(path).await,
        None => None,
    };
    ToolStatus {
        id: "connectors",
        label: "Connector environment",
        required: true,
        available: path.is_some(),
        path: path.as_ref().map(|path| path.display().to_string()),
        version,
        install_supported: false,
        detail: path
            .map(|path| format!("Found {}", path.display()))
            .unwrap_or_else(|| {
                "Install the Cortana ingestion workspace after uv is available.".into()
            }),
    }
}

fn connector_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(prefix) = std::env::var_os("CORTANA_INSTALL_PREFIX") {
        candidates.push(PathBuf::from(prefix).join(connector_relative_path()));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local").join(connector_relative_path()));
    }
    candidates
}

#[cfg(windows)]
fn connector_relative_path() -> &'static str {
    "share/cortana/venv/Scripts/cortana-connectors.exe"
}

#[cfg(not(windows))]
fn connector_relative_path() -> &'static str {
    "share/cortana/venv/bin/cortana-connectors"
}

async fn core_readiness(path: &str) -> Result<Value, String> {
    let output = timeout(
        READINESS_TIMEOUT,
        Command::new(path)
            .arg("readiness")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Cortana readiness exceeded 90 seconds".to_string())?
    .map_err(|error| format!("start Cortana readiness: {error}"))?;
    parse_readiness_output(&output.stdout, &output.stderr)
}

fn parse_readiness_output(stdout: &[u8], stderr: &[u8]) -> Result<Value, String> {
    if stdout.len() > MAX_READINESS_BYTES {
        return Err("Cortana readiness response exceeded 64 KiB".into());
    }
    let body = String::from_utf8_lossy(stdout);
    serde_json::from_str(&body).map_err(|error| {
        let detail = bounded_output(stderr);
        format!("Cortana readiness returned invalid JSON: {error}; {detail}")
    })
}

async fn python_status(install_supported: bool) -> ToolStatus {
    let candidates = ["python3.13", "python3.12", "python3.11", "python3"];
    for candidate in candidates {
        let Some(path) = find_executable(candidate) else {
            continue;
        };
        let version = command_version(&path).await;
        if version.as_deref().is_some_and(python_version_supported) {
            return ToolStatus {
                id: "python",
                label: "Python 3.11+",
                required: true,
                available: true,
                path: Some(path.display().to_string()),
                version,
                install_supported,
                detail: format!("Found {}", path.display()),
            };
        }
    }
    if let Some(path) = uv_managed_python().await {
        let version = command_version(&path).await;
        if version.as_deref().is_some_and(python_version_supported) {
            return ToolStatus {
                id: "python",
                label: "Python 3.11+",
                required: true,
                available: true,
                path: Some(path.display().to_string()),
                version,
                install_supported,
                detail: format!("Found uv-managed interpreter at {}", path.display()),
            };
        }
    }
    ToolStatus {
        id: "python",
        label: "Python 3.11+",
        required: true,
        available: false,
        path: None,
        version: None,
        install_supported,
        detail: "Python 3.11 or newer is required for the ingestion runtime.".into(),
    }
}

async fn uv_managed_python() -> Option<PathBuf> {
    let uv = find_executable("uv")?;
    let output = timeout(
        VERSION_TIMEOUT,
        Command::new(uv)
            .args(["python", "find", "3.11"])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let path = parse_uv_python_path(&output.stdout)?;
    is_executable(&path).then_some(path)
}

fn parse_uv_python_path(bytes: &[u8]) -> Option<PathBuf> {
    let output = String::from_utf8_lossy(bytes);
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let path = PathBuf::from(line);
    path.is_absolute().then_some(path)
}

fn python_version_supported(value: &str) -> bool {
    let Some(version) = value.split_whitespace().find(|part| {
        part.chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
    }) else {
        return false;
    };
    let mut segments = version.split('.');
    matches!(
        (
            segments.next().and_then(|part| part.parse::<u32>().ok()),
            segments.next().and_then(|part| part.parse::<u32>().ok()),
        ),
        (Some(major), Some(minor)) if major > 3 || (major == 3 && minor >= 11)
    )
}

async fn command_version(path: &Path) -> Option<String> {
    let output = timeout(
        VERSION_TIMEOUT,
        Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let value = if output.stdout.is_empty() {
        bounded_output(&output.stderr)
    } else {
        bounded_output(&output.stdout)
    };
    (!value.is_empty()).then_some(value)
}

fn bounded_output(bytes: &[u8]) -> String {
    let end = bytes.len().min(MAX_DETAIL_BYTES);
    String::from_utf8_lossy(&bytes[..end])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return is_executable(candidate).then(|| candidate.to_path_buf());
    }
    executable_search_paths()
        .into_iter()
        .flat_map(|directory| executable_names(name).map(move |name| directory.join(name)))
        .find(|candidate| is_executable(candidate))
}

fn executable_search_paths() -> Vec<PathBuf> {
    let mut paths = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        for path in [home.join(".local/bin"), home.join(".cargo/bin")] {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

#[cfg(not(windows))]
fn executable_names(name: &str) -> impl Iterator<Item = OsString> {
    vec![OsString::from(name)].into_iter()
}

#[cfg(windows)]
fn executable_names(name: &str) -> impl Iterator<Item = OsString> {
    let mut names = vec![OsString::from(name)];
    if Path::new(name).extension().is_none() {
        names.push(OsString::from(format!("{name}.exe")));
        names.push(OsString::from(format!("{name}.cmd")));
    }
    names.into_iter()
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn uv_install_supported() -> bool {
    match std::env::consts::OS {
        "macos" => find_executable("brew").is_some(),
        "windows" => find_executable("winget").is_some(),
        "linux" => find_executable("curl").is_some() && find_executable("sh").is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_output_removes_multiline_log_injection_and_limits_size() {
        let output = bounded_output(format!("one\n two\r\n{}", "x".repeat(4_000)).as_bytes());
        assert!(output.starts_with("one two "));
        assert!(output.len() <= MAX_DETAIL_BYTES);
        assert!(!output.contains('\n'));
    }

    #[test]
    fn executable_lookup_does_not_treat_missing_tools_as_available() {
        assert!(find_executable("cortana-tool-that-does-not-exist").is_none());
    }

    #[test]
    fn python_version_gate_rejects_old_or_malformed_versions() {
        assert!(python_version_supported("Python 3.11.9"));
        assert!(python_version_supported("Python 3.13.1"));
        assert!(!python_version_supported("Python 3.10.14"));
        assert!(!python_version_supported("unknown"));
    }

    #[test]
    fn uv_python_path_parser_accepts_only_an_absolute_path_line() {
        assert_eq!(
            parse_uv_python_path(
                b"/Users/example/.local/share/uv/python/cpython-3.11/bin/python3.11\n"
            ),
            Some(PathBuf::from(
                "/Users/example/.local/share/uv/python/cpython-3.11/bin/python3.11"
            ))
        );
        assert!(parse_uv_python_path(b"python3.11\n").is_none());
        assert!(parse_uv_python_path(b"\n\n").is_none());
    }

    #[test]
    fn readiness_json_is_not_silently_truncated() {
        assert!(parse_readiness_output(br#"{"ready":true}"#, b"").is_ok());
        assert!(parse_readiness_output(&vec![b'x'; MAX_READINESS_BYTES + 1], b"").is_err());
    }
}
