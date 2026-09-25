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
    pub colors: fn(&qol_gpui::kit::Kit) -> (u32, u32),
}

impl StateDefinition {
    fn new(
        label: &'static str,
        priority: u8,
        attention: bool,
        idle: bool,
        colors: fn(&qol_gpui::kit::Kit) -> (u32, u32),
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
        Self::AwaitingReview,
        Self::Coordinating,
        Self::Working,
        Self::Service,
        Self::Unknown,
        Self::Acknowledged,
    ];

    pub fn definition(self) -> StateDefinition {
        match self {
            Self::NeedsYou => StateDefinition::new("needs you", 0, true, false, |k| {
                (k.palette.danger, k.washes.halo_invalid.packed())
            }),
            Self::YourTurn => StateDefinition::new("your turn", 1, true, false, |k| {
                (k.palette.warning, k.washes.halo_attention.packed())
            }),
            Self::Working => StateDefinition::new("working", 4, false, false, |k| {
                (k.palette.success, k.washes.halo_success.packed())
            }),
            Self::Coordinating => {
                StateDefinition::new("coordinating agents", 3, false, false, |k| {
                    (k.palette.info, info_halo(k))
                })
            }
            Self::AwaitingReview => {
                StateDefinition::new("awaiting agent review", 2, false, false, |k| {
                    (k.palette.info, info_halo(k))
                })
            }
            Self::Service => {
                StateDefinition::new("live", 5, false, false, |k| (k.palette.info, info_halo(k)))
            }
            Self::Acknowledged => StateDefinition::new("acknowledged", 7, false, true, |k| {
                (k.grounds.pane.faint, 0)
            }),
            Self::Unknown => {
                StateDefinition::new("idle", 6, false, true, |k| (k.grounds.pane.faint, 0))
            }
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Working | Self::Coordinating | Self::Service)
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

fn info_halo(kit: &qol_gpui::kit::Kit) -> u32 {
    qol_gpui::theme::translucent(kit.palette.info, qol_gpui::theme::Alpha::Halo)
}
