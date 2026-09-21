use anyhow::{anyhow, Result};

use super::registry::{required_wire_names, requirements_for};
use super::types::{AuthHealth, AuthHealthIssue, AuthProvider, MissingScope, Scope};

pub(crate) fn auth_health() -> AuthHealth {
    AuthHealth {
        issues: vec![AuthProvider::GitHub]
            .into_iter()
            .filter_map(github_issue)
            .collect(),
    }
}

pub(crate) fn ensure_scope(scope: Scope) -> Result<()> {
    let provider = scope.provider();
    let granted = granted_wire_names(provider);
    let wire = scope.wire_name();
    if granted.iter().any(|s| s == wire) {
        return Ok(());
    }
    Err(anyhow!(
        "GitHub credential is missing required scope `{wire}`. \
         Open the Profile page and use Reauthorize to grant it."
    ))
}

pub(crate) fn cumulative_scopes_for(provider: AuthProvider) -> Vec<String> {
    let required = required_wire_names(provider);
    let granted = granted_wire_names(provider);
    let mut union: Vec<String> = required
        .iter()
        .map(|s| (*s).to_string())
        .chain(granted)
        .collect();
    union.sort();
    union.dedup();
    union
}

fn github_issue(provider: AuthProvider) -> Option<AuthHealthIssue> {
    let reqs = requirements_for(provider);
    if reqs.is_empty() {
        return None;
    }
    if !provider_connected(provider) {
        return None;
    }
    let granted = granted_wire_names(provider);
    let missing: Vec<MissingScope> = reqs
        .into_iter()
        .filter(|req| !granted.iter().any(|g| g == req.scope.wire_name()))
        .map(|req| MissingScope {
            scope: req.scope,
            wire_name: req.scope.wire_name(),
            reason: req.reason,
        })
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(AuthHealthIssue::InsufficientScope { provider, missing })
}

fn provider_connected(provider: AuthProvider) -> bool {
    match provider {
        AuthProvider::GitHub => crate::features::github_auth::oauth_access_token().is_some(),
    }
}

fn granted_wire_names(provider: AuthProvider) -> Vec<String> {
    match provider {
        AuthProvider::GitHub => {
            if !provider_connected(provider) {
                return Vec::new();
            }
            crate::features::github_auth::oauth_scopes()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_scopes_include_required_wire_names() {
        let scopes = cumulative_scopes_for(AuthProvider::GitHub);
        assert!(scopes.iter().any(|s| s == "repo"));
    }

    #[test]
    fn cumulative_scopes_are_sorted_and_unique() {
        let scopes = cumulative_scopes_for(AuthProvider::GitHub);
        let mut expected = scopes.clone();
        expected.sort();
        expected.dedup();
        assert_eq!(scopes, expected);
    }

    #[test]
    fn auth_provider_wire_name_is_github() {
        let json = serde_json::to_string(&AuthProvider::GitHub).unwrap();
        assert_eq!(json, "\"github\"");
        let parsed: AuthProvider = serde_json::from_str("\"github\"").unwrap();
        assert_eq!(parsed, AuthProvider::GitHub);
    }
}
