use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::config::Config;

const LABELS: [&str; 4] = [
    "ai.cortana.embedding",
    "ai.cortana.server",
    "ai.cortana.sync",
    "ai.cortana.backup",
];

pub struct InstallOptions<'a> {
    pub config: &'a Path,
    pub web_dir: &'a Path,
    pub working_directory: &'a Path,
    pub sync_seconds: u64,
    pub backup_seconds: u64,
    pub install_embedding: bool,
    pub install_sync: bool,
}

pub fn install(config: &Config, options: InstallOptions<'_>) -> Result<()> {
    require_macos()?;
    anyhow::ensure!(
        options.config.is_file(),
        "configuration file does not exist"
    );
    anyhow::ensure!(
        options.web_dir.join("index.html").is_file(),
        "workspace build is missing: {}",
        options.web_dir.display()
    );
    let executable = std::env::current_exe()?.canonicalize()?;
    let launch_agents = launch_agents_directory()?;
    let logs = config.data_dir.join("logs");
    std::fs::create_dir_all(&launch_agents)?;
    std::fs::create_dir_all(&logs)?;

    let common = vec![
        executable.display().to_string(),
        "--config".into(),
        options.config.display().to_string(),
    ];
    let jobs = configured_jobs(
        &common,
        options.web_dir,
        options.sync_seconds,
        options.backup_seconds,
        options.install_embedding,
        options.install_sync,
    );

    for label in LABELS {
        bootout(label)?;
        let path = launch_agents.join(format!("{label}.plist"));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    for job in jobs {
        let path = launch_agents.join(format!("{}.plist", job.label));
        let body = plist(&job, options.working_directory, &logs);
        atomic_write(&path, body.as_bytes())?;
        enable(job.label)?;
        bootstrap(&path)?;
        println!("installed {}", job.label);
    }
    Ok(())
}

fn configured_jobs(
    common: &[String],
    web_dir: &Path,
    sync_seconds: u64,
    backup_seconds: u64,
    install_embedding: bool,
    install_sync: bool,
) -> Vec<Job> {
    let mut jobs = Vec::new();
    if install_embedding {
        jobs.push(Job {
            label: "ai.cortana.embedding",
            arguments: [common.to_vec(), vec!["embedding-service".into()]].concat(),
            schedule: Schedule::KeepAlive,
        });
    }
    jobs.push(Job {
        label: "ai.cortana.server",
        arguments: [
            common.to_vec(),
            vec![
                "serve".into(),
                "--web-dir".into(),
                web_dir.display().to_string(),
            ],
        ]
        .concat(),
        schedule: Schedule::KeepAlive,
    });
    if install_sync {
        jobs.push(Job {
            label: "ai.cortana.sync",
            arguments: [common.to_vec(), vec!["sync".into()]].concat(),
            schedule: Schedule::Interval(sync_seconds),
        });
    }
    jobs.push(Job {
        label: "ai.cortana.backup",
        arguments: [
            common.to_vec(),
            vec!["backup".into(), "--keep".into(), "14".into()],
        ]
        .concat(),
        schedule: Schedule::Interval(backup_seconds),
    });
    jobs
}

pub fn uninstall() -> Result<()> {
    require_macos()?;
    let launch_agents = launch_agents_directory()?;
    for label in LABELS {
        bootout(label)?;
        let path = launch_agents.join(format!("{label}.plist"));
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("removed {label}");
        }
    }
    Ok(())
}

pub fn status() -> Result<()> {
    require_macos()?;
    let domain = launch_domain()?;
    for label in LABELS {
        let output = Command::new("launchctl")
            .args(["print", &format!("{domain}/{label}")])
            .output()?;
        let state = if output.status.success() {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .find_map(|line| line.trim().strip_prefix("state = "))
                .unwrap_or("loaded")
                .to_string()
        } else {
            "not loaded".into()
        };
        println!("{label}: {state}");
    }
    Ok(())
}

pub fn sync_job_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        launch_agents_directory()
            .map(|directory| directory.join("ai.cortana.sync.plist").is_file())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

struct Job {
    label: &'static str,
    arguments: Vec<String>,
    schedule: Schedule,
}

enum Schedule {
    KeepAlive,
    Interval(u64),
}

