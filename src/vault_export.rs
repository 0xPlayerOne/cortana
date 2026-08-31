//! Deterministic, ACL-scoped Obsidian-compatible Markdown vault exports.
//!
//! Vaults are derived read-only projections. Canonical documents remain in the
//! Cortana store; the private manifest is sufficient to rebuild or remove the
//! projection without treating Markdown edits as ingestion authority.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::store::{DocumentCursor, DocumentDetail, Store};

const MANIFEST_NAME: &str = ".cortana-vault.json";
const MANIFEST_VERSION: u32 = 1;
const MAX_EXPORT_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_OBSIDIAN_STATE_FILES: usize = 1_000;
const MAX_OBSIDIAN_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REPORT_FILES: usize = 100;

pub type VaultProgressCallback = Arc<dyn Fn(VaultExportProgress) + Send + Sync>;

#[derive(Clone)]
pub struct VaultExportOptions {
    pub output: PathBuf,
    pub workspaces: BTreeSet<String>,
    pub principal_acl: Vec<String>,
    pub dry_run: bool,
    pub cancel: Arc<AtomicBool>,
    pub progress: Option<VaultProgressCallback>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VaultExportProgress {
    pub phase: &'static str,
    pub documents_completed: usize,
    pub files_written: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct VaultExportReport {
    pub output: PathBuf,
    pub workspaces: Vec<String>,
    pub documents: usize,
    pub content_rewrites: usize,
    pub unchanged_documents: usize,
    pub deleted_documents: usize,
    pub dry_run: bool,
    pub files: Vec<PathBuf>,
    pub files_truncated: bool,
    pub previous_vault: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VaultManifest {
    format_version: u32,
    derived_read_only: bool,
    workspaces: Vec<String>,
    documents: BTreeMap<String, VaultManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VaultManifestEntry {
    path: String,
    content_sha256: String,
    canonical_revision: String,
}

struct RenderedDocument {
    id: String,
    relative_path: PathBuf,
    markdown: Vec<u8>,
    content_sha256: String,
    canonical_revision: String,
}

pub fn export_vault(store: &Store, options: &VaultExportOptions) -> Result<VaultExportReport> {
    if options.workspaces.is_empty() {
        bail!("vault export requires at least one explicitly selected workspace");
    }
    if options.cancel.load(Ordering::SeqCst) {
        bail!("vault export cancelled before it started");
    }
    emit_progress(options, "scanning", 0, 0);
    let output = absolute_output_path(&options.output)?;
    let parent = output
        .parent()
        .context("vault export destination must have a parent directory")?;
    reject_symlink(&output)?;
    let old_manifest = read_existing_manifest(&output)?;
    let rendered = render_documents(store, options)?;
    let new_ids = rendered
        .iter()
        .map(|document| document.id.as_str())
        .collect::<BTreeSet<_>>();
    let old_entries = old_manifest.as_ref().map(|manifest| &manifest.documents);
    let mut unchanged_documents = 0;
    for document in &rendered {
        let unchanged = old_entries
            .and_then(|entries| entries.get(&document.id))
            .is_some_and(|entry| {
                entry.content_sha256 == document.content_sha256
                    && entry.path == path_string(&document.relative_path)
            });
        if unchanged && regular_file(&output.join(&document.relative_path))? {
            unchanged_documents += 1;
        }
    }
    let content_rewrites = rendered.len().saturating_sub(unchanged_documents);
    let deleted_documents = old_entries
        .map(|entries| {
            entries
                .keys()
                .filter(|id| !new_ids.contains(id.as_str()))
                .count()
        })
        .unwrap_or_default();
    let backup = backup_path(&output)?;
    let report = VaultExportReport {
        output: output.clone(),
        workspaces: options.workspaces.iter().cloned().collect(),
        documents: rendered.len(),
        content_rewrites,
        unchanged_documents,
        deleted_documents,
        dry_run: options.dry_run,
        files: rendered
            .iter()
            .take(MAX_REPORT_FILES)
            .map(|document| output.join(&document.relative_path))
            .collect(),
        files_truncated: rendered.len() > MAX_REPORT_FILES,
        previous_vault: backup.is_dir().then_some(backup.clone()),
    };
    if options.dry_run {
        return Ok(report);
    }
    if content_rewrites == 0 && deleted_documents == 0 && old_manifest.is_some() {
        return Ok(report);
    }
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create vault parent {}", parent.display()))?;

    let stage = staging_path(&output)?;
    fs::create_dir(&stage).with_context(|| {
        format!(
            "failed to create vault staging directory {}",
            stage.display()
        )
    })?;
    let stage_result = (|| -> Result<()> {
        preserve_obsidian_state(&output, &stage)?;
        for (index, document) in rendered.iter().enumerate() {
            if options.cancel.load(Ordering::SeqCst) {
                bail!("vault export cancelled before publication");
            }
            let destination = stage.join(&document.relative_path);
            let destination_parent = destination
                .parent()
                .context("vault document must have a parent directory")?;
            fs::create_dir_all(destination_parent)?;
            let unchanged_source = old_entries
                .and_then(|entries| entries.get(&document.id))
                .filter(|entry| {
                    entry.content_sha256 == document.content_sha256
                        && entry.path == path_string(&document.relative_path)
                })
                .map(|_| output.join(&document.relative_path));
            let unchanged_source = match unchanged_source {
                Some(source) if regular_file(&source)? => Some(source),
                _ => None,
            };
            if let Some(source) = unchanged_source {
                if fs::hard_link(&source, &destination).is_err() {
                    fs::copy(&source, &destination).with_context(|| {
                        format!("failed to retain unchanged vault file {}", source.display())
                    })?;
                }
            } else {
                write_new_file(&destination, &document.markdown, false)?;
            }
            emit_progress(options, "writing", rendered.len(), index.saturating_add(1));
        }
        let manifest = VaultManifest {
            format_version: MANIFEST_VERSION,
            derived_read_only: true,
            workspaces: options.workspaces.iter().cloned().collect(),
            documents: rendered
                .iter()
                .map(|document| {
                    (
                        document.id.clone(),
                        VaultManifestEntry {
                            path: path_string(&document.relative_path),
                            content_sha256: document.content_sha256.clone(),
                            canonical_revision: document.canonical_revision.clone(),
                        },
                    )
                })
                .collect(),
        };
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        manifest_bytes.push(b'\n');
        write_new_file(&stage.join(MANIFEST_NAME), &manifest_bytes, true)?;
        if options.cancel.load(Ordering::SeqCst) {
            bail!("vault export cancelled before publication");
        }
        publish_stage(&output, &stage, &backup)?;
        emit_progress(options, "complete", rendered.len(), rendered.len());
        Ok(())
    })();
    if stage_result.is_err() && stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    stage_result?;

    Ok(VaultExportReport {
        previous_vault: backup.is_dir().then_some(backup),
        ..report
    })
}

fn render_documents(store: &Store, options: &VaultExportOptions) -> Result<Vec<RenderedDocument>> {
    let mut rendered = Vec::new();
    for workspace in &options.workspaces {
        let mut cursor: Option<DocumentCursor> = None;
        loop {
            if options.cancel.load(Ordering::SeqCst) {
                bail!("vault export cancelled while reading canonical documents");
            }
            let page = store.list_documents_scoped(
                Some(workspace),
                None,
                None,
                cursor.as_ref(),
                100,
                &options.principal_acl,
            )?;
            for summary in &page.documents {
                let detail = store
                    .document_scoped(
                        &summary.id,
                        &options.principal_acl,
                        MAX_EXPORT_DOCUMENT_BYTES,
                    )?
                    .context("authorized vault document disappeared during export")?;
                if detail.truncated {
                    bail!(
                        "document {} exceeds the {} byte vault export limit",
                        summary.id,
                        MAX_EXPORT_DOCUMENT_BYTES
                    );
                }
                rendered.push(render_document(&detail));
                emit_progress(options, "scanning", rendered.len(), 0);
            }
            if !page.has_more {
                break;
            }
            cursor = page.documents.last().map(|document| DocumentCursor {
                updated_at: document.updated_at.clone(),
                id: document.id.clone(),
            });
        }
    }
    rendered.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(rendered)
}

fn render_document(detail: &DocumentDetail) -> RenderedDocument {
    let summary = &detail.summary;
    let content = &detail.content;
    let workspace = path_component(&summary.project);
    let source = path_component(&summary.source);
    let folder = detail
        .metadata
        .get("folder")
        .and_then(serde_json::Value::as_str)
        .filter(|value| safe_folder(value))
        .map(path_component);
    let mut relative_path = PathBuf::from(workspace).join(source);
    if let Some(folder) = folder {
        relative_path = relative_path.join(folder);
    }
    let relative_path = relative_path.join(format!("{}.md", path_component(&summary.source_id)));
    let mut markdown = String::from("---\n");
    markdown.push_str(&format!(
        "cortana_document_id: {}\n",
        yaml_string(&summary.id)
    ));
    markdown.push_str("cortana_derived_read_only: true\n");
    markdown.push_str(&format!("title: {}\n", yaml_string(&summary.title)));
    markdown.push_str(&format!("workspace: {}\n", yaml_string(&summary.project)));
    markdown.push_str(&format!("source: {}\n", yaml_string(&summary.source)));
    markdown.push_str(&format!("source_id: {}\n", yaml_string(&summary.source_id)));
    markdown.push_str(&format!(
        "updated_at: {}\n",
        yaml_string(&summary.updated_at)
    ));
    markdown.push_str(&format!(
        "canonical_revision: {}\n",
        yaml_string(&summary.content_revision)
    ));
    if let Some(uri) = &summary.uri {
        markdown.push_str(&format!("source_uri: {}\n", yaml_string(uri)));
    }
    let attachments = supported_attachment_uris(&detail.metadata);
    if !attachments.is_empty() {
        markdown.push_str("attachments:\n");
        for uri in attachments {
            markdown.push_str(&format!("  - {}\n", yaml_string(&uri)));
        }
    }
    if summary.acl.is_empty() {
        markdown.push_str("acl: []\n");
    } else {
        markdown.push_str("acl:\n");
        for label in &summary.acl {
            markdown.push_str(&format!("  - {}\n", yaml_string(label)));
        }
    }
    markdown.push_str("---\n\n");
    markdown.push_str(content);
    if !content.ends_with('\n') {
        markdown.push('\n');
    }
    let markdown = markdown.into_bytes();
    let content_sha256 = hex_digest(&markdown);
    RenderedDocument {
        id: summary.id.clone(),
        relative_path,
        markdown,
        content_sha256,
        canonical_revision: summary.content_revision.clone(),
    }
}

fn supported_attachment_uris(metadata: &serde_json::Value) -> Vec<String> {
    let mut uris = metadata
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|uri| {
            uri.len() <= 4_096
                && !uri.chars().any(char::is_control)
                && matches!(uri.split_once(':'), Some(("https" | "http", _)))
        })
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(64)
        .collect::<Vec<_>>();
    uris.sort();
    uris
}

fn read_existing_manifest(output: &Path) -> Result<Option<VaultManifest>> {
    if !output.exists() {
        return Ok(None);
    }
    if !output.is_dir() {
        bail!(
            "vault export destination is not a directory: {}",
            output.display()
        );
    }
    let manifest_path = output.join(MANIFEST_NAME);
    if !manifest_path.is_file() {
        if directory_is_empty(output)? {
            return Ok(None);
        }
        bail!(
            "refusing to replace an unmanaged directory without {MANIFEST_NAME}: {}",
            output.display()
        );
    }
    reject_symlink(&manifest_path)?;
    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read vault manifest {}", manifest_path.display()))?;
    let manifest: VaultManifest = serde_json::from_slice(&bytes)?;
    if manifest.format_version != MANIFEST_VERSION || !manifest.derived_read_only {
        bail!("unsupported or non-derived Cortana vault manifest");
    }
    Ok(Some(manifest))
}

fn publish_stage(output: &Path, stage: &Path, backup: &Path) -> Result<()> {
    if output.exists() {
        if directory_is_empty(output)? {
            fs::remove_dir(output).with_context(|| {
                format!(
                    "failed to prepare empty vault directory {}",
                    output.display()
                )
            })?;
            fs::rename(stage, output).context("failed to publish staged vault")?;
            return Ok(());
        }
        remove_managed_backup(backup)?;
        fs::rename(output, backup).with_context(|| {
            format!(
                "failed to retain previous complete vault {}",
                output.display()
            )
        })?;
        if let Err(error) = fs::rename(stage, output) {
            let _ = fs::rename(backup, output);
            return Err(error).context("failed to publish staged vault");
        }
    } else {
        fs::rename(stage, output).context("failed to publish staged vault")?;
    }
    Ok(())
}

fn directory_is_empty(path: &Path) -> Result<bool> {
    Ok(path.is_dir() && fs::read_dir(path)?.next().is_none())
}

fn remove_managed_backup(backup: &Path) -> Result<()> {
    if !backup.exists() {
        return Ok(());
    }
    reject_symlink(backup)?;
    if !backup.join(MANIFEST_NAME).is_file() {
        bail!(
            "refusing to replace unmanaged previous-vault path {}",
            backup.display()
        );
    }
    fs::remove_dir_all(backup)
        .with_context(|| format!("failed to rotate previous vault {}", backup.display()))
}

fn preserve_obsidian_state(output: &Path, stage: &Path) -> Result<()> {
    let source = output.join(".obsidian");
    if !source.exists() {
        return Ok(());
    }
    reject_symlink(&source)?;
    if !source.is_dir() {
        bail!("vault .obsidian state is not a directory");
    }
    let destination = stage.join(".obsidian");
    let mut pending = vec![(source, destination)];
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    while let Some((from, to)) = pending.pop() {
        fs::create_dir_all(&to)?;
        for entry in fs::read_dir(&from)? {
            let entry = entry?;
            let metadata = entry.file_type()?;
            if metadata.is_symlink() {
                bail!("vault .obsidian state cannot contain symbolic links");
            }
            let target = to.join(entry.file_name());
            if metadata.is_dir() {
                pending.push((entry.path(), target));
                continue;
            }
            if !metadata.is_file() {
                bail!("vault .obsidian state contains an unsupported file type");
            }
            files += 1;
            bytes = bytes.saturating_add(entry.metadata()?.len());
            if files > MAX_OBSIDIAN_STATE_FILES || bytes > MAX_OBSIDIAN_STATE_BYTES {
                bail!("vault .obsidian state exceeds the preservation budget");
            }
            if fs::hard_link(entry.path(), &target).is_err() {
                fs::copy(entry.path(), target)?;
            }
        }
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &[u8], private: bool) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if private { 0o600 } else { 0o644 });
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to create vault file {}", path.display()))?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn absolute_output_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        bail!("vault export destination must be a named directory");
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn staging_path(output: &Path) -> Result<PathBuf> {
    let name = output
        .file_name()
        .context("vault export destination must have a directory name")?
        .to_string_lossy();
    Ok(output.with_file_name(format!(".{name}.cortana-staging-{}", Uuid::new_v4())))
}

fn backup_path(output: &Path) -> Result<PathBuf> {
    let name = output
        .file_name()
        .context("vault export destination must have a directory name")?
        .to_string_lossy();
    Ok(output.with_file_name(format!(".{name}.cortana-previous")))
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "vault export path cannot be a symbolic link: {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn regular_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn path_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push_str(&format!("_{byte:02X}"));
        }
    }
    if encoded.is_empty() {
        encoded.push_str("unnamed");
    }
    if encoded.len() > 96 {
        let digest = hex_digest(value.as_bytes());
        encoded.truncate(80);
        encoded.push('-');
        encoded.push_str(&digest[..15]);
    }
    encoded
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn safe_folder(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.chars().any(char::is_control)
        && !Path::new(value).is_absolute()
        && !Path::new(value).components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
}

fn emit_progress(
    options: &VaultExportOptions,
    phase: &'static str,
    documents_completed: usize,
    files_written: usize,
) {
    if let Some(progress) = &options.progress {
        progress(VaultExportProgress {
            phase,
            documents_completed,
            files_written,
        });
    }
}
