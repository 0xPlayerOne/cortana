use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::{
    ShellExt,
    process::{CommandChild, CommandEvent},
};
use tokio::{process::Command, time::timeout};

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
// The bundled CLI's comprehensive readiness gate allows 240 seconds for the
// observed large local corpus. Keep process/IPC margin for the CLI's maximum
// 300-second embedding probe so Desktop does not report a false timeout while
// the CLI is still fail-closed.
const READINESS_TIMEOUT: Duration = Duration::from_secs(330);
const EMBEDDING_MIGRATION_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_DETAIL_BYTES: usize = 2_048;
const MAX_READINESS_BYTES: usize = 64 * 1024;
const MAX_EMBEDDING_FINGERPRINT_BYTES: usize = 512;

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
    let (bundled_version, uv, connector, rust, embedding_runtime) = tokio::join!(
        sidecar_output(app, &["--version"], VERSION_TIMEOUT),
        tool_status("uv", "uv", &["uv"], true, uv_install_supported()),
        connector_status(app),
        tool_status("rust", "Rust toolchain", &["rustc"], false, false),
        embedding_runtime_status(),
    );
    let cortana = bundled_runtime_status(bundled_version.as_ref());
    let python = python_status(uv.available).await;
    let tools = vec![
        cortana.clone(),
        uv,
        python,
        connector,
        embedding_runtime,
        rust,
    ];
    let (core, core_error) = if bundled_version.is_ok() {
        match sidecar_readiness(app).await {
            Ok(report) => (Some(report), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (
            None,
            Some(
                "The bundled Cortana runtime is unavailable; reinstall this Desktop release."
                    .into(),
            ),
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

fn bundled_runtime_status(result: Result<&ProcessOutput, &String>) -> ToolStatus {
    match result {
        Ok(version) => ToolStatus {
            id: "cortana",
            label: "Cortana runtime",
            required: true,
            available: true,
            path: Some("bundled sidecar".into()),
            version: Some(bounded_output(&version.stdout)),
            install_supported: false,
            detail: "Cryptographically bound to this desktop release.".into(),
        },
        Err(_) => ToolStatus {
            id: "cortana",
            label: "Cortana runtime",
            required: true,
            available: false,
            path: None,
            version: None,
            install_supported: false,
            detail: "Bundled Cortana runtime unavailable; reinstall this Desktop release.".into(),
        },
    }
}

async fn sidecar_readiness(app: &AppHandle) -> Result<Value, String> {
    let output = sidecar_output(app, &["readiness"], READINESS_TIMEOUT).await?;
    parse_readiness_output(&output.stdout, &output.stderr)
}

pub async fn migrate_embedding_generation(app: &AppHandle, from: &str) -> Result<String, String> {
    validate_embedding_fingerprint(from)?;
    let args = ["migrate-embedding", "--from", from, "--force"];
    let output = migration_sidecar_output(app, &args).await?;
    if !output.success {
        let detail = bounded_output(if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        });
        return Err(if detail.is_empty() {
            "embedding generation migration failed".into()
        } else {
            detail
        });
    }
    Ok(bounded_output(&output.stdout))
}

struct ProcessOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn migration_sidecar_output(app: &AppHandle, args: &[&str]) -> Result<ProcessOutput, String> {
    let command = app
        .shell()
        .sidecar("cortana")
        .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
        .args(args)
        .env("CORTANA_DESKTOP_PROCESS_GROUP", "1")
        .set_raw_out(true);
    let (mut receiver, child) = command
        .spawn()
        .map_err(|error| format!("run bundled Cortana runtime: {error}"))?;
    let result = timeout(EMBEDDING_MIGRATION_TIMEOUT, async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut success = false;
        while let Some(event) = receiver.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => append_migration_output(&mut stdout, &bytes),
                CommandEvent::Stderr(bytes) => append_migration_output(&mut stderr, &bytes),
                CommandEvent::Error(error) => {
                    return Err(format!("run bundled Cortana runtime: {error}"));
                }
                CommandEvent::Terminated(payload) => {
                    success = payload.code == Some(0);
                    break;
                }
                _ => {}
            }
        }
        Ok(ProcessOutput {
            success,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            terminate_process(child);
            Err(error)
        }
        Err(_) => {
            terminate_process(child);
            Err("embedding generation migration timed out".into())
        }
    }
}

fn append_migration_output(buffer: &mut Vec<u8>, bytes: &[u8]) {
    let remaining = MAX_DETAIL_BYTES.saturating_sub(buffer.len());
    buffer.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

fn terminate_process(child: CommandChild) {
    #[cfg(unix)]
    {
        let pid = child.pid();
        if pid > 0 && pid <= i32::MAX as u32 {
            let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
    let _ = child.kill();
}

fn validate_embedding_fingerprint(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_EMBEDDING_FINGERPRINT_BYTES {
        return Err(format!(
            "embedding fingerprint must contain 1 to {MAX_EMBEDDING_FINGERPRINT_BYTES} bytes"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("embedding fingerprint must not contain control characters".into());
    }
    Ok(())
}

async fn sidecar_output(
    app: &AppHandle,
    args: &[&str],
    duration: Duration,
) -> Result<ProcessOutput, String> {
    let command = app
        .shell()
        .sidecar("cortana")
        .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
        .args(args)
        .env("CORTANA_DESKTOP_PROCESS_GROUP", "1")
        .set_raw_out(true);
    let (mut receiver, child) = command
        .spawn()
        .map_err(|error| format!("run bundled Cortana runtime: {error}"))?;
    let result = timeout(duration, async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut success = false;
        while let Some(event) = receiver.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => stdout.extend(bytes),
                CommandEvent::Stderr(bytes) => stderr.extend(bytes),
                CommandEvent::Error(error) => {
                    return Err(format!("run bundled Cortana runtime: {error}"));
                }
                CommandEvent::Terminated(payload) => {
                    success = payload.code == Some(0);
                    break;
                }
                _ => {}
            }
        }
        Ok(ProcessOutput {
            success,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            terminate_process(child);
            Err(error)
        }
        Err(_) => {
            terminate_process(child);
            Err("bundled Cortana command timed out".into())
        }
    }
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

async fn connector_status(app: &AppHandle) -> ToolStatus {
    let path = connector_candidates()
        .into_iter()
        .find(|candidate| is_executable(candidate));
    let version = match path.as_deref() {
        Some(path) => command_version(path).await,
        None => None,
    };
    let resource_available = bundled_connector_resource_dir(app).is_ok();
    let uv_available = find_executable("uv").is_some();
    let install_supported = resource_available && uv_available;
    ToolStatus {
        id: "connectors",
        label: "Connector environment",
        required: true,
        available: path.is_some(),
        path: path.as_ref().map(|path| path.display().to_string()),
        version,
        install_supported,
        detail: path.as_ref().map_or_else(
            || {
                if !resource_available {
                    "This Desktop build is missing its bundled connector workspace.".into()
                } else if uv_available {
                    "Approve installation of the bundled ingestion workspace.".into()
                } else {
                    "Install uv before installing the bundled ingestion workspace.".into()
                }
            },
            |path| format!("Found {}", path.display()),
        ),
    }
}

async fn embedding_runtime_status() -> ToolStatus {
    let settings = crate::settings::load().ok();
    let required = settings
        .as_ref()
        .is_none_or(|snapshot| snapshot.embedding.provider == "local");
    let configured_program = settings
        .as_ref()
        .and_then(|snapshot| snapshot.embedding_service_program.as_deref());
    let path = embedding_runtime_path(configured_program);
    let install_supported =
        required && cfg!(target_os = "macos") && find_executable("brew").is_some();
    let detail = match (required, path.as_ref(), configured_program) {
        (false, Some(path), _) => format!("Found optional local runtime at {}", path.display()),
        (false, None, _) => "Not required for the configured cloud embedding provider.".into(),
        (true, Some(path), _) => format!("Found {}", path.display()),
        (true, None, Some(program)) if install_supported => format!(
            "Configured embedding service `{program}` was not found; approve installation of the Homebrew runtime."
        ),
        (true, None, Some(program)) => format!(
            "Configured embedding service `{program}` was not found. Install text-embeddings-inference with your platform package manager."
        ),
        (true, None, None) if install_supported => {
            "Install the text-embeddings-inference runtime with Homebrew.".into()
        }
        (true, None, None) => {
            "The local text-embeddings-router runtime is required for local embeddings.".into()
        }
    };
    let version = match path.as_deref() {
        Some(path) => command_version(path).await,
        None => None,
    };
    ToolStatus {
        id: "embedding-runtime",
        label: "Local embedding runtime",
        required,
        available: path.is_some(),
        path: path.map(|path| path.display().to_string()),
        version,
        install_supported,
        detail,
    }
}

fn embedding_runtime_path(configured_program: Option<&str>) -> Option<PathBuf> {
    configured_program
        .and_then(|program| {
            let candidate = Path::new(program);
            if candidate.is_absolute() {
                Some(candidate.to_path_buf())
            } else {
                find_executable(program)
            }
        })
        .filter(|candidate| is_executable(candidate))
        .or_else(|| {
            configured_program
                .is_none()
                .then(|| find_executable("text-embeddings-router"))
                .flatten()
        })
}

pub(crate) fn bundled_connector_resource_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.extend(bundled_connector_candidates(&resource_dir));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("cortana-connectors"),
    );
    candidates
        .into_iter()
        .find(|candidate| {
            candidate.join("pyproject.toml").is_file()
                && candidate.join("src").join("cortana").is_dir()
        })
        .ok_or_else(|| "bundled connector workspace is unavailable".into())
}

fn bundled_connector_candidates(resource_dir: &Path) -> Vec<PathBuf> {
    ["resources/cortana-connectors", "cortana-connectors"]
        .into_iter()
        .map(|relative| resource_dir.join(relative))
        .collect()
}

fn connector_candidates() -> Vec<PathBuf> {
    connector_candidates_from(
        std::env::var_os("CORTANA_INSTALL_PREFIX").map(PathBuf::from),
        dirs::home_dir(),
    )
}

fn connector_candidates_from(prefix: Option<PathBuf>, home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(prefix) = prefix {
        if prefix.is_absolute() {
            candidates.push(prefix.join("share/cortana").join(connector_relative_path()));
        }
    }
    if let Some(home) = home {
        candidates.push(
            home.join(".local/share/cortana")
                .join(connector_relative_path()),
        );
    }
    candidates
}

#[cfg(windows)]
fn connector_relative_path() -> &'static str {
    "venv/Scripts/cortana-connectors.exe"
}

#[cfg(not(windows))]
fn connector_relative_path() -> &'static str {
    "venv/bin/cortana-connectors"
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
    if output.stdout.len() > MAX_DETAIL_BYTES {
        return None;
    }
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

pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
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
    // GUI-launched macOS processes often do not inherit the shell's Homebrew
    // path. Include the conventional prefixes so readiness and installers
    // work consistently from Finder, the Dock, and a terminal.
    #[cfg(unix)]
    for path in [
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ] {
        if !paths.contains(&path) {
            paths.push(path);
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
    fn missing_bundled_runtime_fails_closed_without_path_fallback() {
        let error = "sidecar unavailable".to_string();
        let status = bundled_runtime_status(Err(&error));
        assert!(status.required);
        assert!(!status.available);
        assert!(status.path.is_none());
        assert!(!status.install_supported);
        assert!(status.detail.contains("reinstall"));
    }

    #[test]
    fn configured_embedding_runtime_does_not_fall_back_to_a_different_binary() {
        assert!(embedding_runtime_path(Some("cortana-tool-that-does-not-exist")).is_none());
    }

    #[test]
    fn connector_candidates_match_release_install_layout() {
        let prefix = if cfg!(windows) {
            PathBuf::from(r"C:\opt\cortana")
        } else {
            PathBuf::from("/opt/cortana")
        };
        let home = if cfg!(windows) {
            PathBuf::from(r"C:\Users\example")
        } else {
            PathBuf::from("/Users/example")
        };
        let candidates = connector_candidates_from(Some(prefix.clone()), Some(home.clone()));
        assert_eq!(
            candidates,
            vec![
                prefix.join("share/cortana").join(connector_relative_path()),
                home.join(".local/share/cortana")
                    .join(connector_relative_path()),
            ]
        );
        assert_eq!(
            connector_candidates_from(Some(PathBuf::from("relative")), Some(home.clone())),
            vec![
                home.join(".local/share/cortana")
                    .join(connector_relative_path())
            ]
        );
    }

    #[test]
    fn bundled_connector_candidates_prefer_tauri_resource_prefix() {
        assert_eq!(
            bundled_connector_candidates(Path::new("/Applications/Cortana.app/Contents/Resources")),
            vec![
                PathBuf::from(
                    "/Applications/Cortana.app/Contents/Resources/resources/cortana-connectors"
                ),
                PathBuf::from("/Applications/Cortana.app/Contents/Resources/cortana-connectors"),
            ]
        );
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

    #[test]
    fn embedding_fingerprint_validation_is_exact_and_bounded() {
        assert!(
            validate_embedding_fingerprint("openai:http://127.0.0.1:6999/v1:model:256").is_ok()
        );
        assert!(validate_embedding_fingerprint("").is_err());
        assert!(validate_embedding_fingerprint("legacy\nmodel").is_err());
        assert!(
            validate_embedding_fingerprint(&"x".repeat(MAX_EMBEDDING_FINGERPRINT_BYTES + 1))
                .is_err()
        );
    }

    #[test]
    fn migration_output_is_bounded_before_rendering_native_errors() {
        let mut output = Vec::new();
        append_migration_output(&mut output, &vec![b'x'; MAX_DETAIL_BYTES + 1]);
        append_migration_output(&mut output, b"more");
        assert_eq!(output.len(), MAX_DETAIL_BYTES);
    }
}
