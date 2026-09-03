//! Versioned, provenance-bearing contract shared by every knowledge-graph surface.
//!
//! Graph records are derived projections. Canonical documents and native memories
//! remain authoritative and graph traversal never expands a principal's visibility.

use std::{collections::BTreeSet, fmt};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

const MAX_NODE_KEY_BYTES: usize = 512;
const MAX_SUPPORT_RECORDS: usize = 64;
const MAX_INVALIDATION_KEYS: usize = 64;

pub struct GraphContract;

impl GraphContract {
    pub const VERSION: &'static str = "cortana.knowledge-graph.v1";
    pub const DEFAULT_DERIVATION_VERSION: &'static str = "cortana.graph-derivation.v1";
    pub const DEFAULT_PAGE_SIZE: usize = 50;
    pub const MAX_PAGE_SIZE: usize = 100;
    pub const MAX_DEPTH: usize = 3;
    pub const MAX_NODES_PER_EXPANSION: usize = 200;
    pub const MAX_EDGES_PER_EXPANSION: usize = 400;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Workspace,
    Source,
    Document,
    Chunk,
    Entity,
    Memory,
    Observation,
    MentalModel,
    Repository,
    File,
    Symbol,
}

impl NodeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Source => "source",
            Self::Document => "document",
            Self::Chunk => "chunk",
            Self::Entity => "entity",
            Self::Memory => "memory",
            Self::Observation => "observation",
            Self::MentalModel => "mental-model",
            Self::Repository => "repository",
            Self::File => "file",
            Self::Symbol => "symbol",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct GraphNodeId(String);

