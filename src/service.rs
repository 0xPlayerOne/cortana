use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;

const LABELS: [&str; 4] = [
    "ai.cortana.embedding",
    "ai.cortana.server",
    "ai.cortana.sync",
    "ai.cortana.backup",
];

#[derive(Debug, Serialize)]
pub struct ServiceReport {
    pub platform: &'static str,
    pub supported: bool,
    pub services: Vec<ServiceStatus>,
}

#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub name: &'static str,
    pub label: &'static str,
    pub installed: bool,
    pub loaded: bool,
    pub state: Option<String>,
    pub pid: Option<u32>,
    pub last_exit_status: Option<i32>,
}

pub struct InstallOptions<'a> {
    pub config: &'a Path,
    pub web_dir: Option<&'a Path>,
    pub no_web: bool,
    pub working_directory: &'a Path,
    pub sync_seconds: u64,
    pub backup_seconds: u64,
    pub install_embedding: bool,
    pub install_sync: bool,
}

pub fn install(config: &Config, options: InstallOptions<'_>) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return install_windows(config, options);
    }
    if cfg!(target_os = "macos") {
        return install_launchd(config, options);
    }
    if cfg!(target_os = "linux") {
        return install_systemd(config, options);
    }
    Err(anyhow::anyhow!(unsupported_service_manager()))
}

