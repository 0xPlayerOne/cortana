//! Buzz community (team) discovery from the read-only `agents/teams.json`
//! identity file.
//!
//! Buzz stores its agent identities under the source root in
//! `agents/teams.json`, a JSON array of records with stable `id` and `name`
//! fields. Community assignment picks which of those communities the current
//! source's workspace may index: the Desktop chooser persists explicitly
//! checked community ids into the per-source `communities` field with display
//! names kept index-aligned in `community_names`. The identity file is
//! treated as read-only data, never as an executable, and identity is never
//! inferred from persona event content.
//!
//! Discovery reads only the identity file: it never runs ingestion, never
//! starts a sync, never touches the retention database or agent logs, and
//! never writes into the Buzz data directory. The file must be a regular,
//! non-symlink file bounded in size; missing, malformed, or duplicate
//! entries fail closed so a stale or tampered identity file can never
//! mislabel an assignment.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, SourceConfig};
use crate::oauth_common;

/// Buzz keeps the community identity file at `<root>/agents/teams.json`.
const AGENTS_DIRECTORY: &str = "agents";
const TEAMS_FILE_NAME: &str = "teams.json";
/// The identity file is a small local JSON array; 512 KiB bounds it far
/// beyond the realistic worst case while keeping a tampered or runaway file
/// out of the parser.
const MAX_TEAMS_FILE_BYTES: u64 = 512 * 1024;
/// At most this many communities are surfaced; the payload stays bounded
/// and `truncated` reports when the identity file holds more.
const MAX_COMMUNITIES: usize = 100;
/// Buzz team ids are stable strings such as `builtin-team:welcome` or a
/// generated identifier; bound the length instead of assuming a format.
const MAX_COMMUNITY_ID_CHARS: usize = 128;
const MAX_COMMUNITY_NAME_CHARS: usize = 128;

#[derive(Debug, Serialize)]
pub struct CommunityList {
    pub communities: Vec<CommunitySummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CommunitySummary {
    pub id: String,
    pub name: String,
}

/// A validated Buzz `agents/teams.json` record. Only the stable `id` and
/// `name` fields are read; the identity file may carry additional record
/// fields (descriptions, persona memberships, timestamps) that discovery
/// deliberately ignores.
#[derive(Debug, Deserialize, PartialEq)]
struct TeamFileEntry {
    id: String,
    name: String,
}

/// List the bounded Buzz communities recorded in the source's read-only
/// `agents/teams.json` identity file. The response stays a bounded list so
/// the Desktop chooser and per-workspace assignment follow the same contract
/// as Discord server and Slack workspace discovery.
pub fn list_communities(config: &Config, selected: &str) -> Result<CommunityList> {
    let source = configured_buzz_source(config, selected)?;
    let path = teams_file_path(source)?;
    let entries = read_teams_file(&path, &source.name)?;
    // `truncated` reports only when the identity file holds more communities
    // than the bounded surface list, so exactly `MAX_COMMUNITIES` entries are
    // a complete (untruncated) listing.
    let truncated = entries.len() > MAX_COMMUNITIES;
    let mut communities = Vec::new();
    for entry in entries.into_iter().take(MAX_COMMUNITIES) {
        communities.push(CommunitySummary {
            id: validate_community_id(&entry.id, &source.name)?,
            name: validate_community_name(&entry.name, &source.name)?,
        });
    }
    Ok(CommunityList {
        communities,
        truncated,
    })
}

/// Resolve the read-only identity file for a Buzz source. The source root is
/// the Buzz data directory (for example
/// `~/Library/Application Support/xyz.block.buzz.app` on macOS); without it
/// there is no identity file to read, so the command fails closed with
/// guidance instead of guessing a location.
fn teams_file_path(source: &SourceConfig) -> Result<PathBuf> {
    let root = source.root.as_deref().with_context(|| {
        format!(
            "Buzz source {} requires a data directory for community discovery; configure the Buzz `root` path first",
            source.name
        )
    })?;
    anyhow::ensure!(
        root.is_absolute()
            && !root.components().any(|component| matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )),
        "Buzz source {} root must be an absolute path without `..` or `.` components",
        source.name
    );
    Ok(root.join(AGENTS_DIRECTORY).join(TEAMS_FILE_NAME))
}

