#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    Working,
    Service,
    YourTurn,
    NeedsYou,
    Unknown,
    Acknowledged,
}

impl Status {
    pub fn is_attention(self) -> bool {
        matches!(self, Status::NeedsYou | Status::YourTurn)
    }
}
