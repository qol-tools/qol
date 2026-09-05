#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Status {
    Working,
    Service,
    Coordinating,
    AwaitingReview,
    YourTurn,
    NeedsYou,
    #[default]
    Unknown,
    Acknowledged,
}

pub struct StateDefinition {
    pub label: &'static str,
    pub priority: u8,
    pub attention: bool,
    pub idle: bool,
    pub colors: fn(&qol_gpui::theme::CliSessionsPalette) -> (u32, u32),
}

impl StateDefinition {
    fn new(
        label: &'static str,
        priority: u8,
        attention: bool,
        idle: bool,
        colors: fn(&qol_gpui::theme::CliSessionsPalette) -> (u32, u32),
    ) -> Self {
        Self {
            label,
            priority,
            attention,
            idle,
            colors,
        }
    }
}

impl Status {
    pub const ALL: [Self; 8] = [
        Self::NeedsYou,
        Self::YourTurn,
        Self::Working,
        Self::Coordinating,
        Self::AwaitingReview,
        Self::Service,
        Self::Acknowledged,
        Self::Unknown,
    ];

    pub fn definition(self) -> StateDefinition {
        match self {
            Self::NeedsYou => StateDefinition::new("needs you", 0, true, false, |p| {
                (p.needs_you, p.needs_you_tint_rgba)
            }),
            Self::YourTurn => StateDefinition::new("your turn", 1, true, false, |p| {
                (p.your_turn, p.your_turn_tint_rgba)
            }),
            Self::Working => StateDefinition::new("working", 2, false, false, |p| {
                (p.working, p.working_tint_rgba)
            }),
            Self::Coordinating => {
                StateDefinition::new("coordinating agents", 3, false, false, |p| {
                    (p.bridged, p.bridged_tint_rgba)
                })
            }
            Self::AwaitingReview => {
                StateDefinition::new("awaiting agent review", 4, false, false, |p| {
                    (p.bridged, p.bridged_tint_rgba)
                })
            }
            Self::Service => StateDefinition::new("live", 5, false, false, |p| {
                (p.service, p.service_tint_rgba)
            }),
            Self::Acknowledged => StateDefinition::new("acknowledged", 6, false, true, |p| {
                (p.unknown, p.transparent_rgba)
            }),
            Self::Unknown => {
                StateDefinition::new("idle", 7, false, true, |p| (p.unknown, p.transparent_rgba))
            }
        }
    }

    pub fn priority(self, bridged: bool) -> u8 {
        let priority = self.definition().priority;
        if bridged {
            return priority.min(Self::AwaitingReview.definition().priority);
        }
        priority
    }

    pub fn is_attention(self) -> bool {
        self.definition().attention
    }
}

pub fn bridge_status(status: Status, bridged: bool, driving: bool) -> Status {
    if status == Status::NeedsYou {
        return status;
    }
    if driving {
        return Status::Coordinating;
    }
    if bridged
        && matches!(
            status,
            Status::YourTurn | Status::Acknowledged | Status::AwaitingReview
        )
    {
        return Status::AwaitingReview;
    }
    status
}