fn install_launchd(config: &Config, options: InstallOptions<'_>) -> Result<()> {
    require_macos()?;
    anyhow::ensure!(
        options.config.is_file(),
        "configuration file does not exist"
    );
    if !options.no_web {
        let web_dir = options
            .web_dir
            .context("workspace directory is required unless --no-web is used")?;
        anyhow::ensure!(
            web_dir.join("index.html").is_file(),
            "workspace build is missing: {}",
            web_dir.display()
        );
    }
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
        options.no_web,
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
    web_dir: Option<&Path>,
    sync_seconds: u64,
    backup_seconds: u64,
    install_embedding: bool,
    install_sync: bool,
    no_web: bool,
) -> Vec<Job> {
    let mut jobs = Vec::new();
    if install_embedding {
        jobs.push(Job {
            label: "ai.cortana.embedding",
            arguments: [common.to_vec(), vec!["embedding-service".into()]].concat(),
            schedule: Schedule::KeepAlive,
        });
    }
    let server_arguments = if no_web {
        [common.to_vec(), vec!["serve".into(), "--no-web".into()]].concat()
    } else {
        let web_dir = web_dir.expect("web directory required for workspace service");
        [
            common.to_vec(),
            vec![
                "serve".into(),
                "--web-dir".into(),
                web_dir.display().to_string(),
            ],
        ]
        .concat()
    };
    jobs.push(Job {
        label: "ai.cortana.server",
        arguments: server_arguments,
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

#[cfg(target_os = "windows")]
const WINDOWS_TASK_MAX_COMMAND_BYTES: usize = 8_191;

#[cfg(target_os = "windows")]
fn install_windows(config: &Config, options: InstallOptions<'_>) -> Result<()> {
    ensure_windows_task_scheduler()?;
    anyhow::ensure!(
        options.config.is_file(),
        "configuration file does not exist"
    );
    if !options.no_web {
        let web_dir = options
            .web_dir
            .context("workspace directory is required unless --no-web is used")?;
        anyhow::ensure!(
            web_dir.join("index.html").is_file(),
            "workspace build is missing: {}",
            web_dir.display()
        );
    }
    let executable = std::env::current_exe()?.canonicalize()?;
    std::fs::create_dir_all(config.data_dir.join("logs"))?;
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
        options.no_web,
    );

    for label in LABELS {
        windows_delete_task(label)?;
    }
    for job in jobs {
        windows_create_task(&job)?;
        println!("installed {}", job.label);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_create_task(job: &Job) -> Result<()> {
    let task = windows_task_name(job.label)?;
    let command = windows_command_line(&job.arguments);
    anyhow::ensure!(
        command.len() <= WINDOWS_TASK_MAX_COMMAND_BYTES,
        "Cortana service command is too long for Windows Task Scheduler"
    );
    let mut args = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        task.to_string(),
        "/TR".to_string(),
        command,
        "/F".to_string(),
        "/RL".to_string(),
        "LIMITED".to_string(),
    ];
    match job.schedule {
        Schedule::KeepAlive => {
            args.extend([
                "/SC".to_string(),
                "ONLOGON".to_string(),
                "/DELAY".to_string(),
                "0000:30".to_string(),
            ]);
        }
        Schedule::Interval(seconds) => {
            args.extend([
                "/SC".to_string(),
                "MINUTE".to_string(),
                "/MO".to_string(),
                seconds.div_ceil(60).max(1).to_string(),
            ]);
        }
    }
    let output = windows_schtasks(&args)
        .with_context(|| format!("install Windows Task Scheduler task {task}"))?;
    anyhow::ensure!(
        output.status.success(),
        "install Windows Task Scheduler task {task} failed: {}",
        bounded_command_error(&output.stderr)
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn uninstall_windows() -> Result<()> {
    ensure_windows_task_scheduler()?;
    for label in LABELS {
        windows_delete_task(label)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_delete_task(label: &str) -> Result<()> {
    let task = windows_task_name(label)?;
    if !windows_task_exists(label) {
        return Ok(());
    }
    let args = vec![
        "/Delete".to_string(),
        "/TN".to_string(),
        task.to_string(),
        "/F".to_string(),
    ];
    let output = windows_schtasks(&args)?;
    anyhow::ensure!(
        output.status.success(),
        "delete Windows Task Scheduler task {task} failed: {}",
        bounded_command_error(&output.stderr)
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn status_windows() -> Result<ServiceReport> {
    ensure_windows_task_scheduler()?;
    let services = managed_services()
        .map(|(name, label)| windows_service_status(name, label))
        .collect();
    Ok(ServiceReport {
        platform: std::env::consts::OS,
        supported: true,
        services,
    })
}

#[cfg(target_os = "windows")]
fn windows_service_status(name: &'static str, label: &'static str) -> ServiceStatus {
    let (installed, body) = windows_task_query(label)
        .map(|body| (true, body))
        .unwrap_or((false, String::new()));
    let (state, last_exit_status) = parse_windows_task_status(&body);
    ServiceStatus {
        name,
        label,
        installed,
        loaded: installed,
        state: state.or_else(|| installed.then(|| "ready".into())),
        pid: None,
        last_exit_status,
    }
}

#[cfg(target_os = "windows")]
fn windows_action(name: &str, action: &str) -> Result<()> {
    let label = service_label(name)?;
    let task = windows_task_name(label)?;
    anyhow::ensure!(
        matches!(action, "start" | "stop" | "restart"),
        "unsupported Cortana service action: {action}"
    );
    anyhow::ensure!(
        windows_task_exists(label),
        "Cortana service is not installed: {name}"
    );
    if matches!(action, "stop" | "restart") {
        let args = vec!["/End".to_string(), "/TN".to_string(), task.to_string()];
        // Ending an already-idle task is harmless for the user-facing stop and
        // restart actions; the subsequent run remains authoritative.
        let _ = windows_schtasks(&args);
    }
    if matches!(action, "start" | "restart") {
        let args = vec!["/Run".to_string(), "/TN".to_string(), task.to_string()];
        let output = windows_schtasks(&args)?;
        anyhow::ensure!(
            output.status.success(),
            "start Windows Task Scheduler task {task} failed: {}",
            bounded_command_error(&output.stderr)
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn ensure_windows_task_scheduler() -> Result<()> {
    let output = Command::new("schtasks.exe").arg("/Query").output();
    anyhow::ensure!(
        output.is_ok(),
        "Windows Task Scheduler is unavailable; install Cortana services from a logged-in user session"
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_schtasks(args: &[String]) -> Result<std::process::Output> {
    Command::new("schtasks.exe")
        .args(args)
        .output()
        .context("run schtasks.exe")
}

#[cfg(target_os = "windows")]
fn windows_task_query(label: &str) -> Option<String> {
    let task = windows_task_name(label).ok()?;
    let args = [
        "/Query".to_string(),
        "/TN".to_string(),
        task.to_string(),
        "/FO".to_string(),
        "LIST".to_string(),
        "/V".to_string(),
    ];
    let output = windows_schtasks(&args).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "windows")]
fn windows_task_exists(label: &str) -> bool {
    windows_task_query(label).is_some()
}

#[cfg(target_os = "windows")]
fn windows_task_name(label: &str) -> Result<&'static str> {
    match label {
        "ai.cortana.embedding" => Ok(r"\Cortana-embedding"),
        "ai.cortana.server" => Ok(r"\Cortana-server"),
        "ai.cortana.sync" => Ok(r"\Cortana-sync"),
        "ai.cortana.backup" => Ok(r"\Cortana-backup"),
        _ => Err(anyhow::anyhow!(
            "unsupported Cortana service label: {label}"
        )),
    }
}

#[cfg(target_os = "windows")]
fn windows_command_line(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| windows_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "windows")]
fn windows_quote(value: &str) -> String {
    let mut output = String::from("\"");
    let mut backslashes = 0usize;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            output.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
            output.push('"');
        } else {
            output.extend(std::iter::repeat('\\').take(backslashes));
            output.push(character);
        }
        backslashes = 0;
    }
    output.extend(std::iter::repeat('\\').take(backslashes * 2));
    output.push('"');
    output
}

#[cfg(target_os = "windows")]
fn parse_windows_task_status(body: &str) -> (Option<String>, Option<i32>) {
    let mut state = None;
    let mut last_exit_status = None;
    for line in body.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "status" => state = Some(value.trim().to_ascii_lowercase()),
            "last run result" => last_exit_status = parse_windows_exit(value.trim()),
            _ => {}
        }
    }
    (state, last_exit_status)
}

#[cfg(target_os = "windows")]
fn parse_windows_exit(value: &str) -> Option<i32> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
            .ok()
            .and_then(|value| i32::try_from(value).ok())
    } else {
        value.parse().ok()
    }
}

pub fn uninstall() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return uninstall_windows();
    }
    if cfg!(target_os = "linux") {
        return uninstall_systemd();
    }
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

pub fn status() -> Result<ServiceReport> {
    #[cfg(target_os = "windows")]
    {
        return status_windows();
    }
    if cfg!(target_os = "linux") {
        return status_systemd();
    }
    if !cfg!(target_os = "macos") {
        return Ok(ServiceReport {
            platform: std::env::consts::OS,
            supported: false,
            services: managed_services()
                .map(|(name, label)| ServiceStatus {
                    name,
                    label,
                    installed: false,
                    loaded: false,
                    state: None,
                    pid: None,
                    last_exit_status: None,
                })
                .collect(),
        });
    }
    let domain = launch_domain()?;
    let launch_agents = launch_agents_directory()?;
    let mut services = Vec::new();
    for (name, label) in managed_services() {
        let output = Command::new("launchctl")
            .args(["print", &format!("{domain}/{label}")])
            .output()?;
        let loaded = output.status.success();
        let body = String::from_utf8_lossy(&output.stdout);
        let (state, pid, last_exit_status) = if loaded {
            parse_launchctl_status(&body)
        } else {
            (None, None, None)
        };
        services.push(ServiceStatus {
            name,
            label,
            installed: launch_agents.join(format!("{label}.plist")).is_file(),
            loaded,
            state: state.or_else(|| loaded.then(|| "loaded".into())),
            pid,
            last_exit_status,
        });
    }
    Ok(ServiceReport {
        platform: std::env::consts::OS,
        supported: true,
        services,
    })
}

pub fn start(name: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return windows_action(name, "start");
    }
    if cfg!(target_os = "linux") {
        return systemd_action(name, "start");
    }
    require_macos()?;
    let label = service_label(name)?;
    let path = installed_plist(label)?;
    if !launchctl_job_loaded(&launch_domain()?, label)? {
        enable(label)?;
        bootstrap(&path)?;
    }
    kickstart(label, false)
}

pub fn stop(name: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return windows_action(name, "stop");
    }
    if cfg!(target_os = "linux") {
        return systemd_action(name, "stop");
    }
    require_macos()?;
    let label = service_label(name)?;
    installed_plist(label)?;
    bootout(label)
}

pub fn restart(name: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        return windows_action(name, "restart");
    }
    if cfg!(target_os = "linux") {
        return systemd_action(name, "restart");
    }
    require_macos()?;
    let label = service_label(name)?;
    let path = installed_plist(label)?;
    bootout(label)?;
    enable(label)?;
    bootstrap(&path)?;
    kickstart(label, true)
}

pub fn sync_job_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        launch_agents_directory()
            .map(|directory| directory.join("ai.cortana.sync.plist").is_file())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        systemd_unit_path("sync")
            .and_then(|path| {
                let timer = path
                    .parent()
                    .map(|directory| directory.join(systemd_timer_unit("sync")))?;
                Ok(path.is_file() && timer.is_file())
            })
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        return windows_task_exists("ai.cortana.sync");
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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

fn kickstart(label: &str, restart: bool) -> Result<()> {
    let mut arguments = vec!["kickstart"];
    if restart {
        arguments.push("-k");
    }
    let target = format!("{}/{label}", launch_domain()?);
    arguments.push(&target);
    let status = Command::new("launchctl").args(arguments).status()?;
    anyhow::ensure!(status.success(), "launchctl kickstart failed for {label}");
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

fn installed_plist(label: &str) -> Result<PathBuf> {
    let path = launch_agents_directory()?.join(format!("{label}.plist"));
    anyhow::ensure!(path.is_file(), "Cortana service is not installed: {label}");
    Ok(path)
}

fn managed_services() -> impl Iterator<Item = (&'static str, &'static str)> {
    [
        ("embedding", LABELS[0]),
        ("server", LABELS[1]),
        ("sync", LABELS[2]),
        ("backup", LABELS[3]),
    ]
    .into_iter()
}

fn service_label(name: &str) -> Result<&'static str> {
    managed_services()
        .find_map(|(candidate, label)| (candidate == name).then_some(label))
        .with_context(|| format!("unsupported Cortana service: {name}"))
}

fn parse_launchctl_status(body: &str) -> (Option<String>, Option<u32>, Option<i32>) {
    let value = |prefix: &str| {
        body.lines()
            .find_map(|line| line.trim().strip_prefix(prefix))
            .map(str::trim)
    };
    (
        value("state = ").map(str::to_string),
        value("pid = ").and_then(|value| value.parse().ok()),
        value("last exit code = ").and_then(|value| value.parse().ok()),
    )
}

const SYSTEMD_SERVICE_UNITS: [&str; 4] = [
    "cortana-embedding.service",
    "cortana.service",
    "cortana-sync.service",
    "cortana-backup.service",
];
const SYSTEMD_TIMER_UNITS: [&str; 2] = ["cortana-sync.timer", "cortana-backup.timer"];

fn install_systemd(config: &Config, options: InstallOptions<'_>) -> Result<()> {
    ensure_systemd_available()?;
    anyhow::ensure!(
        options.config.is_file(),
        "configuration file does not exist"
    );
    if !options.no_web {
        let web_dir = options
            .web_dir
            .context("workspace directory is required unless --no-web is used")?;
        anyhow::ensure!(
            web_dir.join("index.html").is_file(),
            "workspace build is missing: {}",
            web_dir.display()
        );
    }
    let executable = std::env::current_exe()?.canonicalize()?;
    let unit_directory = systemd_unit_directory()?;
    let logs = config.data_dir.join("logs");
    std::fs::create_dir_all(&unit_directory)?;
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
        options.no_web,
    );

    for unit in SYSTEMD_SERVICE_UNITS
        .iter()
        .chain(SYSTEMD_TIMER_UNITS.iter())
    {
        let _ = systemd_run(&["disable", "--now", unit]);
        let path = unit_directory.join(unit);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    for job in &jobs {
        let name = systemd_job_name(job.label)
            .with_context(|| format!("unsupported Cortana service label: {}", job.label))?;
        let path = unit_directory.join(systemd_service_unit(name));
        let body = systemd_service_unit_body(config, job, name, options.working_directory);
        atomic_write(&path, body.as_bytes())?;
        if matches!(name, "sync" | "backup") {
            let seconds = match name {
                "sync" => options.sync_seconds,
                "backup" => options.backup_seconds,
                _ => unreachable!(),
            };
            let timer_path = unit_directory.join(systemd_timer_unit(name));
            let timer_body = systemd_timer_unit_body(name, seconds);
            atomic_write(&timer_path, timer_body.as_bytes())?;
        }
    }
    systemd_run(&["daemon-reload"])?;
    for job in &jobs {
        let name = systemd_job_name(job.label).expect("validated service label");
        let unit = if matches!(name, "sync" | "backup") {
            systemd_timer_unit(name)
        } else {
            systemd_service_unit(name)
        };
        systemd_run(&["enable", "--now", unit])?;
    }
    Ok(())
}

fn uninstall_systemd() -> Result<()> {
    let unit_directory = systemd_unit_directory()?;
    for unit in SYSTEMD_SERVICE_UNITS
        .iter()
        .chain(SYSTEMD_TIMER_UNITS.iter())
    {
        let _ = systemd_run(&["disable", "--now", unit]);
        let path = unit_directory.join(unit);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    if systemd_user_available() {
        systemd_run(&["daemon-reload"])?;
    }
    Ok(())
}

fn status_systemd() -> Result<ServiceReport> {
    if !systemd_user_available() {
        return Ok(ServiceReport {
            platform: std::env::consts::OS,
            supported: false,
            services: unsupported_services(),
        });
    }
    let unit_directory = systemd_unit_directory()?;
    let services = managed_services()
        .map(|(name, label)| systemd_service_status(name, label, &unit_directory))
        .collect::<Vec<_>>();
    Ok(ServiceReport {
        platform: std::env::consts::OS,
        supported: true,
        services,
    })
}

fn systemd_service_status(
    name: &'static str,
    label: &'static str,
    unit_directory: &Path,
) -> ServiceStatus {
    let service_unit = systemd_service_unit(name);
    let control_unit = if matches!(name, "sync" | "backup") {
        systemd_timer_unit(name)
    } else {
        service_unit
    };
    let installed = unit_directory.join(service_unit).is_file()
        && (!matches!(name, "sync" | "backup")
            || unit_directory.join(systemd_timer_unit(name)).is_file());
    let control = systemd_show(control_unit);
    let service = systemd_show(service_unit);
    let (state, pid, _) = control
        .as_ref()
        .map(|(_, body)| parse_systemd_status(body))
        .unwrap_or((None, None, None));
    let last_exit_status = service
        .as_ref()
        .and_then(|(_, body)| parse_systemd_status(body).2);
    ServiceStatus {
        name,
        label,
        installed,
        loaded: control.as_ref().is_some_and(|(success, _)| *success),
        state: state.or_else(|| installed.then(|| "not loaded".into())),
        pid,
        last_exit_status,
    }
}

fn systemd_action(name: &str, action: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(name, "embedding" | "server" | "sync" | "backup"),
        "unsupported Cortana service: {name}"
    );
    anyhow::ensure!(
        matches!(action, "start" | "stop" | "restart"),
        "unsupported Cortana service action: {action}"
    );
    ensure_systemd_available()?;
    let unit = match name {
        "sync" | "backup" => systemd_timer_unit(name),
        _ => systemd_service_unit(name),
    };
    anyhow::ensure!(
        systemd_unit_path(name)?.is_file(),
        "Cortana service is not installed: {name}"
    );
    systemd_run(&[action, unit])
}

fn ensure_systemd_available() -> Result<()> {
    anyhow::ensure!(
        systemd_user_available(),
        "Linux service management requires a running systemd user manager"
    );
    Ok(())
}

fn systemd_user_available() -> bool {
    Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn systemd_run(args: &[&str]) -> Result<()> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .with_context(|| format!("run systemctl --user {}", args.join(" ")))?;
    anyhow::ensure!(
        output.status.success(),
        "systemctl --user {} failed: {}",
        args.join(" "),
        bounded_command_error(&output.stderr)
    );
    Ok(())
}

fn systemd_show(unit: &str) -> Option<(bool, String)> {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "--no-page",
            "--property=ActiveState,SubState,MainPID,ExecMainStatus",
            unit,
        ])
        .output()
        .ok()?;
    Some((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

fn parse_systemd_status(body: &str) -> (Option<String>, Option<u32>, Option<i32>) {
    let mut active = None;
    let mut sub = None;
    let mut pid = None;
    let mut exit = None;
    for line in body.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "ActiveState" => active = Some(value.to_string()),
            "SubState" => sub = Some(value.to_string()),
            "MainPID" => pid = value.parse::<u32>().ok().filter(|value| *value > 0),
            "ExecMainStatus" => exit = value.parse::<i32>().ok(),
            _ => {}
        }
    }
    let state = active.map(|active| {
        if active == "active" {
            sub.unwrap_or(active)
        } else {
            active
        }
    });
    (state, pid, exit)
}

