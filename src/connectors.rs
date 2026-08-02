use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use ignore::WalkBuilder;
use reqwest::Url;
use serde_json::json;

use crate::model::Document;

const TEXT_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cs", "css", "go", "h", "hpp", "html", "java", "js", "jsx", "json", "kt",
    "md", "mdx", "mjs", "py", "rb", "rs", "rst", "sh", "sol", "sql", "swift", "toml", "ts", "tsx",
    "txt", "xml", "yaml", "yml", "zsh",
];

#[derive(Clone, Debug, serde::Serialize)]
pub struct FilesystemPlan {
    pub documents: usize,
    pub bytes: u64,
}

pub fn filesystem_plan(
    root: &Path,
    excludes: &[String],
    max_documents: usize,
    max_bytes: u64,
    max_duration: Duration,
) -> Result<FilesystemPlan> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("source root does not exist: {}", root.display()))?;
    let filter_root = canonical_root.clone();
    let filter_excludes = excludes.to_vec();
    let started = Instant::now();
    let mut plan = FilesystemPlan {
        documents: 0,
        bytes: 0,
    };
    for entry in WalkBuilder::new(&canonical_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(move |entry| {
            !is_generated(entry.path(), &filter_root)
                && !is_excluded(entry.path(), &filter_root, &filter_excludes)
        })
        .build()
    {
        anyhow::ensure!(
            started.elapsed() <= max_duration,
            "filesystem planning exceeded the {} second source budget",
            max_duration.as_secs()
        );
        let entry = entry?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_text(entry.path()) {
            continue;
        }
        let bytes = entry.metadata()?.len();
        if bytes == 0 || bytes > 2_000_000 {
            continue;
        }
        plan.documents = plan.documents.saturating_add(1);
        plan.bytes = plan.bytes.saturating_add(bytes);
        anyhow::ensure!(
            plan.documents <= max_documents,
            "filesystem source exceeds the {max_documents} document budget"
        );
        anyhow::ensure!(
            plan.bytes <= max_bytes,
            "filesystem source exceeds the {max_bytes} byte budget"
        );
    }
    Ok(plan)
}

pub fn filesystem_documents(root: &Path, source: &str, project: &str) -> Result<Vec<Document>> {
    filesystem_documents_with_excludes(root, source, project, &[])
}

pub fn filesystem_documents_with_excludes(
    root: &Path,
    source: &str,
    project: &str,
    excludes: &[String],
) -> Result<Vec<Document>> {
    filesystem_document_iter(root, source, project, excludes)?.collect()
}

pub fn filesystem_document_iter(
    root: &Path,
    source: &str,
    project: &str,
    excludes: &[String],
) -> Result<Box<dyn Iterator<Item = Result<Document>>>> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("source root does not exist: {}", root.display()))?;
    let filter_root = canonical_root.clone();
    let filter_excludes = excludes.to_vec();
    let document_root = canonical_root.clone();
    let source = source.to_string();
    let project = project.to_string();
    let iterator = WalkBuilder::new(&canonical_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(move |entry| {
            !is_generated(entry.path(), &filter_root)
                && !is_excluded(entry.path(), &filter_root, &filter_excludes)
        })
        .build()
        .filter_map(move |entry| match entry {
            Err(error) => Some(Err(error.into())),
            Ok(entry) => match filesystem_document(&entry, &document_root, &source, &project) {
                Ok(Some(document)) => Some(Ok(document)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            },
        });
    Ok(Box::new(iterator))
}

fn filesystem_document(
    entry: &ignore::DirEntry,
    canonical_root: &Path,
    source: &str,
    project: &str,
) -> Result<Option<Document>> {
    let path = entry.path();
    if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_text(path) {
        return Ok(None);
    }
    let metadata = path.metadata()?;
    if metadata.len() > 2_000_000 {
        return Ok(None);
    }
    let content = match std::fs::read_to_string(path) {
        Ok(content) if !content.trim().is_empty() => content,
        _ => return Ok(None),
    };
    let relative = path.strip_prefix(canonical_root).unwrap_or(path);
    let updated_at = metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now);
    let uri = Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| anyhow!("filesystem path cannot be represented as a file URI"))?;
    Ok(Some(Document {
        source: source.to_string(),
        source_id: relative.to_string_lossy().into_owned(),
        title: relative.to_string_lossy().into_owned(),
        content,
        uri: Some(uri),
        updated_at,
        project: project.to_string(),
        acl: Vec::new(),
        metadata: json!({
            "root": canonical_root.file_name().and_then(|name| name.to_str()),
            "extension": path.extension().and_then(|extension| extension.to_str()),
            "bytes": metadata.len(),
        }),
    }))
}

fn is_excluded(path: &Path, root: &Path, excludes: &[String]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    excludes
        .iter()
        .map(Path::new)
        .any(|excluded| !excluded.as_os_str().is_empty() && relative.starts_with(excluded))
}

fn is_text(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| TEXT_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

fn is_generated(path: &Path, root: &Path) -> bool {
    const SKIP: &[&str] = &[
        ".git",
        ".worktrees",
        ".venv",
        "Library",
        "build",
        "coverage",
        "dist",
        "node_modules",
        "target",
    ];
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(|component| SKIP.contains(&component))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_source_applies_relative_excludes_and_generated_defaults() {
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::write(directory.path().join("keep.rs"), "fn keep() {}").expect("keep file");
        std::fs::create_dir_all(directory.path().join("second-brain")).expect("excluded directory");
        std::fs::write(
            directory.path().join("second-brain/private.md"),
            "separate source",
        )
        .expect("excluded file");
        std::fs::create_dir_all(directory.path().join("node_modules"))
            .expect("generated directory");
        std::fs::write(
            directory.path().join("node_modules/generated.js"),
            "generated",
        )
        .expect("generated file");
        std::fs::create_dir_all(directory.path().join(".worktrees/feature"))
            .expect("worktree directory");
        std::fs::write(
            directory.path().join(".worktrees/feature/duplicate.rs"),
            "fn duplicate() {}",
        )
        .expect("worktree file");

        let documents = filesystem_documents_with_excludes(
            directory.path(),
            "code",
            "work",
            &["second-brain".into()],
        )
        .expect("documents");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_id, "keep.rs");
    }

    #[test]
    fn filesystem_plan_stops_before_an_unsafe_scan() {
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::write(directory.path().join("one.rs"), "fn one() {}").expect("first file");
        std::fs::write(directory.path().join("two.rs"), "fn two() {}").expect("second file");

        let error = filesystem_plan(directory.path(), &[], 1, 1_000, Duration::from_secs(5))
            .expect_err("document budget");
        assert!(error.to_string().contains("1 document budget"));
    }

    #[test]
    fn filesystem_source_allows_a_root_named_after_a_generated_directory() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("target").join("project");
        std::fs::create_dir_all(&root).expect("source root");
        std::fs::write(root.join("keep.rs"), "fn keep() {}").expect("source file");

        let documents = filesystem_documents(&root, "code", "work").expect("documents");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_id, "keep.rs");
    }

    #[test]
    fn filesystem_source_escapes_special_characters_in_file_uris() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("project with #hash");
        std::fs::create_dir_all(&root).expect("source root");
        let file = root.join("notes ? draft.md");
        std::fs::write(&file, "encoded source link").expect("source file");

        let documents = filesystem_documents(&root, "code", "work").expect("documents");
        let expected = Url::from_file_path(file.canonicalize().expect("canonical file"))
            .expect("file URL")
            .to_string();

        assert_eq!(documents[0].uri.as_deref(), Some(expected.as_str()));
    }
}