fn plist(job: &Job, working_directory: &Path, logs: &Path) -> String {
    let arguments = job
        .arguments
        .iter()
        .map(|argument| format!("      <string>{}</string>", xml_escape(argument)))
        .collect::<Vec<_>>()
        .join("\n");
    let schedule = match job.schedule {
        Schedule::KeepAlive => {
            "    <key>RunAtLoad</key>\n    <true/>\n    <key>KeepAlive</key>\n    <true/>".into()
        }
        Schedule::Interval(seconds) => format!(
            "    <key>StartInterval</key>\n    <integer>{}</integer>\n    <key>LowPriorityIO</key>\n    <true/>",
            seconds.max(60)
        ),
    };
    let stdout = logs.join(format!("{}.log", job.label));
    let stderr = logs.join(format!("{}.error.log", job.label));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{arguments}
    </array>
    <key>WorkingDirectory</key>
    <string>{working_directory}</string>
{schedule}
    <key>ThrottleInterval</key>
    <integer>30</integer>
    <key>EnvironmentVariables</key>
    <dict>
      <key>PATH</key>
      <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    </dict>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
  </dict>
</plist>
"#,
        label = xml_escape(job.label),
        working_directory = xml_escape(&working_directory.display().to_string()),
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string()),
    )
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn bootstrap(path: &Path) -> Result<()> {
    let status = Command::new("launchctl")
        .args(["bootstrap", &launch_domain()?, &path.display().to_string()])
        .status()?;
    anyhow::ensure!(
        status.success(),
        "launchctl bootstrap failed for {}",
        path.display()
    );
    Ok(())
}

fn enable(label: &str) -> Result<()> {
    let status = Command::new("launchctl")
        .args(["enable", &format!("{}/{label}", launch_domain()?)])
        .status()?;
    anyhow::ensure!(status.success(), "launchctl enable failed for {label}");
    Ok(())
}

fn bootout(label: &str) -> Result<()> {
    let domain = launch_domain()?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{domain}/{label}")])
        .output();
    let deadline = Instant::now() + Duration::from_secs(10);
    while launchctl_job_loaded(&domain, label)? {
        anyhow::ensure!(
            Instant::now() < deadline,
            "launchctl job did not unload: {label}"
        );
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn launchctl_job_loaded(domain: &str, label: &str) -> Result<bool> {
    Ok(Command::new("launchctl")
        .args(["print", &format!("{domain}/{label}")])
        .output()?
        .status
        .success())
}

fn launch_domain() -> Result<String> {
    let output = Command::new("id").arg("-u").output()?;
    anyhow::ensure!(output.status.success(), "could not determine user ID");
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn launch_agents_directory() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("Library/LaunchAgents"))
        .context("home directory is unavailable")
}

fn require_macos() -> Result<()> {
    anyhow::ensure!(
        cfg!(target_os = "macos"),
        "service management currently supports macOS launchd; use packaging/cortana.service on Linux"
    );
    Ok(())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_plist_escapes_paths_and_has_bounded_interval() {
        let body = plist(
            &Job {
                label: "ai.cortana.sync",
                arguments: vec!["cortana".into(), "a&b".into()],
                schedule: Schedule::Interval(1),
            },
            Path::new("/tmp/a&b"),
            Path::new("/tmp/logs"),
        );
        assert!(body.contains("<string>a&amp;b</string>"));
        assert!(body.contains("<integer>60</integer>"));
        assert!(body.contains("<key>LowPriorityIO</key>"));

        #[cfg(target_os = "macos")]
        {
            use std::io::Write;
            use std::process::Stdio;

            let mut child = Command::new("plutil")
                .args(["-lint", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .expect("start plutil");
            child
                .stdin
                .take()
                .expect("plutil stdin")
                .write_all(body.as_bytes())
                .expect("write plist");
            assert!(child.wait().expect("plutil").success());
        }
    }

    #[test]
    fn recurring_sync_job_requires_explicit_opt_in() {
        let common = vec!["cortana".into(), "--config".into(), "config.toml".into()];
        let safe = configured_jobs(&common, Path::new("/tmp/web"), 900, 86_400, true, false);
        assert!(
            safe.iter().all(|job| job.label != "ai.cortana.sync"),
            "safe installation must be query-only by default"
        );
        let scheduled = configured_jobs(&common, Path::new("/tmp/web"), 900, 86_400, true, true);
        assert!(
            scheduled.iter().any(|job| job.label == "ai.cortana.sync"),
            "explicit opt-in must install the recurring sync job"
        );
    }
}
