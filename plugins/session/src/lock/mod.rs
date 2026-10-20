mod platform;

pub use platform::{availability, lock_screen};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready(String),
    Missing { detail: String, fix: String },
}
