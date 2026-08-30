//! Transport-safe integration identity contracts.
//!
//! These types intentionally contain only opaque references and scoped public
//! metadata. Credentials, endpoints, database paths, and consumer-owned state
//! never enter the mapping or principal records.

use anyhow::{Result, anyhow, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::stable_json_digest;

pub const INTEGRATION_CONTRACT_VERSION: &str = "cortana.integration.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingStatus {
    PendingApproval,
    Active,
    Revoked,
    Orphaned,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalWorkspaceMapping {
    pub contract_version: String,
    pub mapping_id: String,
    pub consumer_id: String,
    pub external_workspace_id: String,
    pub cortana_project_id: String,
    pub capability_ref: String,
    pub permitted_acl: Vec<String>,
    pub status: MappingStatus,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
}

impl ExternalWorkspaceMapping {
    pub fn new(
        consumer_id: &str,
        external_workspace_id: &str,
        cortana_project_id: &str,
        capability_ref: &str,
        permitted_acl: Vec<String>,
    ) -> Result<Self> {
        validate_opaque_id("consumer_id", consumer_id)?;
        validate_opaque_id("external_workspace_id", external_workspace_id)?;
        validate_opaque_id("cortana_project_id", cortana_project_id)?;
        validate_opaque_id("capability_ref", capability_ref)?;
        let permitted_acl = normalize_acl(permitted_acl)?;
        ensure!(
            permitted_acl
                .iter()
                .any(|label| label == cortana_project_id),
            "mapping ACL must include its Cortana project"
        );
        let mapping_id = format!(
            "map_{}",
            &stable_json_digest(&serde_json::json!({
                "consumer_id": consumer_id,
                "external_workspace_id": external_workspace_id,
                "cortana_project_id": cortana_project_id,
            }))[..24]
        );
        let now = now();
        Ok(Self {
            contract_version: INTEGRATION_CONTRACT_VERSION.into(),
            mapping_id,
            consumer_id: consumer_id.into(),
            external_workspace_id: external_workspace_id.into(),
            cortana_project_id: cortana_project_id.into(),
            capability_ref: capability_ref.into(),
            permitted_acl,
            status: MappingStatus::PendingApproval,
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
            approved_by: None,
            revoked_by: None,
        })
    }

    pub fn approve(&mut self, approving_principal: &str) -> Result<()> {
        ensure!(
            self.status == MappingStatus::PendingApproval,
            "only a pending mapping can be approved"
        );
        validate_opaque_id("approving_principal", approving_principal)?;
        self.status = MappingStatus::Active;
        self.approved_by = Some(approving_principal.into());
        self.advance_revision();
        Ok(())
    }

    pub fn revoke(&mut self, revoking_principal: &str) -> Result<()> {
        ensure!(
            matches!(
                self.status,
                MappingStatus::PendingApproval | MappingStatus::Active
            ),
            "mapping is already inactive"
        );
        validate_opaque_id("revoking_principal", revoking_principal)?;
        self.status = MappingStatus::Revoked;
        self.revoked_by = Some(revoking_principal.into());
        self.advance_revision();
        Ok(())
    }

    pub fn mark_orphaned(&mut self) -> Result<()> {
        ensure!(
            self.status == MappingStatus::Active,
            "only an active mapping can become orphaned"
        );
        self.status = MappingStatus::Orphaned;
        self.advance_revision();
        Ok(())
    }

    pub fn reconnect(&mut self, capability_ref: &str) -> Result<()> {
        ensure!(
            self.status == MappingStatus::Orphaned,
            "only an orphaned mapping can reconnect"
        );
        validate_opaque_id("capability_ref", capability_ref)?;
        self.capability_ref = capability_ref.into();
        self.status = MappingStatus::PendingApproval;
        self.approved_by = None;
        self.advance_revision();
        Ok(())
    }

    fn advance_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = now();
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalRole {
    QueryOnly,
    QueryAndMemory,
    StatusOnly,
    IntegrationAdmin,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalStatus {
    Active,
    Revoked,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationPrincipal {
    pub contract_version: String,
    pub principal_id: String,
    pub role: PrincipalRole,
    pub mapping_ref: String,
    pub credential_ref: String,
    pub scopes: Vec<String>,
    pub acl: Vec<String>,
    pub status: PrincipalStatus,
    pub revision: u64,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
}

impl IntegrationPrincipal {
    pub fn new(
        principal_id: &str,
        role: PrincipalRole,
        mapping_ref: &str,
        acl: Vec<String>,
        expires_at: Option<&str>,
    ) -> Result<Self> {
        validate_opaque_id("principal_id", principal_id)?;
        validate_opaque_id("mapping_ref", mapping_ref)?;
        let acl = normalize_acl(acl)?;
        ensure!(
            !acl.is_empty(),
            "integration principal ACL must not be empty"
        );
        let expires_at = expires_at
            .map(parse_time)
            .transpose()?
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true));
        Ok(Self {
            contract_version: INTEGRATION_CONTRACT_VERSION.into(),
            principal_id: principal_id.into(),
            role,
            mapping_ref: mapping_ref.into(),
            credential_ref: format!("credential_ref_{principal_id}"),
            scopes: role.scopes(),
            acl,
            status: PrincipalStatus::Active,
            revision: 1,
            created_at: now(),
            expires_at,
            revoked_by: None,
        })
    }

    pub fn can_read_memory(&self) -> bool {
        self.scopes.iter().any(|scope| scope == "memory")
    }

    pub fn can_write_memory(&self) -> bool {
        self.status == PrincipalStatus::Active && self.can_read_memory()
    }

    pub fn is_expired_at(&self, at: &str) -> Result<bool> {
        let at = parse_time(at)?;
        Ok(self
            .expires_at
            .as_deref()
            .map(parse_time)
            .transpose()?
            .is_some_and(|expiry| at >= expiry))
    }

    pub fn is_active_at(&self, at: &str) -> Result<bool> {
        Ok(self.status == PrincipalStatus::Active && !self.is_expired_at(at)?)
    }

    pub fn revoke(&mut self, revoking_principal: &str) -> Result<()> {
        ensure!(
            self.status == PrincipalStatus::Active,
            "principal is already inactive"
        );
        validate_opaque_id("revoking_principal", revoking_principal)?;
        self.status = PrincipalStatus::Revoked;
        self.revoked_by = Some(revoking_principal.into());
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

impl PrincipalRole {
    fn scopes(self) -> Vec<String> {
        match self {
            Self::QueryOnly => vec!["query".into(), "status".into()],
            Self::QueryAndMemory => vec!["query".into(), "status".into(), "memory".into()],
            Self::StatusOnly => vec!["status".into()],
            Self::IntegrationAdmin => vec![
                "query".into(),
                "status".into(),
                "memory".into(),
                "admin".into(),
            ],
        }
    }
}

fn normalize_acl(mut acl: Vec<String>) -> Result<Vec<String>> {
    for label in &acl {
        validate_opaque_id("acl label", label)?;
        ensure!(label != "*", "reserved wildcard ACL is not permitted");
    }
    acl.sort();
    acl.dedup();
    Ok(acl)
}

fn validate_opaque_id(name: &str, value: &str) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{name} must not be empty");
    ensure!(value.len() <= 256, "{name} exceeds 256 bytes");
    ensure!(
        !value.contains('/')
            && !value.contains('\\')
            && !value.contains("://")
            && !value.to_ascii_lowercase().starts_with("bearer "),
        "{name} must be an opaque identifier, not a path, endpoint, or credential"
    );
    ensure!(
        value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        }),
        "{name} contains unsupported characters"
    );
    Ok(())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| anyhow!("timestamp must be RFC3339"))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
