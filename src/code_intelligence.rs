//! Revision-aware, bounded code indexing primitives.
//!
//! Canonical authority remains the source [`Document`](crate::model::Document). Parser, symbol,
//! relation, and chunk records are derived, versioned, rebuildable projections.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::Document;

pub const CODE_INDEX_CONTRACT_VERSION: &str = "cortana.code-index.v1";
pub const BOUNDED_PARSER_VERSION: &str = "cortana.bounded-parser.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryIdentity {
    pub repository_id: String,
    pub display_name: String,
    pub canonical_remote: Option<String>,
    pub branch: Option<String>,
    pub default_branch: Option<String>,
    pub commit_sha: Option<String>,
    pub dirty: bool,
    pub detached: bool,
    pub shallow: bool,
    pub worktree: bool,
    pub submodule: bool,
    pub git_available: bool,
    pub observed_at: DateTime<Utc>,
}

impl RepositoryIdentity {
    pub fn revision_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.commit_sha.as_deref().unwrap_or("no-git"),
            self.branch.as_deref().unwrap_or("detached"),
            if self.dirty { "dirty" } else { "committed" }
        )
    }

    pub fn metadata(&self) -> Value {
        json!({
            "contract_version": CODE_INDEX_CONTRACT_VERSION,
            "repository_id": self.repository_id,
            "repository": self.display_name,
            "canonical_remote": self.canonical_remote,
            "branch": self.branch,
            "default_branch": self.default_branch,
            "commit_sha": self.commit_sha,
            "revision": self.revision_key(),
            "dirty": self.dirty,
            "detached": self.detached,
            "shallow": self.shallow,
            "worktree": self.worktree,
            "submodule": self.submodule,
            "git_available": self.git_available,
            "observed_at": self.observed_at,
        })
    }
}

