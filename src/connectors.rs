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
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("source root does not exist: {}", root.display()))?;
    let mut documents = Vec::new();
    for entry in WalkBuilder::new(&canonical_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| !is_generated(entry.path()))
        .build()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_text(path) {
            continue;
        }
        let metadata = path.metadata()?;
        if metadata.len() > 2_000_000 {
            continue;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(content) if !content.trim().is_empty() => content,
            _ => continue,
        };
        let relative = path.strip_prefix(&canonical_root).unwrap_or(path);
        let updated_at = metadata
            .modified()
            .ok()
            .map(DateTime::<Utc>::from)
            .unwrap_or_else(Utc::now);
        documents.push(Document {
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
        });
    }
    Ok(documents)
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
