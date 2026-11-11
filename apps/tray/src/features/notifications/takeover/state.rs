use serde::{Deserialize, Serialize};

const NO_PERMISSION_NEEDED: &str = "none";
const PERMISSION_JOIN: &str = "_and_";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    FullDiskAccess,
    Accessibility,
}

impl Permission {
    pub fn key(self) -> &'static str {
        match self {
            Self::FullDiskAccess => "full_disk_access",
            Self::Accessibility => "accessibility",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FullDiskAccess => "Full Disk Access",
            Self::Accessibility => "Accessibility",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum Health {
    Active,
    Starting,
    NeedsPermission(Permission),
    Failed(String),
}

impl Health {
    fn describe(&self) -> String {
        match self {
            Self::Active => "active".to_string(),
            Self::Starting => "starting".to_string(),
            Self::NeedsPermission(permission) => format!("needs {}", permission.label()),
            Self::Failed(reason) => format!("failed: {reason}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mechanism {
    pub name: String,
    pub shows: bool,
    pub hides: bool,
    pub health: Health,
}

impl Mechanism {
    pub fn showing(name: &str, health: Health) -> Self {
        Self::new(name, true, false, health)
    }

    pub fn hiding(name: &str, health: Health) -> Self {
        Self::new(name, false, true, health)
    }

    pub fn showing_and_hiding(name: &str, health: Health) -> Self {
        Self::new(name, true, true, health)
    }

    fn new(name: &str, shows: bool, hides: bool, health: Health) -> Self {
        Self {
            name: name.to_string(),
            shows,
            hides,
            health,
        }
    }

    pub fn describe(&self) -> String {
        format!("{}: {}", self.name, self.health.describe())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Off,
    Starting,
    Toasts,
    ToastsPartly,
    ToastsAndBanners,
    Hidden,
    Muted,
    Blocked,
    Unsupported,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverState {
    pub value: Outcome,
    pub needs: String,
    pub wanted: bool,
    pub mechanisms: Vec<Mechanism>,
}

impl TakeoverState {
    pub fn off() -> Self {
        Self::new(false, true, Vec::new())
    }

    pub fn new(wanted: bool, notifications_on: bool, mechanisms: Vec<Mechanism>) -> Self {
        Self {
            value: outcome(wanted, notifications_on, &mechanisms),
            needs: needs(&mechanisms),
            wanted,
            mechanisms,
        }
    }

    pub fn permissions(&self) -> Vec<Permission> {
        permissions(&self.mechanisms)
    }

    pub fn describe(&self) -> String {
        let parts: Vec<String> = self.mechanisms.iter().map(Mechanism::describe).collect();
        format!("{:?} ({})", self.value, parts.join(", "))
    }
}

fn outcome(wanted: bool, notifications_on: bool, mechanisms: &[Mechanism]) -> Outcome {
    if !wanted {
        return Outcome::Off;
    }
    if mechanisms.is_empty() {
        return Outcome::Unsupported;
    }
    if mechanisms
        .iter()
        .any(|mechanism| mechanism.health == Health::Starting)
    {
        return Outcome::Starting;
    }
    let active = |mechanism: &&Mechanism| mechanism.health == Health::Active;
    let shows = mechanisms.iter().filter(active).any(|part| part.shows);
    let hides = mechanisms.iter().filter(active).any(|part| part.hides);
    let whole = mechanisms.iter().filter(active).count() == mechanisms.len();
    match (shows, hides) {
        (_, true) if !notifications_on => Outcome::Muted,
        (true, true) if whole => Outcome::Toasts,
        (true, true) => Outcome::ToastsPartly,
        (true, false) => Outcome::ToastsAndBanners,
        (false, true) => Outcome::Hidden,
        (false, false) if !permissions(mechanisms).is_empty() => Outcome::Blocked,
        (false, false) => Outcome::Failed,
    }
}

fn permissions(mechanisms: &[Mechanism]) -> Vec<Permission> {
    let mut found = Vec::new();
    for mechanism in mechanisms {
        if let Health::NeedsPermission(permission) = mechanism.health {
            if !found.contains(&permission) {
                found.push(permission);
            }
        }
    }
    found
}

fn needs(mechanisms: &[Mechanism]) -> String {
    let keys: Vec<&str> = permissions(mechanisms)
        .into_iter()
        .map(Permission::key)
        .collect();
    if keys.is_empty() {
        return NO_PERMISSION_NEEDED.to_string();
    }
    keys.join(PERMISSION_JOIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(health: Health) -> Mechanism {
        Mechanism::showing("notification history", health)
    }

    fn watch(health: Health) -> Mechanism {
        Mechanism::showing_and_hiding("banner watch", health)
    }

    fn switch(health: Health) -> Mechanism {
        Mechanism::hiding("desktop banner switch", health)
    }

    fn failed() -> Health {
        Health::Failed("gone".to_string())
    }

    #[test]
    fn the_outcome_says_what_happens_to_other_apps_notifications() {
        let disk = Health::NeedsPermission(Permission::FullDiskAccess);
        let access = Health::NeedsPermission(Permission::Accessibility);
        let cases = [
            (false, true, vec![], Outcome::Off, "none"),
            (true, true, vec![], Outcome::Unsupported, "none"),
            (
                true,
                true,
                vec![store(Health::Active), watch(Health::Active)],
                Outcome::Toasts,
                "none",
            ),
            (
                true,
                true,
                vec![store(Health::Active), watch(Health::Starting)],
                Outcome::Starting,
                "none",
            ),
            (
                true,
                true,
                vec![store(disk.clone()), watch(Health::Active)],
                Outcome::ToastsPartly,
                "full_disk_access",
            ),
            (
                true,
                true,
                vec![store(Health::Active), watch(access.clone())],
                Outcome::ToastsAndBanners,
                "accessibility",
            ),
            (
                true,
                true,
                vec![store(disk.clone()), watch(access.clone())],
                Outcome::Blocked,
                "full_disk_access_and_accessibility",
            ),
            (
                true,
                true,
                vec![store(failed()), watch(failed())],
                Outcome::Failed,
                "none",
            ),
            (
                true,
                true,
                vec![switch(Health::Active)],
                Outcome::Hidden,
                "none",
            ),
            (
                true,
                false,
                vec![store(Health::Active), watch(Health::Active)],
                Outcome::Muted,
                "none",
            ),
            (
                true,
                false,
                vec![store(Health::Active), watch(access)],
                Outcome::ToastsAndBanners,
                "accessibility",
            ),
            (true, true, vec![switch(failed())], Outcome::Failed, "none"),
        ];
        for (wanted, notifications_on, mechanisms, value, needs) in cases {
            let state = TakeoverState::new(wanted, notifications_on, mechanisms);
            assert_eq!(state.value, value, "{}", state.describe());
            assert_eq!(state.needs, needs, "{}", state.describe());
        }
    }

    #[test]
    fn the_state_survives_the_trip_to_the_doctor() {
        let state = TakeoverState::new(
            true,
            true,
            vec![
                store(Health::NeedsPermission(Permission::FullDiskAccess)),
                watch(failed()),
            ],
        );
        let wire = serde_json::to_string(&state).unwrap();
        assert!(wire.contains(r#""value":"blocked""#), "{wire}");
        assert_eq!(serde_json::from_str::<TakeoverState>(&wire).unwrap(), state);
    }
}
