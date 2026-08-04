use std::collections::HashSet;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::config::Config;

pub const QUERY_SCOPE: &str = "query";
pub const STATUS_SCOPE: &str = "status";
pub const ADMIN_SCOPE: &str = "admin";

#[derive(Clone, Debug)]
pub struct Principal {
    pub name: String,
    scopes: HashSet<String>,
    acl: HashSet<String>,
}

impl Principal {
    pub fn local(name: &str) -> Self {
        Self {
            name: name.into(),
            scopes: [QUERY_SCOPE, STATUS_SCOPE, ADMIN_SCOPE]
                .into_iter()
                .map(str::to_string)
                .collect(),
            acl: ["*".to_string()].into_iter().collect(),
        }
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    /// Admin credentials can inspect the complete local brain, including
    /// configured sources outside their named labels.
    pub fn is_owner(&self) -> bool {
        self.has_scope(ADMIN_SCOPE)
    }

    /// ACL labels that govern scoped read access.
    pub fn acl_labels(&self) -> Vec<String> {
        let mut labels = self.acl.iter().cloned().collect::<Vec<_>>();
        labels.sort();
        labels
    }

    /// Effective ACL used for scoped read paths.
    pub fn visible_acl(&self) -> Vec<String> {
        if self.is_owner() {
            vec!["*".to_string()]
        } else {
            self.acl_labels()
        }
    }
}

#[derive(Clone)]
struct Credential {
    digest: [u8; 32],
    principal: Principal,
}

#[derive(Clone)]
pub struct AuthPolicy {
    credentials: Vec<Credential>,
}

impl AuthPolicy {
    pub fn legacy(token: Option<String>) -> Self {
        let credentials = token
            .map(|value| Credential {
                digest: Sha256::digest(value.as_bytes()).into(),
                principal: Principal::local("legacy-api-token"),
            })
            .into_iter()
            .collect();
        Self { credentials }
    }

    pub fn from_config(config: &Config, legacy_token: Option<String>) -> Result<Self> {
        let mut credentials = Vec::new();
        let mut principals = HashSet::new();
        for token in &config.auth.tokens {
            anyhow::ensure!(
                !token.principal.trim().is_empty(),
                "auth token principal must not be empty"
            );
            anyhow::ensure!(
                principals.insert(token.principal.clone()),
                "duplicate auth principal {}",
                token.principal
            );
            let scopes = token.scopes.iter().cloned().collect::<HashSet<_>>();
            anyhow::ensure!(
                !scopes.is_empty()
                    && scopes.iter().all(|scope| matches!(
                        scope.as_str(),
                        QUERY_SCOPE | STATUS_SCOPE | ADMIN_SCOPE
                    )),
                "auth principal {} has an invalid scope",
                token.principal
            );
            let acl = token.acl.iter().cloned().collect::<HashSet<_>>();
            anyhow::ensure!(
                !acl.contains("*"),
                "auth principal {} cannot use reserved acl \"*\"",
                token.principal
            );
            let value = config
                .environment_value(&token.token_env)
                .with_context(|| {
                    format!(
                        "auth token environment variable {} is not set",
                        token.token_env
                    )
                })?;
            anyhow::ensure!(!value.is_empty(), "auth bearer token must not be empty");
            credentials.push(Credential {
                digest: Sha256::digest(value.as_bytes()).into(),
                principal: Principal {
                    name: token.principal.clone(),
                    scopes,
                    acl,
                },
            });
        }
        if let Some(value) = legacy_token {
            anyhow::ensure!(!value.is_empty(), "auth bearer token must not be empty");
            credentials.push(Credential {
                digest: Sha256::digest(value.as_bytes()).into(),
                principal: Principal::local("legacy-api-token"),
            });
        }
        let mut digests = HashSet::new();
        anyhow::ensure!(
            credentials
                .iter()
                .all(|credential| digests.insert(credential.digest)),
            "auth bearer token values must be unique"
        );
        Ok(Self { credentials })
    }

    pub fn requires_token(&self) -> bool {
        !self.credentials.is_empty()
    }

    pub fn authenticate(&self, token: &str) -> Option<Principal> {
        let provided: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.credentials
            .iter()
            .find(|credential| constant_time_eq(&provided, &credential.digest))
            .map(|credential| credential.principal.clone())
    }
}

pub fn acl_allows(document_acl: &[String], principal_acl: &[String]) -> bool {
    document_acl.is_empty()
        || principal_acl.iter().any(|label| label == "*")
        || document_acl
            .iter()
            .any(|required| principal_acl.iter().any(|label| label == required))
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthTokenConfig;

    #[test]
    fn acl_requires_an_intersection_unless_document_is_public_or_principal_is_owner() {
        assert!(acl_allows(&[], &[]));
        assert!(acl_allows(&["work".into()], &["*".into()]));
        assert!(acl_allows(&["work".into()], &["work".into()]));
        assert!(!acl_allows(&["personal".into()], &["work".into()]));
    }

    #[test]
    fn configured_tokens_require_valid_unique_principals_scopes_and_values() {
        let mut config = Config::default();
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "work-agent".into(),
            token_env: "WORK_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into()],
            acl: vec!["work".into()],
        }];

        let policy = AuthPolicy::from_config(&config, None).expect("valid policy");
        let principal = policy.authenticate("work-secret").expect("principal");
        assert_eq!(principal.name, "work-agent");
        assert!(principal.has_scope(QUERY_SCOPE));
        assert!(!principal.has_scope(ADMIN_SCOPE));
        assert!(!principal.is_owner());
        assert_eq!(principal.acl_labels(), vec!["work"]);
        assert!(policy.authenticate("wrong-secret").is_none());

        config.auth.tokens[0].scopes = vec!["unknown".into()];
        assert!(AuthPolicy::from_config(&config, None).is_err());

        config.auth.tokens[0].scopes = vec![QUERY_SCOPE.into()];
        config.auth.tokens.push(config.auth.tokens[0].clone());
        assert!(AuthPolicy::from_config(&config, None).is_err());

        config.auth.tokens.pop();
        config.auth.tokens[0].scopes = vec![ADMIN_SCOPE.into()];
        let owner = AuthPolicy::from_config(&config, None)
            .expect("admin policy")
            .authenticate("work-secret")
            .expect("admin principal");
        assert!(owner.is_owner());

        let wildcard_named = Principal {
            name: "wildcard-agent".into(),
            scopes: vec![QUERY_SCOPE.to_string()].into_iter().collect(),
            acl: vec!["*".to_string()].into_iter().collect(),
        };
        assert!(!wildcard_named.is_owner());
        assert_eq!(wildcard_named.visible_acl(), vec!["*".to_string()]);
    }

    #[test]
    fn configured_token_acl_rejects_reserved_wildcard_label() {
        let mut config = Config::default();
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "work-agent".into(),
            token_env: "WORK_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into()],
            acl: vec!["*".into()],
        }];

        assert!(AuthPolicy::from_config(&config, None).is_err());

        let principal = AuthPolicy::legacy(Some("legacy-secret".into()))
            .authenticate("legacy-secret")
            .expect("legacy principal");
        assert_eq!(principal.acl_labels(), vec!["*".to_string()]);
    }

    #[test]
    fn bearer_values_must_be_unique_even_across_legacy_and_named_tokens() {
        let mut config = Config::default();
        config
            .environment
            .insert("WORK_TOKEN".into(), "same-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "work-agent".into(),
            token_env: "WORK_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into()],
            acl: vec!["work".into()],
        }];

        assert!(AuthPolicy::from_config(&config, Some("same-secret".into())).is_err());
    }
}
