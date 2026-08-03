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
    /// Whether the walk covered the entire source within the requested
    /// budgets. `false` means the walk stopped at a budget, so the plan is a
    /// bounded sample of a larger corpus and must never authorize a
    /// full-corpus (reconciling) sync.
    pub complete: bool,
}

/// Walk a filesystem source and report its scope.
///
/// When `sample` is false (the default, fail-closed mode) the walk errors as
/// soon as the document or byte budget is exceeded. When `sample` is true the
/// walk instead stops at the requested budgets and reports the bounded prefix
/// with `complete: false`, so an explicitly sampled validation can authorize
/// an equally bounded non-reconciling run without ever blessing a full
/// corpus. The wall-clock budget stays a hard error in both modes: a walk
/// that cannot finish within its time budget must never be blessed.
pub fn filesystem_plan(
    root: &Path,
    excludes: &[String],
    max_documents: usize,
    max_bytes: u64,
    max_duration: Duration,
    sample: bool,
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
        complete: true,
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
        if sample {
            // Stop at the requested budgets instead of failing; the reported
            // scope never exceeds them and the plan is marked partial. The
            // time budget is still enforced above as a hard error.
            if plan.documents >= max_documents || plan.bytes.saturating_add(bytes) > max_bytes {
                plan.complete = false;
                return Ok(plan);
            }
        } else {
            anyhow::ensure!(
                plan.documents < max_documents,
                "filesystem source exceeds the {max_documents} document budget"
            );
            anyhow::ensure!(
                plan.bytes.saturating_add(bytes) <= max_bytes,
                "filesystem source exceeds the {max_bytes} byte budget"
            );
        }
        plan.documents = plan.documents.saturating_add(1);
        plan.bytes = plan.bytes.saturating_add(bytes);
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
    // `sync-files` also accepts a single file as its root. In that case the
    // walker yields the root itself, so stripping the root produces an empty
    // path and would collapse every one-file import onto the same source ID.
    // Use the filename as the stable relative identity for that shape while
    // preserving directory-root identities.
    let relative = if path == canonical_root {
        path.file_name().map(Path::new).unwrap_or(path)
    } else {
        path.strip_prefix(canonical_root).unwrap_or(path)
    };
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

        let error = filesystem_plan(
            directory.path(),
            &[],
            1,
            1_000,
            Duration::from_secs(5),
            false,
        )
        .expect_err("document budget");
        assert!(error.to_string().contains("1 document budget"));
    }

    #[test]
    fn sampled_filesystem_plan_truncates_instead_of_failing() {
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::write(directory.path().join("one.rs"), "fn one() {}").expect("first file");
        std::fs::write(directory.path().join("two.rs"), "fn two() {}").expect("second file");

        let plan = filesystem_plan(
            directory.path(),
            &[],
            1,
            1_000,
            Duration::from_secs(5),
            true,
        )
        .expect("sampled plan");
        assert_eq!(plan.documents, 1);
        assert!(!plan.complete, "a walk truncated by its budget is partial");

        let plan = filesystem_plan(
            directory.path(),
            &[],
            2,
            1_000,
            Duration::from_secs(5),
            true,
        )
        .expect("complete sampled plan");
        assert_eq!(plan.documents, 2);
        assert!(
            plan.complete,
            "a sample covering the whole corpus is complete"
        );
    }

    #[test]
    fn sampled_filesystem_plan_never_reports_scope_beyond_its_budgets() {
        let directory = tempfile::tempdir().expect("temporary directory");
        std::fs::write(directory.path().join("one.rs"), "aa").expect("first file");
        std::fs::write(directory.path().join("two.rs"), "bb").expect("second file");

        let plan = filesystem_plan(directory.path(), &[], 10, 3, Duration::from_secs(5), true)
            .expect("sampled plan");
        assert_eq!(plan.documents, 1);
        assert!(plan.bytes <= 3, "byte scope must stay within the budget");
        assert!(!plan.complete);
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
    fn filesystem_source_uses_filename_for_a_single_file_root() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join("single.rs");
        std::fs::write(&file, "fn single() {}").expect("source file");

        let documents = filesystem_documents(&file, "code", "work").expect("documents");
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_id, "single.rs");
        assert_eq!(documents[0].title, "single.rs");

        let plan = filesystem_plan(&file, &[], 1, 1_000, Duration::from_secs(5), false)
            .expect("single-file plan");
        assert_eq!(plan.documents, 1);
        assert!(plan.complete);
    }

    #[test]
    fn filesystem_directory_roots_keep_relative_identifiers_for_nested_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("src");
        std::fs::create_dir_all(root.join("nested")).expect("nested directory");
        std::fs::write(root.join("nested/mod.rs"), "mod source").expect("source file");

        let documents = filesystem_documents(&root, "code", "work").expect("documents");

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].source_id, "nested/mod.rs");
        assert_eq!(documents[0].title, "nested/mod.rs");
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
