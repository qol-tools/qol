#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
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
