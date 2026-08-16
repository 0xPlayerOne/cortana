use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;

use crate::config::{Config, SourceConfig};

const DEFAULT_GOOGLE_ACCOUNTS: &[&str] = &["work", "personal", "special"];
const SECRET_NAMES: &[&str] = &["SLACK_BOT_TOKEN"];

#[derive(Debug)]
pub struct HermesMigrationOptions {
    pub config_path: PathBuf,
    pub hermes_home: PathBuf,
    pub developer_root: PathBuf,
    pub data_dir: Option<PathBuf>,
    pub connector_command: Option<PathBuf>,
    pub google_accounts: Vec<String>,
    pub slack_channels: Vec<String>,
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct HermesMigrationReport {
    pub migrated_at: String,
    pub config_path: PathBuf,
    pub secrets_file_created: bool,
    pub google_accounts: Vec<String>,
    pub configured_sources: Vec<String>,
    pub legacy_indexes_retained: Vec<PathBuf>,
}

pub fn migrate_hermes(options: &HermesMigrationOptions) -> Result<HermesMigrationReport> {
    anyhow::ensure!(
        options.force || !options.config_path.exists(),
        "configuration already exists: {}; use --force to replace it",
        options.config_path.display()
    );
    let config_dir = options
        .config_path
        .parent()
        .context("configuration path must have a parent directory")?;
    fs::create_dir_all(config_dir)
        .with_context(|| format!("failed to create {}", config_dir.display()))?;

    let token_dir = config_dir.join("google-tokens");
    let accounts = if options.google_accounts.is_empty() {
        DEFAULT_GOOGLE_ACCOUNTS
            .iter()
            .map(|account| (*account).to_string())
            .collect()
    } else {
        options.google_accounts.clone()
    };
    let mut account_files = Vec::new();
    for account in accounts {
        validate_account_name(&account)?;
        let source = options
            .hermes_home
            .join("google-tokens")
            .join(format!("{account}.json"));
        if !regular_file_no_symlink(&source)? {
            continue;
        }
        let destination = token_dir.join(format!("{account}.json"));
        account_files.push((account, source, destination));
    }

    let secrets_path = config_dir.join("secrets.env");
    let secrets = collect_secrets(&[
        options.hermes_home.join(".env"),
        options.developer_root.join("hermes-infra/.env"),
    ])?;
    let report_path = config_dir.join("hermes-migration-report.json");
    let mut transaction = MigrationTransaction::new(options.force);
    for (_, source, destination) in &account_files {
        transaction.stage_copy(source, destination)?;
    }
    let migrated_accounts = account_files
        .iter()
        .map(|(account, _, _)| account.clone())
        .collect::<Vec<_>>();
    let secrets_file_created = if secrets.is_empty() {
        false
    } else {
        transaction.stage(
            &secrets_path,
            format!("{}\n", secrets.join("\n")).as_bytes(),
        )?;
        true
    };

    let mut config = Config::default();
    if let Some(data_dir) = &options.data_dir {
        config.data_dir.clone_from(data_dir);
    }
    if let Some(command) = &options.connector_command {
        config.connectors.command = vec![command.display().to_string()];
    }
    if secrets_file_created {
        config.runtime.env_file = Some(secrets_path);
    }
    if let Some(router) = find_embedding_router() {
        config.embedding.service.command = vec![
            router.display().to_string(),
            "--model-id".into(),
            config.embedding.model.clone(),
            "--dtype".into(),
            "float16".into(),
            "--hostname".into(),
            "127.0.0.1".into(),
            "--port".into(),
            "6999".into(),
            "--max-batch-tokens".into(),
            "512".into(),
            "--max-batch-requests".into(),
            "16".into(),
            "--max-concurrent-requests".into(),
            "128".into(),
        ];
    }

    let mut work_code = source(
        "work-code",
        "filesystem",
        "work",
        Some(&options.developer_root),
        Some("work-code"),
    );
    work_code.exclude.push("second-brain".into());
    #[cfg(target_os = "macos")]
    config.sources.push(source(
        "personal-notes",
        "apple-notes",
        "personal",
        None,
        None,
    ));
    let legacy_brain = options.developer_root.join("second-brain");
    for (name, project, section) in [
        ("legacy-work-notes", "work", "Nifty League"),
        ("legacy-personal-notes", "personal", "Personal"),
        ("legacy-special-notes", "special", "Pink Binder"),
    ] {
        let root = legacy_brain.join(section).join("Notes");
        if root.is_dir() {
            config
                .sources
                .push(source(name, "filesystem", project, Some(&root), Some(name)));
        }
    }
    for account in &migrated_accounts {
        let token = token_dir.join(format!("{account}.json"));
        let project = account.as_str();
        let mut drive = source(
            &format!("{account}-drive"),
            "google-drive",
            project,
            None,
            Some(&format!("{account}-drive")),
        );
        drive.token = Some(token.clone());
        drive.query = Some("trashed = false".into());
        config.sources.push(drive);

        let mut gmail = source(
            &format!("{account}-gmail"),
            "gmail",
            project,
            None,
            Some(&format!("{account}-gmail")),
        );
        gmail.token = Some(token.clone());
        gmail.query = Some("newer_than:5y".into());
        config.sources.push(gmail);

        let mut calendar = source(
            &format!("{account}-calendar"),
            "google-calendar",
            project,
            None,
            Some(&format!("{account}-calendar")),
        );
        calendar.token = Some(token);
        config.sources.push(calendar);
    }
    if !options.slack_channels.is_empty() {
        let mut slack = source("team-slack", "slack", "work", None, Some("team-slack"));
        slack.channels.clone_from(&options.slack_channels);
        slack.token_env = Some("SLACK_BOT_TOKEN".into());
        slack.enabled = secrets
            .iter()
            .any(|entry| entry.starts_with("SLACK_BOT_TOKEN="));
        config.sources.push(slack);
    }
    let buzz_root = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xyz.block.buzz.app");
    if buzz_root.exists() {
        config.sources.push(source(
            "buzz",
            "buzz",
            "agents",
            Some(&buzz_root),
            Some("buzz"),
        ));
    }
    // The initial code corpus is usually the largest source. Keep it last so
    // a fresh installation makes personal and operational context available
    // before beginning the long code embedding pass.
    config.sources.push(work_code);

    let configured_sources = config
        .sources
        .iter()
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    let legacy_indexes_retained = ["code-index", "second-brain-chroma"]
        .iter()
        .map(|name| options.hermes_home.join(name))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    let report = HermesMigrationReport {
        migrated_at: Utc::now().to_rfc3339(),
        config_path: options.config_path.clone(),
        secrets_file_created,
        google_accounts: migrated_accounts,
        configured_sources,
        legacy_indexes_retained,
    };
    transaction.stage(
        &options.config_path,
        toml::to_string_pretty(&config)?.as_bytes(),
    )?;
    transaction.stage(&report_path, serde_json::to_vec_pretty(&report)?.as_slice())?;
    fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("failed to create {}", config.data_dir.display()))?;
    transaction.publish()?;
    Ok(report)
}