/// Inspect a repository without leaking an absolute local path. Git failures produce an explicit
/// no-Git identity so plain source trees remain indexable.
pub fn inspect_repository(root: &Path) -> Result<RepositoryIdentity> {
    let canonical = root
        .canonicalize()
        .with_context(|| format!("source root does not exist: {}", root.display()))?;
    let repo_root = if canonical.is_file() {
        canonical.parent().unwrap_or(&canonical)
    } else {
        canonical.as_path()
    };
    let display_name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository")
        .to_string();
    let inside = git(repo_root, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|value| value == "true");
    if !inside {
        return Ok(RepositoryIdentity {
            // The absolute path is hashed, never emitted. This distinguishes
            // same-named local roots while keeping host paths private.
            repository_id: stable_hash(&format!("local:{}", repo_root.display())),
            display_name,
            canonical_remote: None,
            branch: None,
            default_branch: None,
            commit_sha: None,
            dirty: false,
            detached: false,
            shallow: false,
            worktree: false,
            submodule: false,
            git_available: false,
            observed_at: Utc::now(),
        });
    }
    let remote =
        git(repo_root, &["remote", "get-url", "origin"]).and_then(|value| sanitize_remote(&value));
    let branch = git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let commit_sha = git(repo_root, &["rev-parse", "HEAD"]);
    let common_dir = git(repo_root, &["rev-parse", "--git-common-dir"]);
    let git_dir = git(repo_root, &["rev-parse", "--git-dir"]);
    let default_branch = git(
        repo_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .and_then(|value| value.strip_prefix("origin/").map(str::to_string))
    .or_else(|| {
        ["main", "master"]
            .into_iter()
            .find(|name| {
                git(
                    repo_root,
                    &[
                        "show-ref",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{name}"),
                    ],
                )
                .is_some()
            })
            .map(str::to_string)
    });
    let identity_seed = remote.as_deref().unwrap_or(&display_name);
    Ok(RepositoryIdentity {
        repository_id: stable_hash(identity_seed),
        display_name,
        canonical_remote: remote,
        branch: branch.clone(),
        default_branch,
        commit_sha,
        dirty: git(
            repo_root,
            &["status", "--porcelain", "--untracked-files=normal"],
        )
        .is_some_and(|value| !value.is_empty()),
        detached: branch.is_none(),
        shallow: git(repo_root, &["rev-parse", "--is-shallow-repository"])
            .is_some_and(|value| value == "true"),
        worktree: common_dir
            .zip(git_dir)
            .is_some_and(|(common, local)| common != local),
        submodule: repo_root.join(".git").is_file(),
        git_available: true,
        observed_at: Utc::now(),
    })
}

fn git(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn sanitize_remote(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('/') || value.starts_with("file:") {
        return None;
    }
    if !value.contains("://")
        && let Some((_, host_path)) = value.split_once('@')
    {
        let (host, path) = host_path.split_once(':')?;
        return Some(format!("https://{host}/{}", path.trim_end_matches(".git")));
    }
    let scheme = value.find("://")?;
    let prefix = &value[..scheme + 3];
    let rest = &value[scheme + 3..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = rest[..authority_end].rsplit('@').next()?;
    let path = rest[authority_end..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git");
    Some(format!("{prefix}{authority}{path}"))
}

pub fn is_generated_or_vendor(path: &Path) -> (bool, bool) {
    let parts = path.components().filter_map(|part| match part {
        Component::Normal(value) => value.to_str(),
        _ => None,
    });
    let mut generated = false;
    let mut vendor = false;
    for part in parts {
        generated |= matches!(
            part,
            "dist" | "build" | "coverage" | "target" | ".next" | ".worktrees"
        ) || part.ends_with(".generated.rs")
            || part.ends_with(".generated.ts");
        vendor |= matches!(part, "vendor" | "node_modules" | ".venv");
    }
    (generated, vendor)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Java,
    Cpp,
    Swift,
    Ruby,
    Unknown,
}

impl Language {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::Java => "java",
            Self::Cpp => "cpp",
            Self::Swift => "swift",
            Self::Ruby => "ruby",
            Self::Unknown => "unknown",
        }
    }
}

pub fn detect_language(path: &str) -> Language {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => Language::Rust,
        "py" => Language::Python,
        "ts" | "tsx" => Language::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
        "go" => Language::Go,
        "java" | "kt" => Language::Java,
        "c" | "cc" | "cpp" | "h" | "hpp" => Language::Cpp,
        "swift" => Language::Swift,
        "rb" => Language::Ruby,
        _ => Language::Unknown,
    }
}

#[derive(Clone, Debug)]
pub struct ParseLimits {
    pub max_bytes: usize,
    pub max_symbols: usize,
    pub max_relations: usize,
    pub timeout: Duration,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_bytes: 2_000_000,
            max_symbols: 20_000,
            max_relations: 40_000,
            timeout: Duration::from_millis(250),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Complete,
    Partial,
    Unsupported,
    Oversized,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Module,
    Class,
    Interface,
    Struct,
    Enum,
    Trait,
    Function,
    Method,
    Constant,
    Variable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolRole {
    Definition,
    Declaration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeSymbol {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub role: SymbolRole,
    pub language: Language,
    pub repository_id: Option<String>,
    pub revision: Option<String>,
    pub file: String,
    pub span: SourceSpan,
    pub signature: String,
    pub visibility: Option<String>,
    pub container: Option<String>,
    pub documentation: Option<String>,
    pub aliases: Vec<String>,
    pub generated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Import,
    Dependency,
    Inheritance,
    Implementation,
    Call,
    Containment,
    Reference,
    Export,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodeRelation {
    pub id: String,
    pub kind: RelationKind,
    pub from_symbol_id: Option<String>,
    pub from_name: String,
    pub to_symbol_id: Option<String>,
    pub to_name: String,
    pub repository_id: Option<String>,
    pub revision: Option<String>,
    pub file: String,
    pub span: SourceSpan,
    pub parser_version: String,
    pub confidence: f32,
    pub origin: String,
    pub resolved: bool,
    pub dynamic: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParseOutput {
    pub contract_version: String,
    pub parser_version: String,
    pub language: Language,
    pub status: ParseStatus,
    pub content_hash: String,
    pub cache_key: String,
    pub diagnostics: Vec<String>,
    pub symbols: Vec<CodeSymbol>,
    pub relations: Vec<CodeRelation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SymbolSearchHit {
    pub symbol: CodeSymbol,
    pub exact: bool,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RelationPage {
    pub relations: Vec<CodeRelation>,
    pub next_cursor: Option<usize>,
    pub truncated: bool,
}

/// Replaceable parser boundary. Implementations return derived data and never mutate Documents.
pub trait CodeParser: Send + Sync {
    fn version(&self) -> &'static str;
    fn parse(
        &self,
        document: &Document,
        limits: &ParseLimits,
        cancelled: &AtomicBool,
    ) -> ParseOutput;
}

#[derive(Default)]
pub struct BoundedSyntaxParser;

impl CodeParser for BoundedSyntaxParser {
    fn version(&self) -> &'static str {
        BOUNDED_PARSER_VERSION
    }

    fn parse(
        &self,
        document: &Document,
        limits: &ParseLimits,
        cancelled: &AtomicBool,
    ) -> ParseOutput {
        parse_document(document, limits, cancelled)
    }
}

pub fn parse_document(
    document: &Document,
    limits: &ParseLimits,
    cancelled: &AtomicBool,
) -> ParseOutput {
    let language = detect_language(&document.source_id);
    let hash = stable_hash(&document.content);
    let cache_key = stable_hash(&format!(
        "{hash}:{}:{BOUNDED_PARSER_VERSION}:{}:{}:{}:{}",
        language.as_str(),
        limits.max_bytes,
        limits.max_symbols,
        limits.max_relations,
        limits.timeout.as_millis()
    ));
    let mut output = ParseOutput {
        contract_version: CODE_INDEX_CONTRACT_VERSION.into(),
        parser_version: BOUNDED_PARSER_VERSION.into(),
        language,
        status: ParseStatus::Complete,
        content_hash: hash,
        cache_key,
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        relations: Vec::new(),
    };
    if cancelled.load(Ordering::Relaxed) {
        output.status = ParseStatus::Cancelled;
        return output;
    }
    if document.content.len() > limits.max_bytes {
        output.status = ParseStatus::Oversized;
        output
            .diagnostics
            .push("file exceeds parser byte budget".into());
        return output;
    }
    if language == Language::Unknown {
        output.status = ParseStatus::Unsupported;
        return output;
    }
    let started = Instant::now();
    let repo = code_value(document, "repository_id");
    let revision = code_value(document, "revision");
    let generated = document
        .metadata
        .get("code")
        .and_then(|v| v.get("generated"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut containers: Vec<(usize, String)> = Vec::new();
    let mut declarations = BTreeMap::<String, Vec<String>>::new();
    let mut imports = Vec::<(String, SourceSpan)>::new();
    let mut offset = 0usize;
    let mut pending_docs = Vec::<String>::new();
    for (line_index, line) in document.content.split_inclusive('\n').enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            output.status = ParseStatus::Cancelled;
            break;
        }
        if started.elapsed() > limits.timeout {
            output.status = ParseStatus::TimedOut;
            output
                .diagnostics
                .push("parser time budget exceeded".into());
            break;
        }
        let bare = line.trim_end_matches(['\r', '\n']);
        let trimmed = bare.trim_start();
        let indent = bare.len() - trimmed.len();
        while containers
            .last()
            .is_some_and(|(level, _)| *level >= indent && indent > 0)
        {
            containers.pop();
        }
        if is_doc_comment(trimmed, language) {
            pending_docs.push(
                trimmed
                    .trim_start_matches(['/', '#', '!', '*'])
                    .trim()
                    .to_string(),
            );
            offset += line.len();
            continue;
        }
        let span = SourceSpan {
            start_byte: offset + indent,
            end_byte: offset + bare.len(),
            start_line: line_index + 1,
            end_line: line_index + 1,
        };
        if let Some(target) = import_target(trimmed, language) {
            imports.push((target, span.clone()));
        }
        if let Some((name, kind, signature, visibility)) = declaration(trimmed, language) {
            if output.symbols.len() >= limits.max_symbols {
                output.status = ParseStatus::Partial;
                output.diagnostics.push("symbol budget exhausted".into());
                break;
            }
            let container = containers.last().map(|(_, name)| name.clone());
            let qualified_name = container
                .as_ref()
                .map_or_else(|| name.clone(), |parent| format!("{parent}::{name}"));
            let id = symbol_id(
                repo.as_deref(),
                revision.as_deref(),
                &document.source_id,
                &qualified_name,
                &kind,
                &span,
            );
            declarations
                .entry(name.clone())
                .or_default()
                .push(id.clone());
            output.symbols.push(CodeSymbol {
                id: id.clone(),
                name: name.clone(),
                qualified_name: qualified_name.clone(),
                kind: kind.clone(),
                role: if matches!(kind, SymbolKind::Interface | SymbolKind::Trait) {
                    SymbolRole::Declaration
                } else {
                    SymbolRole::Definition
                },
                language,
                repository_id: repo.clone(),
                revision: revision.clone(),
                file: document.source_id.clone(),
                span,
                signature,
                visibility,
                container,
                documentation: (!pending_docs.is_empty()).then(|| pending_docs.join("\n")),
                aliases: Vec::new(),
                generated,
            });
            pending_docs.clear();
            if matches!(
                kind,
                SymbolKind::Class
                    | SymbolKind::Interface
                    | SymbolKind::Struct
                    | SymbolKind::Enum
                    | SymbolKind::Trait
                    | SymbolKind::Module
            ) {
                containers.push((indent, qualified_name));
            }
        } else if !trimmed.is_empty() {
            pending_docs.clear();
        }
        offset += line.len();
    }
    for symbol in &output.symbols {
        if output.relations.len() >= limits.max_relations {
            output.status = ParseStatus::Partial;
            output.diagnostics.push("relation budget exhausted".into());
            break;
        }
        let Some(container) = symbol.container.as_deref() else {
            continue;
        };
        let parent = output
            .symbols
            .iter()
            .find(|candidate| candidate.qualified_name == container);
        output.relations.push(relation(
            document,
            repo.clone(),
            revision.clone(),
            RelationKind::Containment,
            Some(symbol.id.clone()),
            &symbol.name,
            parent.map(|candidate| candidate.id.clone()),
            container.to_string(),
            symbol.span.clone(),
            1.0,
            "syntax",
            false,
        ));
    }
    for (target, span) in imports {
        if output.relations.len() >= limits.max_relations {
            output.status = ParseStatus::Partial;
            output.diagnostics.push("relation budget exhausted".into());
            break;
        }
        let candidates = declarations.get(&target);
        let resolved_target = candidates
            .filter(|values| values.len() == 1)
            .map(|values| values[0].clone());
        let ambiguous = candidates.is_some_and(|values| values.len() > 1);
        output.relations.push(relation(
            document,
            repo.clone(),
            revision.clone(),
            RelationKind::Import,
            None,
            &document.source_id,
            resolved_target,
            target,
            span,
            if ambiguous { 0.5 } else { 1.0 },
            if ambiguous { "ambiguous" } else { "syntax" },
            false,
        ));
    }
    // Resolve only exact, unique identifier calls. Unknown and duplicate targets stay explicit.
    let mut line_offset = 0usize;
    'relation_lines: for (line_index, line) in document.content.split_inclusive('\n').enumerate() {
        if output.relations.len() >= limits.max_relations {
            output.status = ParseStatus::Partial;
            output.diagnostics.push("relation budget exhausted".into());
            break 'relation_lines;
        }
        let owner = output
            .symbols
            .iter()
            .rev()
            .find(|symbol| symbol.span.start_byte <= line_offset);
        for token in call_tokens(line) {
            if output.relations.len() >= limits.max_relations {
                output.status = ParseStatus::Partial;
                output.diagnostics.push("relation budget exhausted".into());
                break 'relation_lines;
            }
            if owner.is_some_and(|symbol| {
                symbol.name == token && symbol.span.start_line == line_index + 1
            }) {
                continue;
            }
            let candidates = declarations.get(&token);
            let target = candidates
                .filter(|values| values.len() == 1)
                .map(|values| values[0].clone());
            let ambiguous = candidates.is_some_and(|values| values.len() > 1);
            let span = SourceSpan {
                start_byte: line_offset,
                end_byte: line_offset + line.trim_end_matches(['\r', '\n']).len(),
                start_line: line_index + 1,
                end_line: line_index + 1,
            };
            output.relations.push(relation(
                document,
                repo.clone(),
                revision.clone(),
                RelationKind::Call,
                owner.map(|symbol| symbol.id.clone()),
                owner.map_or(document.source_id.as_str(), |symbol| symbol.name.as_str()),
                target.clone(),
                token,
                span,
                if target.is_some() { 0.95 } else { 0.5 },
                if ambiguous { "ambiguous" } else { "syntax" },
                false,
            ));
        }
        for (kind, target_name) in inheritance_targets(line, language) {
            if output.relations.len() >= limits.max_relations {
                output.status = ParseStatus::Partial;
                output.diagnostics.push("relation budget exhausted".into());
                break 'relation_lines;
            }
            let candidates = declarations.get(&target_name);
            let target = candidates
                .filter(|values| values.len() == 1)
                .map(|values| values[0].clone());
            let span = SourceSpan {
                start_byte: line_offset,
                end_byte: line_offset + line.trim_end_matches(['\r', '\n']).len(),
                start_line: line_index + 1,
                end_line: line_index + 1,
            };
            output.relations.push(relation(
                document,
                repo.clone(),
                revision.clone(),
                kind,
                owner.map(|symbol| symbol.id.clone()),
                owner.map_or(document.source_id.as_str(), |symbol| symbol.name.as_str()),
                target,
                target_name,
                span,
                0.9,
                "syntax",
                false,
            ));
        }
        line_offset += line.len();
    }
    output
}

fn code_value(document: &Document, key: &str) -> Option<String> {
    document
        .metadata
        .get("code")
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn declaration(
    line: &str,
    language: Language,
) -> Option<(String, SymbolKind, String, Option<String>)> {
    let normalized = line
        .trim_start_matches("export ")
        .trim_start_matches("async ");
    let patterns: &[(&str, SymbolKind)] = match language {
        Language::Rust => &[
            ("pub fn ", SymbolKind::Function),
            ("fn ", SymbolKind::Function),
            ("pub struct ", SymbolKind::Struct),
            ("struct ", SymbolKind::Struct),
            ("pub enum ", SymbolKind::Enum),
            ("enum ", SymbolKind::Enum),
            ("pub trait ", SymbolKind::Trait),
            ("trait ", SymbolKind::Trait),
            ("mod ", SymbolKind::Module),
            ("const ", SymbolKind::Constant),
        ],
        Language::Python => &[
            ("def ", SymbolKind::Function),
            ("class ", SymbolKind::Class),
        ],
        Language::TypeScript | Language::JavaScript => &[
            ("function ", SymbolKind::Function),
            ("class ", SymbolKind::Class),
            ("interface ", SymbolKind::Interface),
            ("type ", SymbolKind::Struct),
            ("const ", SymbolKind::Constant),
            ("let ", SymbolKind::Variable),
        ],
        Language::Go => &[
            ("func ", SymbolKind::Function),
            ("type ", SymbolKind::Struct),
            ("const ", SymbolKind::Constant),
            ("var ", SymbolKind::Variable),
        ],
        Language::Java | Language::Cpp | Language::Swift | Language::Ruby => &[
            ("class ", SymbolKind::Class),
            ("interface ", SymbolKind::Interface),
            ("struct ", SymbolKind::Struct),
            ("func ", SymbolKind::Function),
            ("def ", SymbolKind::Function),
        ],
        Language::Unknown => &[],
    };
    for (prefix, kind) in patterns {
        if let Some(rest) = normalized.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()?
                .to_string();
            if name.is_empty() {
                continue;
            }
            let visibility =
                (line.starts_with("pub ") || line.starts_with("export ")).then(|| "public".into());
            return Some((name, kind.clone(), line.trim().to_string(), visibility));
        }
    }
    None
}

fn import_target(line: &str, language: Language) -> Option<String> {
    let target = match language {
        Language::Rust => line
            .strip_prefix("use ")
            .or_else(|| line.strip_prefix("mod "))?,
        Language::Python => line
            .strip_prefix("import ")
            .or_else(|| line.strip_prefix("from "))?,
        Language::TypeScript | Language::JavaScript => line.strip_prefix("import ")?,
        Language::Go | Language::Java | Language::Cpp | Language::Swift | Language::Ruby => {
            line.strip_prefix("import ")?
        }
        Language::Unknown => return None,
    };
    target
        .trim_matches([';', '\'', '"'])
        .split([':', '.', '/', ' ', '{'])
        .find(|part| !part.is_empty())
        .map(str::to_string)
}

fn is_doc_comment(line: &str, language: Language) -> bool {
    line.starts_with("///")
        || line.starts_with("/**")
        || (language == Language::Python && line.starts_with("#"))
}

fn call_tokens(line: &str) -> BTreeSet<String> {
    let mut calls = BTreeSet::new();
    for (offset, character) in line.char_indices() {
        if character != '(' {
            continue;
        }
        let prefix = line[..offset].trim_end();
        let name = prefix
            .rsplit(|value: char| !(value.is_alphanumeric() || value == '_'))
            .next()
            .unwrap_or_default();
        if !name.is_empty()
            && !matches!(
                name,
                "if" | "for" | "while" | "match" | "fn" | "function" | "def"
            )
        {
            calls.insert(name.to_string());
        }
    }
    calls
}

fn inheritance_targets(line: &str, language: Language) -> Vec<(RelationKind, String)> {
    let mut targets = Vec::new();
    for (keyword, kind) in [
        (" extends ", RelationKind::Inheritance),
        (" implements ", RelationKind::Implementation),
    ] {
        if let Some((_, rest)) = line.split_once(keyword) {
            for target in rest
                .split([',', '{', ':'])
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let name = target.split_whitespace().next().unwrap_or_default();
                if !name.is_empty() {
                    targets.push((kind.clone(), name.to_string()));
                }
            }
        }
    }
    if language == Language::Rust && line.trim_start().starts_with("impl ") {
        let rest = line.trim_start().trim_start_matches("impl ");
        if let Some((interface, target)) = rest.split_once(" for ") {
            targets.push((RelationKind::Implementation, interface.trim().to_string()));
            let target = target
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_matches('{');
            if !target.is_empty() {
                targets.push((RelationKind::Reference, target.to_string()));
            }
        }
    }
    targets
}

#[allow(clippy::too_many_arguments)]
fn relation(
    document: &Document,
    repository_id: Option<String>,
    revision: Option<String>,
    kind: RelationKind,
    from_symbol_id: Option<String>,
    from_name: &str,
    to_symbol_id: Option<String>,
    to_name: String,
    span: SourceSpan,
    confidence: f32,
    origin: &str,
    dynamic: bool,
) -> CodeRelation {
    let resolved = to_symbol_id.is_some();
    let id = stable_hash(&format!(
        "{:?}:{from_name}:{to_name}:{}:{}",
        kind,
        span.start_byte,
        revision.as_deref().unwrap_or("none")
    ));
    CodeRelation {
        id,
        kind,
        from_symbol_id,
        from_name: from_name.into(),
        to_symbol_id,
        to_name,
        repository_id,
        revision,
        file: document.source_id.clone(),
        span,
        parser_version: BOUNDED_PARSER_VERSION.into(),
        confidence,
        origin: origin.into(),
        resolved,
        dynamic,
    }
}

fn symbol_id(
    repository: Option<&str>,
    revision: Option<&str>,
    file: &str,
    qualified: &str,
    kind: &SymbolKind,
    span: &SourceSpan,
) -> String {
    stable_hash(&format!(
        "{}:{}:{file}:{qualified}:{kind:?}:{}",
        repository.unwrap_or("local"),
        revision.unwrap_or("unknown"),
        span.start_byte
    ))
}

fn stable_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn document(path: &str, content: &str) -> Document {
        Document {
            source: "code".into(),
            source_id: path.into(),
            title: path.into(),
            content: content.into(),
            uri: None,
            updated_at: Utc::now(),
            project: "test".into(),
            acl: vec!["work".into()],
            metadata: json!({"code": {"repository_id": "repo-1", "revision": "abc:main:committed"}}),
        }
    }

    #[test]
    fn parser_emits_exact_revision_aware_symbol_spans() {
        let source = "/// Greets somebody.\npub fn greet(name: &str) {\n    println!(\"hello {name}\");\n}\n";
        let result = parse_document(
            &document("src/lib.rs", source),
            &ParseLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(result.status, ParseStatus::Complete);
        let symbol = result
            .symbols
            .iter()
            .find(|value| value.name == "greet")
            .expect("greet symbol");
        assert_eq!(
            &source[symbol.span.start_byte..symbol.span.end_byte],
            "pub fn greet(name: &str) {"
        );
        assert_eq!(symbol.revision.as_deref(), Some("abc:main:committed"));
        assert_eq!(symbol.documentation.as_deref(), Some("Greets somebody."));
    }

    #[test]
    fn unsupported_oversized_and_cancelled_inputs_fail_safe() {
        let unknown = parse_document(
            &document("asset.bin", "value"),
            &ParseLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(unknown.status, ParseStatus::Unsupported);
        let oversized = parse_document(
            &document("lib.rs", "12345"),
            &ParseLimits {
                max_bytes: 4,
                ..ParseLimits::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(oversized.status, ParseStatus::Oversized);
        let cancelled = parse_document(
            &document("lib.rs", "fn value() {}"),
            &ParseLimits::default(),
            &AtomicBool::new(true),
        );
        assert_eq!(cancelled.status, ParseStatus::Cancelled);
    }

    #[test]
    fn remote_sanitization_removes_credentials_and_local_paths() {
        assert_eq!(
            sanitize_remote("https://token@example.com/org/repo.git?secret=1"),
            Some("https://example.com/org/repo".into())
        );
        assert_eq!(
            sanitize_remote("git@example.com:org/repo.git"),
            Some("https://example.com/org/repo".into())
        );
        assert_eq!(sanitize_remote("/Users/person/private/repo"), None);
    }

    #[test]
    fn revision_changes_derived_identity_and_cache_key() {
        let first = document("src/lib.rs", "pub fn greet() {}");
        let mut second = first.clone();
        second.metadata["code"]["revision"] = json!("def:feature:dirty");
        let cancel = AtomicBool::new(false);
        let first = parse_document(&first, &ParseLimits::default(), &cancel);
        let second = parse_document(&second, &ParseLimits::default(), &cancel);
        assert_ne!(first.symbols[0].id, second.symbols[0].id);
    }

    #[test]
    fn relations_mark_duplicate_targets_ambiguous_and_capture_implementation() {
        let source = "trait Runner {}\nstruct Job {}\nimpl Runner for Job {}\nfn run() {}\nfn run() {}\nfn caller() { run(); }\n";
        let result = parse_document(
            &document("src/lib.rs", source),
            &ParseLimits::default(),
            &AtomicBool::new(false),
        );
        let ambiguous = result
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::Call && relation.to_name == "run")
            .expect("ambiguous call");
        assert!(!ambiguous.resolved);
        assert_eq!(ambiguous.origin, "ambiguous");
        assert!(result.relations.iter().any(|relation| {
            relation.kind == RelationKind::Implementation && relation.to_name == "Runner"
        }));
    }

    #[test]
    fn symbol_budget_returns_a_partial_rebuildable_projection() {
        let result = parse_document(
            &document("src/lib.rs", "fn one() {}\nfn two() {}\n"),
            &ParseLimits {
                max_symbols: 1,
                ..ParseLimits::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(result.status, ParseStatus::Partial);
        assert_eq!(result.symbols.len(), 1);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|value| value.contains("symbol budget"))
        );
        let relation_limited = parse_document(
            &document("src/lib.rs", "fn one() { two(); }\nfn two() {}\n"),
            &ParseLimits {
                max_relations: 0,
                ..ParseLimits::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(relation_limited.status, ParseStatus::Partial);
        assert!(relation_limited.relations.is_empty());
    }

    #[test]
    fn repository_identity_tracks_branch_detached_and_dirty_without_leaking_paths() {
        let directory = tempdir().expect("repository");
        let run = |arguments: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(arguments)
                .status()
                .expect("git command");
            assert!(status.success(), "git {arguments:?}");
        };
        run(&["init", "-b", "main"]);
        std::fs::write(directory.path().join("lib.rs"), "fn one() {}\n").expect("source");
        run(&["add", "lib.rs"]);
        run(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture",
        ]);
        run(&[
            "remote",
            "add",
            "origin",
            "https://token@example.com/org/repo.git",
        ]);
        let clean = inspect_repository(directory.path()).expect("clean identity");
        assert_eq!(clean.branch.as_deref(), Some("main"));
        assert!(!clean.dirty);
        assert_eq!(
            clean.canonical_remote.as_deref(),
            Some("https://example.com/org/repo")
        );
        assert!(
            !serde_json::to_string(&clean)
                .expect("identity JSON")
                .contains(directory.path().to_string_lossy().as_ref())
        );

        std::fs::write(directory.path().join("lib.rs"), "fn changed() {}\n").expect("change");
        assert!(
            inspect_repository(directory.path())
                .expect("dirty identity")
                .dirty
        );
        run(&["checkout", "--detach"]);
        let detached = inspect_repository(directory.path()).expect("detached identity");
        assert!(detached.detached);
        assert_eq!(detached.branch, None);
    }

    #[test]
    fn plain_directories_receive_distinct_explicit_no_git_identities() {
        let first = tempdir().expect("first root");
        let second = tempdir().expect("second root");
        let first = inspect_repository(first.path()).expect("first identity");
        let second = inspect_repository(second.path()).expect("second identity");
        assert!(!first.git_available);
        assert_ne!(first.repository_id, second.repository_id);
        assert_eq!(first.commit_sha, None);
    }

    #[test]
    fn generated_vendor_and_worktree_paths_are_classified_without_becoming_authority() {
        assert_eq!(
            is_generated_or_vendor(Path::new("src/lib.rs")),
            (false, false)
        );
        assert_eq!(
            is_generated_or_vendor(Path::new("dist/app.js")),
            (true, false)
        );
        assert_eq!(
            is_generated_or_vendor(Path::new("vendor/lib.rs")),
            (false, true)
        );
        assert_eq!(
            is_generated_or_vendor(Path::new("node_modules/pkg/index.js")),
            (false, true)
        );
        assert_eq!(
            is_generated_or_vendor(Path::new(".worktrees/topic/src/lib.rs")),
            (true, false)
        );
    }
}
