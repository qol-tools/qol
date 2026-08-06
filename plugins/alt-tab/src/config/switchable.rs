use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwitchablePanelOverride {
    pub app: String,
    #[serde(default)]
    pub switchable: bool,
}
