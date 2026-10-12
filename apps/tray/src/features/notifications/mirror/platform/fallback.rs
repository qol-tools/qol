use super::super::MirrorAccess;

pub(in super::super) fn sync(_enabled: bool) {}

pub(in super::super) fn access() -> MirrorAccess {
    MirrorAccess::Unsupported
}