fn source(
    name: &str,
    kind: &str,
    project: &str,
    root: Option<&Path>,
    canonical_source: Option<&str>,
) -> SourceConfig {
    SourceConfig {
        name: name.into(),
        kind: kind.into(),
        enabled: true,
        project: project.into(),
        root: root.map(Path::to_path_buf),
        source: canonical_source.map(str::to_string),
        channels: Vec::new(),
        folders: Vec::new(),
        exclude_folders: Vec::new(),
        servers: Vec::new(),
        teams: Vec::new(),
        team_names: Vec::new(),
        communities: Vec::new(),
        community_names: Vec::new(),
        repositories: Vec::new(),
        token_env: None,
        token: None,
        oauth_client: None,
        query: None,
        labels: Vec::new(),
        max_content_chars: None,
        max_documents: None,
        max_bytes: None,
        max_duration_seconds: None,
        exclude: Vec::new(),
        command: Vec::new(),
        acl: Vec::new(),
    }
}

fn validate_account_name(account: &str) -> Result<()> {
    anyhow::ensure!(
        !account.is_empty()
            && account.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            }),
        "invalid Google account name: {account}"
    );
    Ok(())
}

fn collect_secrets(paths: &[PathBuf]) -> Result<Vec<String>> {
    let mut secrets = Vec::new();
    for path in paths {
        if !regular_file_no_symlink(path)? {
            continue;
        }
        let body = fs::read_to_string(path)
            .with_context(|| format!("failed to read legacy environment {}", path.display()))?;
        for raw in body.lines() {
            let line = raw.trim();
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            if SECRET_NAMES.contains(&name.trim())
                && !value.trim().is_empty()
                && !secrets
                    .iter()
                    .any(|existing: &String| existing.starts_with(&format!("{}=", name.trim())))
            {
                secrets.push(format!("{}={}", name.trim(), value.trim()));
            }
        }
    }
    Ok(secrets)
}

fn find_embedding_router() -> Option<PathBuf> {
    [
        "/opt/homebrew/bin/text-embeddings-router",
        "/usr/local/bin/text-embeddings-router",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
    .or_else(|| {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join("text-embeddings-router"))
                .find(|candidate| candidate.is_file())
        })
    })
}

