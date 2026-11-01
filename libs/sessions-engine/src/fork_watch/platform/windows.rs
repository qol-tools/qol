use std::path::Path;

pub(in crate::fork_watch) struct Keeper;

pub(in crate::fork_watch) fn keeper_of(root: &Path) -> Option<Keeper> {
    root.is_dir().then_some(Keeper)
}

pub(in crate::fork_watch) fn is_users(path: &Path, _: &Keeper) -> bool {
    path.exists()
}

pub(in crate::fork_watch) fn is_trusted_tool(path: &Path, _: &Keeper) -> bool {
    path.exists()
}