/// Read and validate the bounded identity file. Missing files, symlinks,
/// non-regular files, oversized files, malformed entries, and duplicate ids
/// all fail closed.
fn read_teams_file(path: &Path, source_name: &str) -> Result<Vec<TeamFileEntry>> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Buzz community discovery for {source_name} found no identity file at {}; make sure the Buzz data directory is configured as the source root and Buzz has written agents/teams.json",
                path.display()
            );
        }
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        metadata.is_file(),
        "Buzz teams.json must be a regular file: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.len() <= MAX_TEAMS_FILE_BYTES,
        "Buzz teams.json exceeds the {MAX_TEAMS_FILE_BYTES} byte safety limit: {}",
        path.display()
    );
    let body =
        fs::read(path).with_context(|| format!("read Buzz teams.json {}", path.display()))?;
    // Parse the top level as a JSON array first so a record that is not an
    // object, or is missing `id`/`name`, can be reported by entry index.
    let raw: Vec<serde_json::Value> = serde_json::from_slice(&body).map_err(|error| {
        anyhow::anyhow!(
            "Buzz teams.json must be a JSON array of records with string `id` and `name` fields: {} ({error})",
            path.display()
        )
    })?;
    let mut entries = Vec::with_capacity(raw.len());
    let mut seen = std::collections::HashSet::new();
    for (index, value) in raw.iter().enumerate() {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Buzz teams.json entry {index} must be an object"))?;
        let id = object
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("Buzz teams.json entry {index} must have a string `id`")
            })?;
        let name = object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("Buzz teams.json entry {index} must have a string `name`")
            })?;
        validate_community_id(id, source_name)
            .with_context(|| format!("Buzz teams.json entry {index} has an invalid id"))?;
        validate_community_name(name, source_name)
            .with_context(|| format!("Buzz teams.json entry {index} has an invalid name"))?;
        anyhow::ensure!(
            seen.insert(id),
            "Buzz teams.json entry {index} duplicates community id `{id}`"
        );
        entries.push(TeamFileEntry {
            id: id.to_string(),
            name: name.trim().to_string(),
        });
    }
    Ok(entries)
}

/// Buzz community ids are stable, printable strings with a bounded length.
/// Unlike Slack team ids they have no fixed prefix, so only the safety bounds
/// are enforced and the exact id is preserved for persistence.
pub(crate) fn validate_community_id(value: &str, source_name: &str) -> Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= MAX_COMMUNITY_ID_CHARS
            && value == value.trim()
            && !value.chars().any(char::is_control),
        "Buzz community discovery for {source_name} returned an invalid community id"
    );
    Ok(value.to_string())
}

/// Community display names are trimmed, printable, bounded text. A name that
/// is empty or unusable after trimming fails closed instead of being
/// sanitized, because a malformed identity file must not silently relabel an
/// assignment.
pub(crate) fn validate_community_name(value: &str, source_name: &str) -> Result<String> {
    let trimmed = value.trim();
    anyhow::ensure!(
        !trimmed.is_empty() && trimmed.len() <= MAX_COMMUNITY_NAME_CHARS,
        "Buzz community discovery for {source_name} returned an invalid community name"
    );
    anyhow::ensure!(
        !trimmed.chars().any(char::is_control),
        "Buzz community discovery for {source_name} returned a community name with control characters"
    );
    Ok(trimmed.to_string())
}

fn validate_source_name(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 64,
        "source name is invalid"
    );
    anyhow::ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "source name is invalid"
    );
    Ok(())
}