fn systemd_unit_directory() -> Result<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .context("home directory is unavailable")?;
    Ok(root.join("systemd/user"))
}

fn systemd_unit_path(name: &str) -> Result<PathBuf> {
    Ok(systemd_unit_directory()?.join(systemd_service_unit(name)))
}

fn systemd_service_unit(name: &str) -> &'static str {
    match name {
        "embedding" => "cortana-embedding.service",
        "server" => "cortana.service",
        "sync" => "cortana-sync.service",
        "backup" => "cortana-backup.service",
        _ => "cortana-invalid.service",
    }
}

fn systemd_timer_unit(name: &str) -> &'static str {
    match name {
        "sync" => "cortana-sync.timer",
        "backup" => "cortana-backup.timer",
        _ => "cortana-invalid.timer",
    }
}

fn systemd_job_name(label: &str) -> Option<&'static str> {
    match label {
        "ai.cortana.embedding" => Some("embedding"),
        "ai.cortana.server" => Some("server"),
        "ai.cortana.sync" => Some("sync"),
        "ai.cortana.backup" => Some("backup"),
        _ => None,
    }
}

fn systemd_service_unit_body(
    config: &Config,
    job: &Job,
    name: &str,
    working_directory: &Path,
) -> String {
    let arguments = job
        .arguments
        .iter()
        .map(|argument| systemd_quote(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let description = match name {
        "embedding" => "Cortana local embedding supervisor",
        "server" => "Cortana second-brain API",
        "sync" => "Cortana knowledge source synchronization",
        "backup" => "Cortana verified backup",
        _ => "Cortana service",
    };
    let service_type = if matches!(name, "sync" | "backup") {
        "oneshot"
    } else {
        "simple"
    };
    let after = if name == "server" {
        "network-online.target cortana-embedding.service"
    } else if name == "sync" {
        "cortana-embedding.service"
    } else {
        "network-online.target"
    };
    let restart = if service_type == "simple" {
        "Restart=on-failure\nRestartSec=5\n"
    } else {
        ""
    };
    let home_cache = dirs::home_dir()
        .map(|home| home.join(".cache/huggingface"))
        .map(|path| systemd_quote(&path.display().to_string()))
        .unwrap_or_default();
    format!(
        "[Unit]\nDescription={description}\nAfter={after}\n\n[Service]\nType={service_type}\nExecStart={arguments}\nWorkingDirectory={}\n{restart}UMask=0077\nNoNewPrivileges=true\nPrivateTmp=true\nProtectSystem=strict\nProtectHome=read-only\nReadWritePaths={}{}\nRestrictAddressFamilies=AF_UNIX AF_INET AF_INET6\n{}",
        systemd_quote(&working_directory.display().to_string()),
        systemd_quote(&config.data_dir.display().to_string()),
        if home_cache.is_empty() {
            String::new()
        } else {
            format!(" {home_cache}")
        },
        if service_type == "simple" {
            "\n[Install]\nWantedBy=default.target\n"
        } else {
            ""
        }
    )
}

fn systemd_timer_unit_body(name: &str, seconds: u64) -> String {
    let description = if name == "sync" {
        "Synchronize Cortana knowledge sources"
    } else {
        "Back up Cortana"
    };
    let service = systemd_service_unit(name);
    format!(
        "[Unit]\nDescription={description}\n\n[Timer]\nOnBootSec=5m\nOnUnitActiveSec={}s\nPersistent=true\nRandomizedDelaySec=30\nUnit={service}\n\n[Install]\nWantedBy=timers.target\n",
        seconds.max(60)
    )
}

fn systemd_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

fn bounded_command_error(bytes: &[u8]) -> String {
    let end = bytes.len().min(2048);
    let value = String::from_utf8_lossy(&bytes[..end])
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect::<String>();
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        "no diagnostic output".into()
    } else {
        value
    }
}

fn unsupported_services() -> Vec<ServiceStatus> {
    managed_services()
        .map(|(name, label)| ServiceStatus {
            name,
            label,
            installed: false,
            loaded: false,
            state: None,
            pid: None,
            last_exit_status: None,
        })
        .collect()
}

fn unsupported_service_manager() -> &'static str {
    "service management supports macOS launchd, Linux systemd user services, and Windows Task Scheduler"
}

