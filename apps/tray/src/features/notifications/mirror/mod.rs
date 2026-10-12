mod platform;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorAccess {
    Unsupported,
    Readable,
    Unreadable(String),
}

pub(super) fn sync(enabled: bool) {
    platform::sync(enabled);
}

pub(super) fn access() -> MirrorAccess {
    platform::access()
}
