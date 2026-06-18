#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    Working,
    YourTurn,
    NeedsYou,
    Unknown,
    Acknowledged,
}
