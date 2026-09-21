use super::types::{AuthProvider, ScopeRequirement};

pub(super) fn all_requirements() -> Vec<&'static ScopeRequirement> {
    let mut out = Vec::new();
    out.extend(crate::features::profile::sync::SCOPE_REQUIREMENTS);
    out
}

pub(super) fn requirements_for(provider: AuthProvider) -> Vec<&'static ScopeRequirement> {
    all_requirements()
        .into_iter()
        .filter(|req| req.scope.provider() == provider)
        .collect()
}

pub(super) fn required_wire_names(provider: AuthProvider) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = requirements_for(provider)
        .into_iter()
        .map(|req| req.scope.wire_name())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::auth::types::{GitHubScope, Scope};

    #[test]
    fn profile_sync_requires_github_repo() {
        let reqs = requirements_for(AuthProvider::GitHub);
        assert!(reqs
            .iter()
            .any(|r| matches!(r.scope, Scope::GitHub(GitHubScope::Repo))));
    }

    #[test]
    fn required_wire_names_sorted_and_unique() {
        let names = required_wire_names(AuthProvider::GitHub);
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
    }
}
