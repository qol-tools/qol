use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

pub(super) const EVIDENCE_BASIS: &str = "configuration_declared";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum AgentRole {
    Scout,
    Implement,
    Architect,
    Review,
    Debug,
}

impl AgentRole {
    pub(super) fn token(self) -> &'static str {
        match self {
            AgentRole::Scout => "scout",
            AgentRole::Implement => "implement",
            AgentRole::Architect => "architect",
            AgentRole::Review => "review",
            AgentRole::Debug => "debug",
        }
    }

    pub(super) fn from_token(token: &str) -> Option<Self> {
        match token {
            "scout" => Some(AgentRole::Scout),
            "implement" => Some(AgentRole::Implement),
            "architect" => Some(AgentRole::Architect),
            "review" => Some(AgentRole::Review),
            "debug" => Some(AgentRole::Debug),
            _ => None,
        }
    }

    pub(super) fn all() -> [Self; 5] {
        [
            AgentRole::Scout,
            AgentRole::Implement,
            AgentRole::Architect,
            AgentRole::Review,
            AgentRole::Debug,
        ]
    }
}

pub(super) fn role_catalog() -> String {
    AgentRole::all()
        .iter()
        .map(|role| role.token())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn role_tokens(roles: &[AgentRole]) -> String {
    if roles.is_empty() {
        return "no roles".to_owned();
    }
    roles
        .iter()
        .map(|role| role.token())
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ImageInput {
    None,
    Native,
    #[default]
    Unknown,
}

impl ImageInput {
    pub(super) fn token(self) -> &'static str {
        match self {
            ImageInput::None => "none",
            ImageInput::Native => "native",
            ImageInput::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum VisualReview {
    #[default]
    Deny,
    Allow,
}

impl VisualReview {
    pub(super) fn token(self) -> &'static str {
        match self {
            VisualReview::Deny => "deny",
            VisualReview::Allow => "allow",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgentRequirement {
    ImageInput,
    VisualReview,
}

impl AgentRequirement {
    pub(super) fn token(self) -> &'static str {
        match self {
            AgentRequirement::ImageInput => "image_input",
            AgentRequirement::VisualReview => "visual_review",
        }
    }

    pub(super) fn from_token(token: &str) -> Option<Self> {
        match token {
            "image_input" => Some(AgentRequirement::ImageInput),
            "visual_review" => Some(AgentRequirement::VisualReview),
            _ => None,
        }
    }
}

pub(super) fn parse_requires(token: &str) -> Result<Vec<AgentRequirement>> {
    if token.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut requires = Vec::new();
    for entry in token.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            bail!(
                "--requires takes a comma-separated list drawn from image_input and visual_review"
            );
        }
        let requirement = AgentRequirement::from_token(entry).ok_or_else(|| {
            anyhow!("unknown requirement `{entry}`; expected image_input or visual_review")
        })?;
        if requires.contains(&requirement) {
            bail!("requirement `{entry}` is listed twice");
        }
        requires.push(requirement);
    }
    Ok(requires)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum AgentStatus {
    Constrained,
    Unconstrained,
}

impl AgentStatus {
    pub(super) fn of(assignment: Option<&AgentAssignment>) -> Self {
        match assignment {
            Some(_) => AgentStatus::Constrained,
            None => AgentStatus::Unconstrained,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentProfileSpec {
    pub(super) tool: String,
    pub(super) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) provider: Option<String>,
    pub(super) roles: Vec<AgentRole>,
    #[serde(default)]
    pub(super) image_input: ImageInput,
    #[serde(default)]
    pub(super) visual_review: VisualReview,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) preference: Option<i64>,
}

impl AgentProfileSpec {
    fn validate(&self, name: &str) -> Result<()> {
        if self.tool.trim().is_empty() {
            bail!("agent profile `{name}` must declare a non-empty tool");
        }
        if self.model.trim().is_empty() {
            bail!("agent profile `{name}` must declare a non-empty model");
        }
        if self
            .provider
            .as_deref()
            .is_some_and(|provider| provider.trim().is_empty())
        {
            bail!("agent profile `{name}` must not declare an empty provider");
        }
        let mut seen = Vec::new();
        for role in &self.roles {
            if seen.contains(role) {
                bail!("agent profile `{name}` lists role `{}` twice", role.token());
            }
            seen.push(*role);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct AgentPolicy {
    profiles: BTreeMap<String, AgentProfileSpec>,
    default_profile: Option<String>,
    enforcement: bool,
}

impl AgentPolicy {
    pub(super) fn build(
        profiles: BTreeMap<String, AgentProfileSpec>,
        default_profile: Option<String>,
        enforce: Option<bool>,
    ) -> Result<Self> {
        for (name, profile) in &profiles {
            if name.trim().is_empty() {
                bail!("agent_profiles declares a profile with an empty name");
            }
            profile.validate(name)?;
        }
        if let Some(name) = default_profile.as_deref() {
            if !profiles.contains_key(name) {
                bail!("default_agent_profile `{name}` is not one of the configured agent_profiles");
            }
        }
        let enforcement = enforce.unwrap_or(!profiles.is_empty());
        Ok(Self {
            profiles,
            default_profile,
            enforcement,
        })
    }

    pub(super) fn enforcement(&self) -> bool {
        self.enforcement
    }

    pub(super) fn profile(&self, name: &str) -> Option<&AgentProfileSpec> {
        self.profiles.get(name)
    }

    pub(super) fn default_profile_name(&self) -> Option<&str> {
        self.default_profile.as_deref()
    }

    pub(super) fn profile_names(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }

    pub(super) fn profiles_by_preference(&self) -> Vec<(&str, &AgentProfileSpec)> {
        let mut entries = self
            .profiles
            .iter()
            .map(|(name, profile)| (name.as_str(), profile))
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.1
                .preference
                .unwrap_or(i64::MAX)
                .cmp(&right.1.preference.unwrap_or(i64::MAX))
                .then_with(|| left.0.cmp(right.0))
        });
        entries
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct DispatchPolicy {
    pub(super) agent: AgentPolicy,
    pub(super) default_model: Option<String>,
    pub(super) allowed_models: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct AssignmentRequest {
    pub(super) agent_profile: Option<String>,
    pub(super) task_role: Option<AgentRole>,
    pub(super) requires: Option<Vec<AgentRequirement>>,
}

impl AssignmentRequest {
    pub(super) fn is_unconstrained(&self) -> bool {
        self.agent_profile.is_none() && self.task_role.is_none() && self.requires.is_none()
    }

    pub(super) fn merged_with(&self, lane_key: &str, lane: &AssignmentRequest) -> Result<Self> {
        Ok(Self {
            agent_profile: merge_field(
                "agent_profile",
                lane_key,
                &self.agent_profile,
                &lane.agent_profile,
            )?,
            task_role: merge_field("task_role", lane_key, &self.task_role, &lane.task_role)?,
            requires: merge_field("requires", lane_key, &self.requires, &lane.requires)?,
        })
    }
}

fn merge_field<T: Clone>(
    field: &str,
    lane_key: &str,
    top: &Option<T>,
    lane: &Option<T>,
) -> Result<Option<T>> {
    match (top, lane) {
        (Some(_), Some(_)) => bail!(
            "`{field}` is set both at the top level and on lane `{lane_key}`; set it in one place so the lane's assignment is unambiguous"
        ),
        (Some(value), None) | (None, Some(value)) => Ok(Some(value.clone())),
        (None, None) => Ok(None),
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct RecordedIdentity {
    pub(super) assignment: Option<AgentAssignment>,
    pub(super) model: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct AgentDispatch {
    pub(super) policy: DispatchPolicy,
    pub(super) request: AssignmentRequest,
}

impl AgentDispatch {
    pub(super) fn new(policy: DispatchPolicy, request: AssignmentRequest) -> Self {
        Self { policy, request }
    }

    #[cfg(test)]
    pub(super) fn unconfigured() -> Self {
        Self {
            policy: DispatchPolicy::default(),
            request: AssignmentRequest::default(),
        }
    }

    pub(super) fn with_request(&self, request: AssignmentRequest) -> Self {
        Self {
            policy: self.policy.clone(),
            request,
        }
    }

    pub(super) fn is_constrained(&self) -> bool {
        self.policy.agent.enforcement() || !self.request.is_unconstrained()
    }

    pub(super) fn admit_launch(
        &self,
        tool: &str,
        explicit_model: Option<&str>,
    ) -> Result<Admission> {
        if !self.is_constrained() {
            let model = resolve_launch_model(&self.policy, None, explicit_model)?;
            return Ok(Admission {
                model,
                assignment: None,
            });
        }
        let (name, profile) = resolve_profile(&self.policy.agent, &self.request)?;
        if profile.tool != tool {
            bail!(
                "agent profile `{name}` declares tool `{}`, so it cannot launch `{tool}`; a profile stays bound to the harness it was declared for",
                profile.tool
            );
        }
        let task_role = require_role(name, profile, self.request.task_role)?;
        let requires = self.request.requires.clone().unwrap_or_default();
        require_capabilities(name, profile, &requires)?;
        let model = resolve_launch_model(&self.policy, Some((name, profile)), explicit_model)?;
        Ok(Admission {
            model,
            assignment: Some(assignment_from(name, profile, task_role, requires)),
        })
    }

    pub(super) fn admit_reuse(
        &self,
        tool: &str,
        explicit_model: Option<&str>,
        recorded: &RecordedIdentity,
    ) -> Result<Admission> {
        let Some(assignment) = recorded.assignment.as_ref() else {
            if self.is_constrained() {
                bail!(
                    "this constrained assignment needs an agent profile recorded against the live session, and that session has none; an unmanaged session is never relabelled with a trusted profile. Spawn a fresh managed lane, or pass resume=false under a new key"
                );
            }
            let model = match (recorded.model.as_deref(), explicit_model) {
                (Some(recorded_model), Some(requested)) if requested != recorded_model => bail!(
                    "this session's recorded launch used model `{recorded_model}`, so requesting model `{requested}` would report a model switch that does not happen; the recorded identity wins, or spawn a fresh lane"
                ),
                (Some(recorded_model), _) => Some(recorded_model.to_owned()),
                (None, requested) => requested.map(str::to_owned),
            };
            if let Some(model) = model.as_deref() {
                enforce_allowed_model(model, &self.policy.allowed_models)?;
            }
            return Ok(Admission {
                model,
                assignment: None,
            });
        };
        let profile = verify_recorded(&self.policy.agent, assignment, Some(tool), explicit_model)?;
        let assignment = resolve_recorded_assignment(&self.request, assignment, profile)?;
        let model = Some(assignment.model.clone());
        enforce_allowed_model(assignment.model.as_str(), &self.policy.allowed_models)?;
        Ok(Admission {
            model,
            assignment: Some(assignment),
        })
    }

    pub(super) fn admit_submit(
        &self,
        recorded: Option<&AgentAssignment>,
    ) -> Result<Option<AgentAssignment>> {
        let Some(recorded) = recorded else {
            if self.is_constrained() {
                bail!(
                    "this constrained submit needs an agent profile recorded against the session, and it has none; a live session is never relabelled with a trusted profile. Submit to a managed lane, or spawn one"
                );
            }
            return Ok(None);
        };
        let profile = verify_recorded(&self.policy.agent, recorded, None, None)?;
        enforce_allowed_model(recorded.model.as_str(), &self.policy.allowed_models)?;
        Ok(Some(resolve_recorded_assignment(
            &self.request,
            recorded,
            profile,
        )?))
    }

    pub(super) fn verify_prior_assignment(&self, recorded: &AgentAssignment) -> Result<()> {
        verify_recorded(&self.policy.agent, recorded, None, None).map(|_| ())
    }
}

#[derive(Clone, Debug)]
pub(super) struct Admission {
    pub(super) model: Option<String>,
    pub(super) assignment: Option<AgentAssignment>,
}

impl Admission {
    pub(super) fn status(&self) -> AgentStatus {
        AgentStatus::of(self.assignment.as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct AgentAssignment {
    pub(super) profile: String,
    pub(super) tool: String,
    pub(super) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) provider: Option<String>,
    pub(super) roles: Vec<AgentRole>,
    pub(super) image_input: ImageInput,
    pub(super) visual_review: VisualReview,
    pub(super) task_role: AgentRole,
    pub(super) requires: Vec<AgentRequirement>,
    #[serde(default = "default_evidence_basis")]
    pub(super) evidence_basis: String,
}

fn default_evidence_basis() -> String {
    EVIDENCE_BASIS.to_owned()
}

fn assignment_from(
    profile_name: &str,
    profile: &AgentProfileSpec,
    task_role: AgentRole,
    requires: Vec<AgentRequirement>,
) -> AgentAssignment {
    AgentAssignment {
        profile: profile_name.to_owned(),
        tool: profile.tool.clone(),
        model: profile.model.clone(),
        provider: profile.provider.clone(),
        roles: profile.roles.clone(),
        image_input: profile.image_input,
        visual_review: profile.visual_review,
        task_role,
        requires,
        evidence_basis: EVIDENCE_BASIS.to_owned(),
    }
}

fn resolve_profile<'a>(
    policy: &'a AgentPolicy,
    request: &'a AssignmentRequest,
) -> Result<(&'a str, &'a AgentProfileSpec)> {
    let name = match request.agent_profile.as_deref() {
        Some(name) => name,
        None => policy.default_profile_name().ok_or_else(|| {
            anyhow!(
                "a constrained assignment requires an explicit agent_profile or a default_agent_profile in sessions.toml; neither was supplied"
            )
        })?,
    };
    let Some(profile) = policy.profile(name) else {
        let configured = policy.profile_names();
        let hint = if configured.is_empty() {
            "no agent profiles are configured".to_owned()
        } else {
            format!("configured agent profiles: {}", configured.join(", "))
        };
        bail!("agent profile `{name}` is not configured in sessions.toml ({hint})");
    };
    Ok((name, profile))
}

fn require_role(
    profile_name: &str,
    profile: &AgentProfileSpec,
    requested: Option<AgentRole>,
) -> Result<AgentRole> {
    let Some(role) = requested else {
        bail!(
            "a constrained assignment to agent profile `{profile_name}` requires an explicit task_role (one of {}); pass one even when requires is empty",
            role_catalog()
        )
    };
    if !profile.roles.contains(&role) {
        bail!(
            "agent profile `{profile_name}` does not permit task_role `{}`; it declares {}",
            role.token(),
            role_tokens(&profile.roles)
        );
    }
    Ok(role)
}

fn require_capabilities(
    profile_name: &str,
    profile: &AgentProfileSpec,
    requires: &[AgentRequirement],
) -> Result<()> {
    let mut seen = Vec::new();
    for requirement in requires {
        if seen.contains(requirement) {
            bail!(
                "requirement `{}` is listed twice for agent profile `{profile_name}`",
                requirement.token()
            );
        }
        seen.push(*requirement);
    }
    for requirement in seen {
        match requirement {
            AgentRequirement::ImageInput => {
                if profile.image_input != ImageInput::Native {
                    bail!(
                        "agent profile `{profile_name}` cannot satisfy requirement `image_input`: it declares image_input=`{}`, and only a native declaration satisfies it",
                        profile.image_input.token()
                    );
                }
            }
            AgentRequirement::VisualReview => {
                if profile.image_input != ImageInput::Native
                    || profile.visual_review != VisualReview::Allow
                {
                    bail!(
                        "agent profile `{profile_name}` cannot satisfy requirement `visual_review`: it declares image_input=`{}` and visual_review=`{}`, and visual review needs a native declaration plus an allow declaration",
                        profile.image_input.token(),
                        profile.visual_review.token()
                    );
                }
            }
        }
    }
    Ok(())
}

fn resolve_launch_model(
    policy: &DispatchPolicy,
    profile: Option<(&str, &AgentProfileSpec)>,
    explicit_model: Option<&str>,
) -> Result<Option<String>> {
    let model = match profile {
        Some((name, profile)) => {
            if let Some(explicit) = explicit_model {
                if explicit != profile.model {
                    bail!(
                        "agent profile `{name}` declares model `{}`, so requesting model `{explicit}` conflicts; a profile assignment launches its declared tool and model",
                        profile.model
                    );
                }
            }
            Some(profile.model.clone())
        }
        None => explicit_model
            .map(str::to_owned)
            .or_else(|| policy.default_model.clone()),
    };
    if let Some(model) = model.as_deref() {
        enforce_allowed_model(model, &policy.allowed_models)?;
    }
    Ok(model)
}

fn verify_recorded<'a>(
    policy: &'a AgentPolicy,
    recorded: &AgentAssignment,
    expected_tool: Option<&str>,
    explicit_model: Option<&str>,
) -> Result<&'a AgentProfileSpec> {
    let Some(profile) = policy.profile(&recorded.profile) else {
        bail!(
            "agent profile `{}` recorded for this session is no longer configured, so its assignment is revoked; a revoked profile cannot be reused, submitted to, or resumed",
            recorded.profile
        )
    };
    if profile.tool != recorded.tool {
        bail!(
            "agent profile `{}` now declares tool `{}` but this session was assigned tool `{}`; the recorded assignment no longer satisfies current policy",
            recorded.profile,
            profile.tool,
            recorded.tool
        );
    }
    if profile.model != recorded.model {
        bail!(
            "agent profile `{}` now declares model `{}` but this session was assigned model `{}`; the recorded assignment no longer satisfies current policy",
            recorded.profile,
            profile.model,
            recorded.model
        );
    }
    if let Some(tool) = expected_tool {
        if tool != recorded.tool {
            bail!(
                "this session runs tool `{}`, so it cannot take a `{tool}` assignment",
                recorded.tool
            );
        }
    }
    if let Some(model) = explicit_model {
        if model != recorded.model {
            bail!(
                "this session already runs model `{}`, so requesting model `{model}` would report a model switch that does not happen; the recorded identity wins, or spawn a fresh lane",
                recorded.model
            );
        }
    }
    require_role(&recorded.profile, profile, Some(recorded.task_role))?;
    require_capabilities(&recorded.profile, profile, &recorded.requires)?;
    Ok(profile)
}

fn resolve_recorded_assignment(
    request: &AssignmentRequest,
    recorded: &AgentAssignment,
    profile: &AgentProfileSpec,
) -> Result<AgentAssignment> {
    if let Some(name) = request.agent_profile.as_deref() {
        if name != recorded.profile {
            bail!(
                "this session is assigned agent profile `{}`, so requesting profile `{name}` conflicts; a live session keeps its recorded identity",
                recorded.profile
            );
        }
    }
    let task_role = request.task_role.unwrap_or(recorded.task_role);
    require_role(&recorded.profile, profile, Some(task_role))?;
    let requires = request
        .requires
        .clone()
        .unwrap_or_else(|| recorded.requires.clone());
    require_capabilities(&recorded.profile, profile, &requires)?;
    Ok(assignment_from(
        &recorded.profile,
        profile,
        task_role,
        requires,
    ))
}

pub(super) fn enforce_allowed_model(model: &str, allowed: &[String]) -> Result<()> {
    if allowed.is_empty() || allowed.iter().any(|entry| entry == model) {
        return Ok(());
    }
    bail!(
        "model {model} is not one this host may launch. sessions.toml allows: {}. Model tiers are billed per token, so only the person paying picks one: ask them to widen allowed_models rather than passing another tier",
        allowed.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(
        tool: &str,
        model: &str,
        roles: Vec<AgentRole>,
        image_input: ImageInput,
        visual_review: VisualReview,
    ) -> AgentProfileSpec {
        AgentProfileSpec {
            tool: tool.to_owned(),
            model: model.to_owned(),
            provider: None,
            roles,
            image_input,
            visual_review,
            preference: None,
        }
    }

    fn policy_with(
        name: &str,
        profile: AgentProfileSpec,
        default_profile: Option<&str>,
        enforce: Option<bool>,
    ) -> AgentPolicy {
        let mut profiles = BTreeMap::new();
        profiles.insert(name.to_owned(), profile);
        AgentPolicy::build(profiles, default_profile.map(str::to_owned), enforce).unwrap()
    }

    fn dispatch(policy: AgentPolicy, request: AssignmentRequest) -> AgentDispatch {
        AgentDispatch::new(
            DispatchPolicy {
                agent: policy,
                default_model: None,
                allowed_models: Vec::new(),
            },
            request,
        )
    }

    fn request(
        agent_profile: Option<&str>,
        task_role: Option<AgentRole>,
        requires: Option<Vec<AgentRequirement>>,
    ) -> AssignmentRequest {
        AssignmentRequest {
            agent_profile: agent_profile.map(str::to_owned),
            task_role,
            requires,
        }
    }

    fn identity(assignment: &AgentAssignment) -> RecordedIdentity {
        RecordedIdentity {
            assignment: Some(assignment.clone()),
            model: Some(assignment.model.clone()),
        }
    }

    #[test]
    fn role_and_requirement_tokens_round_trip_the_closed_sets() {
        for role in AgentRole::all() {
            assert_eq!(AgentRole::from_token(role.token()), Some(role));
        }
        assert_eq!(AgentRole::from_token("supervisor"), None);
        for requirement in [AgentRequirement::ImageInput, AgentRequirement::VisualReview] {
            assert_eq!(
                AgentRequirement::from_token(requirement.token()),
                Some(requirement)
            );
        }
        assert_eq!(AgentRequirement::from_token("sight"), None);
        assert_eq!(
            parse_requires("visual_review, image_input").unwrap(),
            vec![AgentRequirement::VisualReview, AgentRequirement::ImageInput]
        );
        assert!(parse_requires("image_input,image_input").is_err());
        assert_eq!(parse_requires("").unwrap(), Vec::new());
        assert_eq!(parse_requires("   ").unwrap(), Vec::new());
        assert!(parse_requires("image_input,").is_err());
        assert!(parse_requires(",").is_err());
        assert!(parse_requires("vision").is_err());
    }

    #[test]
    fn configuring_profiles_enables_enforcement_unless_explicitly_disabled() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        assert!(policy_with("worker", spec.clone(), None, None).enforcement());
        assert!(!policy_with("worker", spec.clone(), None, Some(false)).enforcement());
        assert!(policy_with("worker", spec, None, Some(true)).enforcement());
        assert!(!AgentPolicy::default().enforcement());
    }

    #[test]
    fn unknown_profile_values_and_keys_are_rejected_by_the_parser() {
        let unknown_role = toml::from_str::<AgentProfileSpec>(
            "tool = \"pi\"\nmodel = \"flash\"\nroles = [\"supervisor\"]\n",
        );
        assert!(unknown_role.is_err());
        let unknown_image = toml::from_str::<AgentProfileSpec>(
            "tool = \"pi\"\nmodel = \"flash\"\nroles = [\"implement\"]\nimage_input = \"screenshots\"\n",
        );
        assert!(unknown_image.is_err());
        let unknown_trust = toml::from_str::<AgentProfileSpec>(
            "tool = \"pi\"\nmodel = \"flash\"\nroles = [\"implement\"]\nvisual_review = \"maybe\"\n",
        );
        assert!(unknown_trust.is_err());
        let unknown_key = toml::from_str::<AgentProfileSpec>(
            "tool = \"pi\"\nmodel = \"flash\"\nroles = [\"implement\"]\nvision = \"yes\"\n",
        );
        assert!(unknown_key.is_err());
    }

    #[test]
    fn profiles_reject_empty_identities_and_duplicate_roles() {
        let empty_tool = AgentPolicy::build(
            BTreeMap::from([(
                "worker".to_owned(),
                profile(
                    "  ",
                    "flash",
                    vec![AgentRole::Implement],
                    ImageInput::Unknown,
                    VisualReview::Deny,
                ),
            )]),
            None,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(empty_tool.contains("non-empty tool"), "{empty_tool}");

        let empty_model = AgentPolicy::build(
            BTreeMap::from([(
                "worker".to_owned(),
                profile(
                    "pi",
                    "",
                    vec![AgentRole::Implement],
                    ImageInput::Unknown,
                    VisualReview::Deny,
                ),
            )]),
            None,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(empty_model.contains("non-empty model"), "{empty_model}");

        let duplicate = AgentPolicy::build(
            BTreeMap::from([(
                "worker".to_owned(),
                profile(
                    "pi",
                    "flash",
                    vec![AgentRole::Implement, AgentRole::Implement],
                    ImageInput::Unknown,
                    VisualReview::Deny,
                ),
            )]),
            None,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(duplicate.contains("twice"), "{duplicate}");

        let missing_default = AgentPolicy::build(
            BTreeMap::from([(
                "worker".to_owned(),
                profile(
                    "pi",
                    "flash",
                    vec![AgentRole::Implement],
                    ImageInput::Unknown,
                    VisualReview::Deny,
                ),
            )]),
            Some("absent".to_owned()),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(missing_default.contains("absent"), "{missing_default}");
    }

    #[test]
    fn the_image_and_trust_truth_table_only_admits_native_and_allow() {
        let cases: [(ImageInput, VisualReview, bool, bool); 6] = [
            (ImageInput::None, VisualReview::Deny, false, false),
            (ImageInput::None, VisualReview::Allow, false, false),
            (ImageInput::Unknown, VisualReview::Deny, false, false),
            (ImageInput::Unknown, VisualReview::Allow, false, false),
            (ImageInput::Native, VisualReview::Deny, true, false),
            (ImageInput::Native, VisualReview::Allow, true, true),
        ];
        for (image_input, visual_review, image_ok, visual_ok) in cases {
            let spec = profile(
                "pi",
                "flash",
                vec![AgentRole::Review],
                image_input,
                visual_review,
            );
            let image_call = request(
                Some("visual"),
                Some(AgentRole::Review),
                Some(vec![AgentRequirement::ImageInput]),
            );
            let admitted = dispatch(policy_with("visual", spec, None, None), image_call)
                .admit_launch("pi", Some("flash"));
            assert_eq!(
                admitted.is_ok(),
                image_ok,
                "image_input={} visual_review={}",
                image_input.token(),
                visual_review.token()
            );

            let spec = profile(
                "pi",
                "flash",
                vec![AgentRole::Review],
                image_input,
                visual_review,
            );
            let visual_call = request(
                Some("visual"),
                Some(AgentRole::Review),
                Some(vec![AgentRequirement::VisualReview]),
            );
            let admitted = dispatch(policy_with("visual", spec, None, None), visual_call)
                .admit_launch("pi", Some("flash"));
            assert_eq!(
                admitted.is_ok(),
                visual_ok,
                "image_input={} visual_review={}",
                image_input.token(),
                visual_review.token()
            );
        }
    }

    #[test]
    fn a_written_specification_needs_no_visual_declaration() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::None,
            VisualReview::Deny,
        );
        let admitted = dispatch(
            policy_with("visual", spec, None, None),
            request(Some("visual"), Some(AgentRole::Implement), Some(Vec::new())),
        )
        .admit_launch("pi", Some("flash"))
        .unwrap();
        let assignment = admitted.assignment.unwrap();
        assert!(assignment.requires.is_empty());
        assert_eq!(assignment.evidence_basis, EVIDENCE_BASIS);
    }

    #[test]
    fn a_default_profile_supplies_the_identity_when_the_call_omits_one() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let policy = policy_with("worker", spec, Some("worker"), None);
        let admitted = dispatch(policy, request(None, Some(AgentRole::Implement), None))
            .admit_launch("pi", None)
            .unwrap();
        assert_eq!(admitted.model.as_deref(), Some("flash"));
        assert_eq!(admitted.assignment.unwrap().profile, "worker");
    }

    #[test]
    fn a_constrained_assignment_requires_a_role_even_with_no_requirements() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("worker", spec, None, None),
            request(Some("worker"), None, None),
        )
        .admit_launch("pi", None)
        .unwrap_err()
        .to_string();
        assert!(error.contains("task_role"), "{error}");
        assert!(error.contains("worker"), "{error}");
    }

    #[test]
    fn a_role_outside_the_profile_is_rejected_by_name() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("worker", spec, None, None),
            request(Some("worker"), Some(AgentRole::Review), None),
        )
        .admit_launch("pi", None)
        .unwrap_err()
        .to_string();
        assert!(error.contains("profile `worker`"), "{error}");
        assert!(error.contains("review"), "{error}");
        assert!(error.contains("implement"), "{error}");
    }

    #[test]
    fn a_requirement_diagnostic_names_the_profile_and_the_declaration() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Review],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("visual", spec, None, None),
            request(
                Some("visual"),
                Some(AgentRole::Review),
                Some(vec![AgentRequirement::ImageInput]),
            ),
        )
        .admit_launch("pi", None)
        .unwrap_err()
        .to_string();
        assert!(error.contains("`visual`"), "{error}");
        assert!(error.contains("image_input"), "{error}");
        assert!(error.contains("unknown"), "{error}");
        assert!(!error.contains("allowed_models"), "{error}");
    }

    #[test]
    fn an_unknown_profile_names_itself_and_the_configured_names() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("worker", spec, None, None),
            request(Some("typo"), Some(AgentRole::Implement), None),
        )
        .admit_launch("pi", None)
        .unwrap_err()
        .to_string();
        assert!(error.contains("`typo`"), "{error}");
        assert!(error.contains("worker"), "{error}");
    }

    #[test]
    fn a_profile_cannot_change_the_harness_or_the_model_it_was_bound_to() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let wrong_tool = dispatch(
            policy_with("worker", spec.clone(), None, None),
            request(Some("worker"), Some(AgentRole::Implement), None),
        )
        .admit_launch("codex", None)
        .unwrap_err()
        .to_string();
        assert!(wrong_tool.contains("pi"), "{wrong_tool}");
        assert!(wrong_tool.contains("codex"), "{wrong_tool}");

        let conflicting_model = dispatch(
            policy_with("worker", spec, None, None),
            request(Some("worker"), Some(AgentRole::Implement), None),
        )
        .admit_launch("pi", Some("pro"))
        .unwrap_err()
        .to_string();
        assert!(conflicting_model.contains("flash"), "{conflicting_model}");
        assert!(conflicting_model.contains("pro"), "{conflicting_model}");
    }

    #[test]
    fn vendor_names_never_grant_capability() {
        let spec = profile(
            "deepseek-chat-vision",
            "vision-ultra-omni",
            vec![AgentRole::Review],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("vision-model", spec, None, None),
            request(
                Some("vision-model"),
                Some(AgentRole::Review),
                Some(vec![AgentRequirement::VisualReview]),
            ),
        )
        .admit_launch("deepseek-chat-vision", Some("vision-ultra-omni"))
        .unwrap_err()
        .to_string();
        assert!(error.contains("visual_review"), "{error}");
    }

    #[test]
    fn the_spending_allowlist_refuses_a_profile_model_without_widening_it() {
        let spec = profile(
            "pi",
            "pro",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let policy =
            AgentPolicy::build(BTreeMap::from([("worker".to_owned(), spec)]), None, None).unwrap();
        let dispatch = AgentDispatch::new(
            DispatchPolicy {
                agent: policy,
                default_model: None,
                allowed_models: vec!["flash".to_owned()],
            },
            request(Some("worker"), Some(AgentRole::Implement), None),
        );
        let error = dispatch.admit_launch("pi", None).unwrap_err().to_string();
        assert!(error.contains("pro"), "{error}");
        assert!(error.contains("flash"), "{error}");
    }

    #[test]
    fn an_unconstrained_call_resolves_the_default_model_without_an_assignment() {
        let dispatch = AgentDispatch::new(
            DispatchPolicy {
                agent: AgentPolicy::default(),
                default_model: Some("flash".to_owned()),
                allowed_models: Vec::new(),
            },
            AssignmentRequest::default(),
        );
        assert!(!dispatch.is_constrained());
        let admitted = dispatch.admit_launch("pi", None).unwrap();
        assert_eq!(admitted.model.as_deref(), Some("flash"));
        assert!(admitted.assignment.is_none());
        assert_eq!(admitted.status(), AgentStatus::Unconstrained);
    }

    #[test]
    fn recorded_reuse_revalidates_current_policy_and_blocks_revocation() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Native,
            VisualReview::Allow,
        );
        let recorded = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: None,
            roles: vec![AgentRole::Implement],
            image_input: ImageInput::Native,
            visual_review: VisualReview::Allow,
            task_role: AgentRole::Implement,
            requires: vec![AgentRequirement::VisualReview],
            evidence_basis: EVIDENCE_BASIS.to_owned(),
        };
        let reused = dispatch(
            policy_with("worker", spec.clone(), None, None),
            AssignmentRequest::default(),
        )
        .admit_reuse("pi", None, &identity(&recorded))
        .unwrap();
        assert_eq!(reused.assignment.unwrap().task_role, AgentRole::Implement);

        let downgraded = dispatch(
            policy_with(
                "worker",
                profile(
                    "pi",
                    "flash",
                    vec![AgentRole::Implement],
                    ImageInput::Unknown,
                    VisualReview::Deny,
                ),
                None,
                None,
            ),
            AssignmentRequest::default(),
        )
        .admit_reuse("pi", None, &identity(&recorded))
        .unwrap_err()
        .to_string();
        assert!(downgraded.contains("visual_review"), "{downgraded}");
        assert!(downgraded.contains("worker"), "{downgraded}");

        let revoked = dispatch(
            AgentPolicy::build(BTreeMap::new(), None, Some(false)).unwrap(),
            AssignmentRequest::default(),
        )
        .admit_reuse("pi", None, &identity(&recorded))
        .unwrap_err()
        .to_string();
        assert!(revoked.contains("no longer configured"), "{revoked}");
        assert!(revoked.contains("worker"), "{revoked}");

        let unconfigured = dispatch(AgentPolicy::default(), AssignmentRequest::default())
            .admit_reuse("pi", None, &identity(&recorded))
            .unwrap_err()
            .to_string();
        assert!(
            unconfigured.contains("no longer configured"),
            "{unconfigured}"
        );
    }

    #[test]
    fn recorded_reuse_rejects_a_conflicting_profile_or_model() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let other = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let recorded = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: None,
            roles: vec![AgentRole::Implement],
            image_input: ImageInput::Unknown,
            visual_review: VisualReview::Deny,
            task_role: AgentRole::Implement,
            requires: Vec::new(),
            evidence_basis: EVIDENCE_BASIS.to_owned(),
        };
        let mut profiles = BTreeMap::new();
        profiles.insert("worker".to_owned(), spec);
        profiles.insert("other".to_owned(), other);
        let policy = AgentPolicy::build(profiles, None, None).unwrap();

        let conflicting_profile = dispatch(
            policy.clone(),
            request(Some("other"), Some(AgentRole::Implement), None),
        )
        .admit_reuse("pi", Some("flash"), &identity(&recorded))
        .unwrap_err()
        .to_string();
        assert!(
            conflicting_profile.contains("other"),
            "{conflicting_profile}"
        );

        let conflicting_model = dispatch(
            policy,
            request(Some("worker"), Some(AgentRole::Implement), None),
        )
        .admit_reuse("pi", Some("pro"), &identity(&recorded))
        .unwrap_err()
        .to_string();
        assert!(conflicting_model.contains("flash"), "{conflicting_model}");
    }

    #[test]
    fn constrained_reuse_without_a_recorded_assignment_is_refused() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("worker", spec, None, None),
            AssignmentRequest::default(),
        )
        .admit_reuse("pi", None, &RecordedIdentity::default())
        .unwrap_err()
        .to_string();
        assert!(error.contains("resume=false"), "{error}");

        let legacy = dispatch(AgentPolicy::default(), AssignmentRequest::default())
            .admit_reuse("pi", Some("flash"), &RecordedIdentity::default())
            .unwrap();
        assert!(legacy.assignment.is_none());
        assert_eq!(legacy.model.as_deref(), Some("flash"));
    }

    #[test]
    fn submit_inherits_the_recorded_profile_role_and_requirements() {
        let mut spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement, AgentRole::Debug],
            ImageInput::Native,
            VisualReview::Allow,
        );
        spec.provider = Some("current-provider".to_owned());
        let recorded = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: Some("informational".to_owned()),
            roles: vec![AgentRole::Implement, AgentRole::Debug],
            image_input: ImageInput::Native,
            visual_review: VisualReview::Allow,
            task_role: AgentRole::Debug,
            requires: vec![AgentRequirement::VisualReview],
            evidence_basis: EVIDENCE_BASIS.to_owned(),
        };
        let inherited = dispatch(
            policy_with("worker", spec.clone(), None, None),
            AssignmentRequest::default(),
        )
        .admit_submit(Some(&recorded))
        .unwrap()
        .unwrap();
        assert_eq!(inherited.task_role, AgentRole::Debug);
        assert_eq!(inherited.requires, vec![AgentRequirement::VisualReview]);
        assert_eq!(
            inherited.provider.as_deref(),
            Some("current-provider"),
            "informational provider metadata snapshots current policy rather than the recorded assignment"
        );

        let new_requirement = dispatch(
            policy_with("worker", spec, None, None),
            request(None, None, Some(vec![AgentRequirement::ImageInput])),
        )
        .admit_submit(Some(&recorded))
        .unwrap()
        .unwrap();
        assert_eq!(new_requirement.requires, vec![AgentRequirement::ImageInput]);
    }

    #[test]
    fn constrained_submit_without_a_recorded_assignment_is_refused() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let error = dispatch(
            policy_with("worker", spec, None, None),
            AssignmentRequest::default(),
        )
        .admit_submit(None)
        .unwrap_err()
        .to_string();
        assert!(error.contains("managed lane"), "{error}");
        assert!(
            dispatch(AgentPolicy::default(), AssignmentRequest::default())
                .admit_submit(None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_bound_record_without_an_assignment_reports_its_model_and_rejects_a_conflict() {
        let dispatch = dispatch(AgentPolicy::default(), AssignmentRequest::default());
        let bound = RecordedIdentity {
            assignment: None,
            model: Some("flash".to_owned()),
        };
        let reused = dispatch.admit_reuse("pi", None, &bound).unwrap();
        assert!(reused.assignment.is_none());
        assert_eq!(reused.model.as_deref(), Some("flash"));

        let conflict = dispatch
            .admit_reuse("pi", Some("pro"), &bound)
            .unwrap_err()
            .to_string();
        assert!(conflict.contains("flash"), "{conflict}");
        assert!(conflict.contains("pro"), "{conflict}");

        let unrecorded = dispatch
            .admit_reuse("pi", Some("pro"), &RecordedIdentity::default())
            .unwrap();
        assert_eq!(unrecorded.model.as_deref(), Some("pro"));
        assert!(unrecorded.assignment.is_none());
    }

    #[test]
    fn a_recorded_model_outside_the_allowlist_blocks_a_constrained_submit() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        let recorded = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: None,
            roles: vec![AgentRole::Implement],
            image_input: ImageInput::Unknown,
            visual_review: VisualReview::Deny,
            task_role: AgentRole::Implement,
            requires: Vec::new(),
            evidence_basis: EVIDENCE_BASIS.to_owned(),
        };
        let blocked = AgentDispatch::new(
            DispatchPolicy {
                agent: policy_with("worker", spec.clone(), None, None),
                default_model: None,
                allowed_models: vec!["other".to_owned()],
            },
            AssignmentRequest::default(),
        );
        let error = blocked
            .admit_submit(Some(&recorded))
            .unwrap_err()
            .to_string();
        assert!(error.contains("flash"), "{error}");
        assert!(error.contains("other"), "{error}");

        let permissive = AgentDispatch::new(
            DispatchPolicy {
                agent: policy_with("worker", spec, None, None),
                default_model: None,
                allowed_models: Vec::new(),
            },
            AssignmentRequest::default(),
        );
        assert!(permissive.admit_submit(Some(&recorded)).is_ok());
    }

    #[test]
    fn prior_assignment_revalidation_blocks_a_redeclared_model_and_a_revoked_permission() {
        let recorded = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: None,
            roles: vec![AgentRole::Implement],
            image_input: ImageInput::Native,
            visual_review: VisualReview::Allow,
            task_role: AgentRole::Implement,
            requires: vec![AgentRequirement::VisualReview],
            evidence_basis: EVIDENCE_BASIS.to_owned(),
        };
        let redeclared = dispatch(
            policy_with(
                "worker",
                profile(
                    "pi",
                    "upgraded",
                    vec![AgentRole::Implement],
                    ImageInput::Native,
                    VisualReview::Allow,
                ),
                None,
                None,
            ),
            AssignmentRequest::default(),
        );
        let error = redeclared
            .verify_prior_assignment(&recorded)
            .unwrap_err()
            .to_string();
        assert!(error.contains("upgraded"), "{error}");
        assert!(error.contains("flash"), "{error}");

        let revoked = dispatch(
            policy_with(
                "worker",
                profile(
                    "pi",
                    "flash",
                    vec![AgentRole::Implement],
                    ImageInput::Unknown,
                    VisualReview::Deny,
                ),
                None,
                None,
            ),
            AssignmentRequest::default(),
        );
        let error = revoked
            .verify_prior_assignment(&recorded)
            .unwrap_err()
            .to_string();
        assert!(error.contains("visual_review"), "{error}");

        let compatible = dispatch(
            policy_with(
                "worker",
                profile(
                    "pi",
                    "flash",
                    vec![AgentRole::Implement],
                    ImageInput::Native,
                    VisualReview::Allow,
                ),
                None,
                None,
            ),
            AssignmentRequest::default(),
        );
        assert!(compatible.verify_prior_assignment(&recorded).is_ok());
    }

    #[test]
    fn an_explicitly_empty_requires_clears_inherited_requirements_on_submit() {
        let spec = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Native,
            VisualReview::Allow,
        );
        let recorded = AgentAssignment {
            profile: "worker".to_owned(),
            tool: "pi".to_owned(),
            model: "flash".to_owned(),
            provider: None,
            roles: vec![AgentRole::Implement],
            image_input: ImageInput::Native,
            visual_review: VisualReview::Allow,
            task_role: AgentRole::Implement,
            requires: vec![AgentRequirement::VisualReview],
            evidence_basis: EVIDENCE_BASIS.to_owned(),
        };
        let inherited = dispatch(
            policy_with("worker", spec.clone(), None, None),
            AssignmentRequest::default(),
        )
        .admit_submit(Some(&recorded))
        .unwrap()
        .unwrap();
        assert_eq!(inherited.requires, vec![AgentRequirement::VisualReview]);

        let cleared = dispatch(
            policy_with("worker", spec, None, None),
            request(None, None, Some(Vec::new())),
        )
        .admit_submit(Some(&recorded))
        .unwrap()
        .unwrap();
        assert!(
            cleared.requires.is_empty(),
            "an explicit empty list must clear the inherited visual requirement"
        );
    }

    #[test]
    fn lane_requests_inherit_top_level_defaults_and_reject_duplicates() {
        let top = request(Some("worker"), Some(AgentRole::Implement), None);
        let lane = request(None, None, Some(vec![AgentRequirement::ImageInput]));
        let merged = top.merged_with("lane-a", &lane).unwrap();
        assert_eq!(merged.agent_profile.as_deref(), Some("worker"));
        assert_eq!(merged.task_role, Some(AgentRole::Implement));
        assert_eq!(merged.requires, Some(vec![AgentRequirement::ImageInput]));

        let conflict = request(Some("other"), None, None);
        let error = top
            .merged_with("lane-b", &conflict)
            .unwrap_err()
            .to_string();
        assert!(error.contains("agent_profile"), "{error}");
        assert!(error.contains("lane-b"), "{error}");
    }

    #[test]
    fn profiles_sort_by_preference_then_name() {
        let mut profiles = BTreeMap::new();
        let mut slow = profile(
            "pi",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        slow.preference = Some(20);
        let mut fast = profile(
            "codex",
            "flash",
            vec![AgentRole::Implement],
            ImageInput::Unknown,
            VisualReview::Deny,
        );
        fast.preference = Some(10);
        profiles.insert("slow".to_owned(), slow);
        profiles.insert("fast".to_owned(), fast);
        profiles.insert(
            "unranked".to_owned(),
            profile(
                "claude",
                "flash",
                vec![AgentRole::Implement],
                ImageInput::Unknown,
                VisualReview::Deny,
            ),
        );
        let policy = AgentPolicy::build(profiles, None, None).unwrap();
        let names = policy
            .profiles_by_preference()
            .into_iter()
            .map(|(name, _)| name.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["fast", "slow", "unranked"]);
    }
}
