#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    Working,
    Service,
    YourTurn,
    NeedsYou,
    Unknown,
    Acknowledged,
}
