use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ignore::WalkBuilder;
use serde_json::json;

use crate::model::Document;

const TEXT_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cs", "css", "go", "h", "hpp", "html", "java", "js", "jsx", "json", "kt",
    "md", "mdx", "mjs", "py", "rb", "rs", "rst", "sh", "sol", "sql", "swift", "toml", "ts", "tsx",
    "txt", "xml", "yaml", "yml", "zsh",
];

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
            !is_generated(entry.path())
                && !is_excluded(entry.path(), &filter_root, &filter_excludes)
        })
        .build()
        .filter_map(Result::ok)
        .filter_map(move |entry| {
            match filesystem_document(&entry, &document_root, &source, &project) {
                Ok(Some(document)) => Some(Ok(document)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
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
    Ok(Some(Document {
        source: source.to_string(),
        source_id: relative.to_string_lossy().into_owned(),
        title: relative.to_string_lossy().into_owned(),
        content,
        uri: Some(format!("file://{}", path.to_string_lossy())),
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

fn is_generated(path: &Path) -> bool {
    const SKIP: &[&str] = &[
        ".git",
        ".venv",
        "Library",
        "build",
        "coverage",
        "dist",
        "node_modules",
        "target",
    ];
    path.components()
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
}
