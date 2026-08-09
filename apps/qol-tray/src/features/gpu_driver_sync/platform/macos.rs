use super::Observation;

pub(crate) fn watch_supported() -> bool {
    false
}

pub(crate) fn observe() -> Observation {
    Observation::Unsupported
}
