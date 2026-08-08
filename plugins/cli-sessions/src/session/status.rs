#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Status {
    Working,
    Service,
    YourTurn,
    NeedsYou,
    #[default]
    Unknown,
    Acknowledged,
}

impl Status {
    pub fn is_attention(self) -> bool {
        matches!(self, Status::NeedsYou | Status::YourTurn)
    }
}