impl GraphNodeId {
    pub fn new(kind: NodeKind, stable_key: impl AsRef<str>) -> Result<Self> {
        let stable_key = stable_key.as_ref();
        if stable_key.is_empty() || stable_key.len() > MAX_NODE_KEY_BYTES {
            bail!("graph node keys must contain between 1 and {MAX_NODE_KEY_BYTES} bytes");
        }
        if stable_key.chars().any(char::is_control) {
            bail!("graph node keys cannot contain control characters");
        }
        Ok(Self(format!(
            "{}:{}",
            kind.as_str(),
            percent_encode(stable_key.as_bytes())
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GraphNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    Contains,
    References,
    Backlink,
    Nearby,
    SameThread,
    AuthoredBy,
    Mentions,
    Temporal,
    SemanticallyRelated,
    Supports,
    Contradicts,
    Reinforces,
    Supersedes,
    Observes,
    Derives,
    DependsOn,
    Defines,
    Calls,
    Imports,
}

impl EdgeKind {
    pub const ALL: [Self; 19] = [
        Self::Contains,
        Self::References,
        Self::Backlink,
        Self::Nearby,
        Self::SameThread,
        Self::AuthoredBy,
        Self::Mentions,
        Self::Temporal,
        Self::SemanticallyRelated,
        Self::Supports,
        Self::Contradicts,
        Self::Reinforces,
        Self::Supersedes,
        Self::Observes,
        Self::Derives,
        Self::DependsOn,
        Self::Defines,
        Self::Calls,
        Self::Imports,
    ];

    pub const fn is_directional(self) -> bool {
        !matches!(
            self,
            Self::Nearby | Self::SemanticallyRelated | Self::Reinforces
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeOrigin {
    Explicit,
    Derived,
    Inferred,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationshipSupport {
    pub record_ids: Vec<String>,
    pub invalidation_keys: Vec<String>,
}

impl RelationshipSupport {
    fn normalize(&mut self) {
        normalize_bounded(&mut self.record_ids, MAX_SUPPORT_RECORDS);
        normalize_bounded(&mut self.invalidation_keys, MAX_INVALIDATION_KEYS);
    }

    fn validate(&self) -> Result<()> {
        if self.record_ids.is_empty() {
            bail!("graph relationships require at least one supporting record id");
        }
        if self.invalidation_keys.is_empty() {
            bail!("graph relationships require at least one invalidation key");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphEdge {
    pub contract_version: &'static str,
    pub source: GraphNodeId,
    pub target: GraphNodeId,
    pub kind: EdgeKind,
    pub origin: EdgeOrigin,
    pub derivation_version: &'static str,
    pub confidence: Option<f32>,
    pub citation_authority: bool,
    pub updated_at: String,
    pub project: String,
    pub acl: Vec<String>,
    pub support: RelationshipSupport,
}

impl GraphEdge {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: GraphNodeId,
        target: GraphNodeId,
        kind: EdgeKind,
        origin: EdgeOrigin,
        updated_at: impl Into<String>,
        project: impl Into<String>,
        mut acl: Vec<String>,
        mut support: RelationshipSupport,
    ) -> Result<Self> {
        let project = project.into();
        if project.trim().is_empty() {
            bail!("graph relationships require a workspace scope");
        }
        normalize_bounded(&mut acl, 64);
        support.normalize();
        Ok(Self {
            contract_version: GraphContract::VERSION,
            source,
            target,
            kind,
            origin,
            derivation_version: GraphContract::DEFAULT_DERIVATION_VERSION,
            confidence: None,
            citation_authority: matches!(origin, EdgeOrigin::Explicit),
            updated_at: updated_at.into(),
            project,
            acl,
            support,
        })
    }

    pub fn with_confidence(mut self, confidence: f32) -> Result<Self> {
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            bail!("graph confidence must be between zero and one");
        }
        self.confidence = Some(confidence);
        Ok(self)
    }

    pub fn with_citation_authority(mut self, citation_authority: bool) -> Self {
        self.citation_authority = citation_authority;
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.contract_version != GraphContract::VERSION {
            bail!("unsupported graph contract version");
        }
        if self.source == self.target {
            bail!("graph self-edges are not supported");
        }
        if self.updated_at.trim().is_empty() {
            bail!("graph relationships require an update time");
        }
        self.support.validate()?;
        match self.origin {
            EdgeOrigin::Inferred if self.confidence.is_none() => {
                bail!("inferred graph relationships require confidence")
            }
            EdgeOrigin::Explicit if self.confidence.is_some() => {
                bail!("explicit graph relationships cannot claim inferred confidence")
            }
            _ => {}
        }
        Ok(())
    }

    pub fn explanation(&self) -> String {
        format!(
            "{} relationship from {} to {}, {} by {}, supported by {}",
            format!("{:?}", self.kind).to_ascii_lowercase(),
            self.source,
            self.target,
            match self.origin {
                EdgeOrigin::Explicit => "declared explicitly",
                EdgeOrigin::Derived => "derived deterministically",
                EdgeOrigin::Inferred => "inferred and qualified",
            },
            self.derivation_version,
            self.support.record_ids.join(", ")
        )
    }

    pub fn deduplication_key(&self) -> (&GraphNodeId, &GraphNodeId, EdgeKind, EdgeOrigin) {
        (&self.source, &self.target, self.kind, self.origin)
    }
}

fn normalize_bounded(values: &mut Vec<String>, maximum: usize) {
    let normalized = values
        .drain(..)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    *values = normalized.into_iter().take(maximum).collect();
}

fn percent_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            use std::fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    encoded
}

pub fn require_acl_intersection(left: &[String], right: &[String]) -> Result<Vec<String>> {
    let left = left.iter().cloned().collect::<BTreeSet<_>>();
    let right = right.iter().cloned().collect::<BTreeSet<_>>();
    if left.is_empty() {
        return Ok(right.into_iter().collect());
    }
    if right.is_empty() {
        return Ok(left.into_iter().collect());
    }
    let intersection = left.intersection(&right).cloned().collect::<Vec<_>>();
    if intersection.is_empty() {
        return Err(anyhow!(
            "graph relationship support has no shared ACL visibility"
        ));
    }
    Ok(intersection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acl_intersection_never_unions_visibility() {
        assert_eq!(
            require_acl_intersection(&["work".into(), "team".into()], &["team".into()])
                .expect("intersection"),
            vec!["team"]
        );
        assert!(require_acl_intersection(&["work".into()], &["personal".into()]).is_err());
        assert_eq!(
            require_acl_intersection(&[], &["work".into()]).expect("public plus scoped"),
            vec!["work"]
        );
    }
}