fn require_macos() -> Result<()> {
    anyhow::ensure!(
        cfg!(target_os = "macos"),
        "service management supports macOS launchd, Linux systemd user services, and Windows Task Scheduler"
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
        let safe = configured_jobs(
            &common,
            Some(Path::new("/tmp/web")),
            900,
            86_400,
            true,
            false,
            false,
        );
        assert!(
            safe.iter().all(|job| job.label != "ai.cortana.sync"),
            "safe installation must be query-only by default"
        );
        let scheduled = configured_jobs(
            &common,
            Some(Path::new("/tmp/web")),
            900,
            86_400,
            true,
            true,
            false,
        );
        assert!(
            scheduled.iter().any(|job| job.label == "ai.cortana.sync"),
            "explicit opt-in must install the recurring sync job"
        );
    }

    #[test]
    fn no_web_install_uses_api_only_server_arguments() {
        let common = vec!["cortana".into(), "--config".into(), "config.toml".into()];
        let jobs = configured_jobs(&common, None, 900, 86_400, true, false, true);
        let server = jobs
            .iter()
            .find(|job| job.label == "ai.cortana.server")
            .expect("server job");
        assert_eq!(
            server.arguments,
            ["cortana", "--config", "config.toml", "serve", "--no-web"]
        );
    }

    #[test]
    fn service_names_are_fixed_and_launchctl_status_is_structured() {
        assert_eq!(service_label("server").unwrap(), "ai.cortana.server");
        assert!(service_label("../server").is_err());
        assert_eq!(
            parse_launchctl_status("state = running\n\tpid = 123\n\tlast exit code = 0\n"),
            (Some("running".into()), Some(123), Some(0))
        );
    }

    #[test]
    fn systemd_units_and_status_are_fixed_and_structured() {
        assert_eq!(
            systemd_service_unit("embedding"),
            "cortana-embedding.service"
        );
        assert_eq!(systemd_service_unit("server"), "cortana.service");
        assert_eq!(systemd_timer_unit("sync"), "cortana-sync.timer");
        assert_eq!(systemd_job_name("ai.cortana.backup"), Some("backup"));
        assert_eq!(systemd_job_name("arbitrary"), None);
        assert_eq!(
            parse_systemd_status(
                "ActiveState=active\nSubState=running\nMainPID=123\nExecMainStatus=0\n"
            ),
            (Some("running".into()), Some(123), Some(0))
        );
        assert_eq!(
            parse_systemd_status(
                "ActiveState=failed\nSubState=failed\nMainPID=0\nExecMainStatus=1\n"
            ),
            (Some("failed".into()), None, Some(1))
        );
    }

    #[test]
    fn systemd_timer_and_argument_rendering_escape_paths_and_bound_intervals() {
        assert!(systemd_timer_unit_body("sync", 1).contains("OnUnitActiveSec=60s"));
        assert!(systemd_timer_unit_body("backup", 86_400).contains("OnUnitActiveSec=86400s"));
        assert_eq!(systemd_quote("/tmp/a b%$\"c"), "\"/tmp/a b%%$$\\\"c\"");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_task_status_parses_state_and_hex_exit_codes() {
        assert_eq!(
            parse_windows_task_status("Status: Running\nLast Run Result: 0x0\n"),
            (Some("running".into()), Some(0))
        );
        assert_eq!(
            parse_windows_task_status("Status: Ready\nLast Run Result: 0x41301\n"),
            (Some("ready".into()), Some(267_009))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_command_line_quotes_paths_and_embedded_quotes() {
        assert_eq!(
            windows_command_line(&[
                r"C:\Program Files\Cortana\cortana.exe".into(),
                "--config".into(),
                r#"C:\Users\A "B"\config.toml"#.into(),
            ]),
            r#""C:\Program Files\Cortana\cortana.exe" "--config" "C:\Users\A \"B\"\config.toml""#
        );
    }
}