pub(crate) fn configured_buzz_source<'a>(
    config: &'a Config,
    selected: &str,
) -> Result<&'a SourceConfig> {
    validate_source_name(selected)?;
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        source.kind == "buzz",
        "source {} is not a Buzz connector",
        source.name
    );
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buzz_source(root: Option<PathBuf>) -> SourceConfig {
        SourceConfig {
            name: "agent-buzz".into(),
            kind: "buzz".into(),
            enabled: true,
            project: "agents".into(),
            root,
            source: None,
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

    fn write_identity(root: &Path, body: &str) -> PathBuf {
        let directory = root.join("agents");
        fs::create_dir_all(&directory).expect("agents directory");
        let path = directory.join("teams.json");
        fs::write(&path, body).expect("write identity file");
        path
    }

    #[test]
    fn discovery_reads_only_the_stable_id_and_name_fields() {
        let directory = tempfile::tempdir().unwrap();
        write_identity(
            directory.path(),
            r#"[
                {
                    "id": "builtin-team:welcome",
                    "name": "Welcome Team",
                    "description": "A friendly starter trio",
                    "persona_ids": ["builtin:fizz", "builtin:honey"],
                    "is_builtin": true,
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                },
                {"id": "team:research", "name": "Research"}
            ]"#,
        );
        let config = Config {
            sources: vec![buzz_source(Some(directory.path().to_path_buf()))],
            ..Config::default()
        };
        let list = list_communities(&config, "agent-buzz").expect("discover communities");
        assert_eq!(
            list.communities,
            vec![
                CommunitySummary {
                    id: "builtin-team:welcome".into(),
                    name: "Welcome Team".into(),
                },
                CommunitySummary {
                    id: "team:research".into(),
                    name: "Research".into(),
                },
            ]
        );
        assert!(!list.truncated);
    }

    #[test]
    fn discovery_truncates_at_the_bounded_count() {
        let directory = tempfile::tempdir().unwrap();
        let entries = (0..MAX_COMMUNITIES + 5)
            .map(|index| format!("{{\"id\": \"team-{index:03}\", \"name\": \"Team {index:03}\"}}"))
            .collect::<Vec<_>>()
            .join(",");
        write_identity(directory.path(), &format!("[{entries}]"));
        let config = Config {
            sources: vec![buzz_source(Some(directory.path().to_path_buf()))],
            ..Config::default()
        };
        let list = list_communities(&config, "agent-buzz").expect("discover communities");
        assert_eq!(list.communities.len(), MAX_COMMUNITIES);
        assert!(list.truncated);
        assert_eq!(list.communities[0].id, "team-000");
    }

    #[test]
    fn exact_bounded_count_is_not_truncated() {
        // Truncation means the identity file held *more* communities than the
        // bounded surface list: exactly `MAX_COMMUNITIES` entries are surfaced
        // completely and must not report `truncated`.
        let directory = tempfile::tempdir().unwrap();
        let entries = (0..MAX_COMMUNITIES)
            .map(|index| format!("{{\"id\": \"team-{index:03}\", \"name\": \"Team {index:03}\"}}"))
            .collect::<Vec<_>>()
            .join(",");
        write_identity(directory.path(), &format!("[{entries}]"));
        let config = Config {
            sources: vec![buzz_source(Some(directory.path().to_path_buf()))],
            ..Config::default()
        };
        let list = list_communities(&config, "agent-buzz").expect("discover communities");
        assert_eq!(list.communities.len(), MAX_COMMUNITIES);
        assert!(
            !list.truncated,
            "exactly {MAX_COMMUNITIES} entries is a complete listing"
        );
        assert_eq!(list.communities[0].id, "team-000");
        assert_eq!(list.communities[MAX_COMMUNITIES - 1].id, "team-099");
    }

    #[test]
    fn missing_or_unconfigured_identity_fails_closed() {
        // No root configured: the command fails closed with guidance instead
        // of guessing a Buzz data directory.
        let config = Config {
            sources: vec![buzz_source(None)],
            ..Config::default()
        };
        let error = list_communities(&config, "agent-buzz").expect_err("root must be configured");
        assert!(error.to_string().contains("requires a data directory"));

        // Root configured but the identity file has never been written.
        let directory = tempfile::tempdir().unwrap();
        let config = Config {
            sources: vec![buzz_source(Some(directory.path().to_path_buf()))],
            ..Config::default()
        };
        let error = list_communities(&config, "agent-buzz").expect_err("missing file must fail");
        assert!(error.to_string().contains("found no identity file"));
    }

    #[test]
    fn discovery_requires_a_buzz_connector() {
        let config = Config {
            sources: vec![SourceConfig {
                name: "community".into(),
                kind: "discord".into(),
                ..buzz_source(None)
            }],
            ..Config::default()
        };
        let error = list_communities(&config, "community").expect_err("kind must be buzz");
        assert!(error.to_string().contains("not a Buzz connector"));
    }

    #[test]
    fn malformed_identity_files_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_identity(directory.path(), "not-json");
        let error = read_teams_file(&path, "agent-buzz").expect_err("invalid JSON must fail");
        assert!(error.to_string().contains("JSON array"));

        fs::write(&path, r#"{"id": "not-an-array"}"#).unwrap();
        let error = read_teams_file(&path, "agent-buzz").expect_err("object must fail");
        assert!(error.to_string().contains("JSON array"));

        fs::write(&path, r#"[{"id": "team-a"}]"#).unwrap();
        let error = read_teams_file(&path, "agent-buzz").expect_err("missing name must fail");
        assert!(error.to_string().contains("must have a string `name`"));

        fs::write(
            &path,
            r#"[{"id": "team-a", "name": "Team A"}, {"id": "team-a", "name": "Other"}]"#,
        )
        .unwrap();
        let error = read_teams_file(&path, "agent-buzz").expect_err("duplicate id must fail");
        assert!(error.to_string().contains("duplicates community id"));

        fs::write(
            &path,
            r#"[{"id": "team-a", "name": "Team A"}, {"id": "team-b", "name": ""}]"#,
        )
        .unwrap();
        let error = read_teams_file(&path, "agent-buzz").expect_err("empty name must fail");
        assert!(error.to_string().contains("invalid name"));
    }

    #[test]
    fn oversized_identity_files_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_identity(directory.path(), "[]");
        // Pad the file past the safety bound with a single oversized entry so
        // the size check trips before parsing.
        let oversized = format!(
            "[{{\"id\": \"team-big\", \"name\": \"{}\"}}]",
            "x".repeat(MAX_TEAMS_FILE_BYTES as usize)
        );
        fs::write(&path, oversized).unwrap();
        let error = read_teams_file(&path, "agent-buzz").expect_err("oversized file must fail");
        assert!(error.to_string().contains("safety limit"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_identity_files_fail_closed() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = write_identity(directory.path(), r#"[{"id": "team-a", "name": "Team A"}]"#);
        let linked = directory.path().join("agents-linked");
        fs::create_dir_all(&linked).unwrap();
        let linked_file = linked.join("teams.json");
        symlink(&target, &linked_file).expect("symlink identity file");
        let error = read_teams_file(&linked_file, "agent-buzz").expect_err("symlink must fail");
        assert!(error.to_string().contains("symlink"));
    }

    #[test]
    fn community_ids_and_names_are_bounded() {
        assert_eq!(
            validate_community_id("builtin-team:welcome", "agent-buzz").unwrap(),
            "builtin-team:welcome"
        );
        for invalid in [
            "",
            " ",
            " id-with-space",
            &"x".repeat(129),
            "id\x00nul",
            "id\nbreak",
        ] {
            assert!(
                validate_community_id(invalid, "agent-buzz").is_err(),
                "community id {invalid:?} must be rejected"
            );
        }
        assert_eq!(
            validate_community_name("  Welcome Team  ", "agent-buzz").unwrap(),
            "Welcome Team",
            "names are trimmed"
        );
        assert!(validate_community_name("", "agent-buzz").is_err());
        assert!(validate_community_name(&"x".repeat(129), "agent-buzz").is_err());
        assert!(validate_community_name("bad\x00name", "agent-buzz").is_err());
    }

    #[test]
    fn serialized_communities_never_contain_credentials() {
        let list = CommunityList {
            communities: vec![CommunitySummary {
                id: "builtin-team:welcome".into(),
                name: "Welcome Team".into(),
            }],
            truncated: false,
        };
        let serialized = serde_json::to_string(&list).expect("serialize communities");
        assert!(serialized.contains("Welcome Team"));
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("secret"));
    }
}
