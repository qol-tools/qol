use super::PlatformInspection;

pub(crate) fn inspect() -> PlatformInspection {
    PlatformInspection {
        name: "Windows",
        supported: false,
        inventory_roots: Vec::new(),
        trash: None,
        trash_creation_anchor: None,
    }
}