struct MigrationTransaction {
    outputs: Vec<PendingOutput>,
    replace: bool,
}

struct PendingOutput {
    path: PathBuf,
    temporary: PathBuf,
    backup: Option<PathBuf>,
    published: bool,
}

impl MigrationTransaction {
    fn new(replace: bool) -> Self {
        Self {
            outputs: Vec::new(),
            replace,
        }
    }

    fn stage_copy(&mut self, source: &Path, destination: &Path) -> Result<()> {
        anyhow::ensure!(
            regular_file_no_symlink(source)?,
            "migration source does not exist: {}",
            source.display()
        );
        let contents =
            fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
        self.stage(destination, &contents)
    }

    fn stage(&mut self, path: &Path, contents: &[u8]) -> Result<()> {
        if path_exists(path)? {
            anyhow::ensure!(
                self.replace,
                "refusing to replace {}; use --force to replace migrated files",
                path.display()
            );
            regular_file_no_symlink(path)?;
        }
        anyhow::ensure!(
            self.outputs.iter().all(|output| output.path != path),
            "migration output is staged more than once: {}",
            path.display()
        );
        let parent = path
            .parent()
            .context("secure output path must have a parent directory")?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("cortana"),
            uuid::Uuid::new_v4()
        ));
        write_secure_temporary(&temporary, contents)?;
        self.outputs.push(PendingOutput {
            path: path.to_path_buf(),
            temporary,
            backup: None,
            published: false,
        });
        Ok(())
    }

    fn publish(self) -> Result<()> {
        self.publish_with_failure(None)
    }

    fn publish_with_failure(mut self, fail_at: Option<usize>) -> Result<()> {
        let result = (|| {
            for (index, output) in self.outputs.iter_mut().enumerate() {
                if fail_at == Some(index) {
                    anyhow::bail!("injected migration publication failure at output {index}");
                }
                if path_exists(&output.path)? {
                    anyhow::ensure!(
                        self.replace,
                        "refusing to replace {}; use --force to replace migrated files",
                        output.path.display()
                    );
                    regular_file_no_symlink(&output.path)?;
                    let parent = output
                        .path
                        .parent()
                        .context("secure output path must have a parent directory")?;
                    let backup = parent.join(format!(
                        ".{}.{}.bak",
                        output
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("cortana"),
                        uuid::Uuid::new_v4()
                    ));
                    fs::rename(&output.path, &backup).with_context(|| {
                        format!(
                            "failed to stage existing migration output {}",
                            output.path.display()
                        )
                    })?;
                    output.backup = Some(backup);
                }
                fs::rename(&output.temporary, &output.path)
                    .with_context(|| format!("failed to publish {}", output.path.display()))?;
                output.published = true;
            }
            Ok::<(), anyhow::Error>(())
        })();

        match result {
            Ok(()) => {
                for output in &self.outputs {
                    if let Some(backup) = &output.backup {
                        if let Err(error) = fs::remove_file(backup) {
                            tracing::warn!(
                                path = %backup.display(),
                                %error,
                                "failed to remove migration backup after successful publication"
                            );
                        }
                    }
                }
                Ok(())
            }
            Err(error) => {
                let rollback_error = self.rollback();
                if let Err(rollback_error) = rollback_error {
                    Err(error.context(format!(
                        "migration publication failed and rollback was incomplete: {rollback_error}"
                    )))
                } else {
                    Err(error)
                }
            }
        }
    }

    fn rollback(&mut self) -> Result<()> {
        let mut first_error = None;
        for output in self.outputs.iter_mut().rev() {
            if output.published {
                if let Err(error) = fs::remove_file(&output.path) {
                    if error.kind() != std::io::ErrorKind::NotFound && first_error.is_none() {
                        first_error = Some(anyhow::anyhow!(
                            "failed to remove partially published {}: {error}",
                            output.path.display()
                        ));
                    }
                }
            }
            if let Some(backup) = output.backup.take() {
                if let Err(error) = fs::rename(&backup, &output.path) {
                    if first_error.is_none() {
                        first_error = Some(anyhow::anyhow!(
                            "failed to restore {} from backup: {error}",
                            output.path.display()
                        ));
                    }
                }
            }
            if let Err(error) = fs::remove_file(&output.temporary) {
                if error.kind() != std::io::ErrorKind::NotFound && first_error.is_none() {
                    first_error = Some(anyhow::anyhow!(
                        "failed to remove staged {}: {error}",
                        output.path.display()
                    ));
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for MigrationTransaction {
    fn drop(&mut self) {
        for output in &self.outputs {
            let _ = fs::remove_file(&output.temporary);
            if let Some(backup) = &output.backup {
                let _ = fs::remove_file(backup);
            }
        }
    }
}

fn write_secure_temporary(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok::<(), anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn regular_file_no_symlink(path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        !metadata.file_type().is_symlink(),
        "migration input must not be a symlink: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "migration input must be a regular file: {}",
        path.display()
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(root: &Path) -> HermesMigrationOptions {
        HermesMigrationOptions {
            config_path: root.join("config/cortana/config.toml"),
            hermes_home: root.join("legacy"),
            developer_root: root.join("Developer"),
            data_dir: Some(root.join("data")),
            connector_command: Some(root.join("venv/bin/cortana-connectors")),
            google_accounts: Vec::new(),
            slack_channels: Vec::new(),
            force: false,
        }
    }

    #[test]
    fn migrates_only_whitelisted_secrets_and_configured_accounts() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let options = options(directory.path());
        fs::create_dir_all(options.hermes_home.join("google-tokens")).expect("token directory");
        fs::write(
            options.hermes_home.join("google-tokens/work.json"),
            r#"{"token":"private"}"#,
        )
        .expect("token");
        fs::write(
            options.hermes_home.join(".env"),
            "SLACK_BOT_TOKEN=slack-private\nUNRELATED_SECRET=do-not-copy\n",
        )
        .expect("environment");

        let report = migrate_hermes(&options).expect("migration succeeds");
        assert_eq!(report.google_accounts, ["work"]);
        assert!(report.secrets_file_created);
        let config = Config::load(Some(&options.config_path)).expect("generated config");
        assert!(
            config
                .sources
                .iter()
                .any(|source| source.name == "work-calendar")
        );
        assert_eq!(
            config.connectors.command,
            [directory
                .path()
                .join("venv/bin/cortana-connectors")
                .display()
                .to_string()]
        );
        let secrets = fs::read_to_string(
            options
                .config_path
                .parent()
                .expect("config directory")
                .join("secrets.env"),
        )
        .expect("secrets");
        assert_eq!(secrets, "SLACK_BOT_TOKEN=slack-private\n");
        assert!(!secrets.contains("UNRELATED_SECRET"));
    }

    #[test]
    fn refuses_to_replace_existing_configuration_without_force() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let options = options(directory.path());
        fs::create_dir_all(options.config_path.parent().expect("config directory"))
            .expect("directory");
        fs::write(&options.config_path, "existing = true\n").expect("existing config");

        let error = migrate_hermes(&options).expect_err("must refuse overwrite");
        assert!(error.to_string().contains("use --force"));
    }

    #[cfg(unix)]
    #[test]
    fn writes_migrated_credentials_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let options = options(directory.path());
        fs::create_dir_all(options.hermes_home.join("google-tokens")).expect("token directory");
        fs::write(
            options.hermes_home.join("google-tokens/personal.json"),
            "{}",
        )
        .expect("token");
        fs::write(
            options.hermes_home.join(".env"),
            "SLACK_BOT_TOKEN=private\n",
        )
        .expect("environment");

        migrate_hermes(&options).expect("migration succeeds");
        for path in [
            options.config_path.clone(),
            options
                .config_path
                .parent()
                .expect("config directory")
                .join("secrets.env"),
            options
                .config_path
                .parent()
                .expect("config directory")
                .join("google-tokens/personal.json"),
        ] {
            let mode = fs::metadata(path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_legacy_credentials_before_copying() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let options = options(directory.path());
        fs::create_dir_all(options.hermes_home.join("google-tokens")).expect("token directory");
        symlink(
            options.hermes_home.join("google-tokens/missing.json"),
            options.hermes_home.join("google-tokens/work.json"),
        )
        .expect("token symlink");

        let error = migrate_hermes(&options).expect_err("symlinked token must fail closed");
        assert!(
            error
                .to_string()
                .contains("migration input must not be a symlink")
        );
    }

    #[test]
    fn rolls_back_partial_publication_without_leaving_staged_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = directory.path().join("first.json");
        let second = directory.path().join("second.json");
        fs::write(&second, "old").expect("existing output");

        let mut transaction = MigrationTransaction::new(true);
        transaction
            .stage(&first, b"new-first")
            .expect("stage first");
        transaction
            .stage(&second, b"new-second")
            .expect("stage second");
        let error = transaction
            .publish_with_failure(Some(1))
            .expect_err("injected failure must abort publication");
        assert!(
            error
                .to_string()
                .contains("injected migration publication failure")
        );

        assert!(!first.exists(), "new outputs must be removed on rollback");
        assert_eq!(fs::read_to_string(&second).expect("restored output"), "old");
        let entries = fs::read_dir(directory.path())
            .expect("read directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("directory entries");
        assert_eq!(
            entries.len(),
            1,
            "temporary and backup files must be cleaned up"
        );
    }
}
