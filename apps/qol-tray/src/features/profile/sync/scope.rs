use crate::features::auth::{GitHubScope, Scope, ScopeRequirement};

pub(crate) const SCOPE_REQUIREMENTS: &[ScopeRequirement] = &[ScopeRequirement {
    scope: Scope::GitHub(GitHubScope::Repo),
    reason: "Profile sync writes to a private GitHub repo on your account.",
}];
