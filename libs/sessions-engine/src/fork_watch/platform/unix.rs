use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const SYSTEM: u32 = 0;
const WORLD_WRITABLE: u32 = 0o002;
const GROUP_WRITABLE: u32 = 0o020;

pub(in crate::fork_watch) struct Keeper {
    uid: u32,
    gid: u32,
}

pub(in crate::fork_watch) fn keeper_of(root: &Path) -> Option<Keeper> {
    let meta = fs::metadata(root).ok()?;
    Some(Keeper {
        uid: meta.uid(),
        gid: meta.gid(),
    })
}

pub(in crate::fork_watch) fn is_users(path: &Path, keeper: &Keeper) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        let mode = meta.mode();
        meta.uid() == keeper.uid
            && mode & WORLD_WRITABLE == 0
            && (mode & GROUP_WRITABLE == 0 || meta.gid() == keeper.gid)
    })
}

pub(in crate::fork_watch) fn is_trusted_tool(path: &Path, keeper: &Keeper) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        (meta.uid() == keeper.uid || meta.uid() == SYSTEM) && meta.mode() & WORLD_WRITABLE == 0
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn a_folder_anyone_can_write_is_never_kept() {
        let folder = tempfile::tempdir().unwrap();
        let keeper = keeper_of(folder.path()).unwrap();
        let open = folder.path().join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_users(&open, &keeper) && is_trusted_tool(&open, &keeper));
        fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!is_users(&open, &keeper) && !is_trusted_tool(&open, &keeper));
        let stranger = Keeper {
            uid: keeper.uid.wrapping_add(1),
            gid: keeper.gid,
        };
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_users(&open, &stranger));
    }
}
