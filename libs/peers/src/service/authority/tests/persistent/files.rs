use std::{
    fs,
    io::Read,
    os::{
        fd::AsRawFd,
        unix::{
            fs::{symlink, MetadataExt, PermissionsExt},
            process::CommandExt,
        },
    },
    time::Duration,
};

use super::{now, persistent, revision, AuthorityError, PeerAuthority};

#[test]
fn private_modes_and_fixed_lock_inode_survive_mutations() {
    let (_temporary, root, authority) = persistent();
    assert_eq!(fs::metadata(&root).unwrap().mode() & 0o7777, 0o700);
    for name in ["writer.lock", "state.json"] {
        let metadata = fs::metadata(root.join(name)).unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o600, "{name}");
        assert_eq!(metadata.nlink(), 1, "{name}");
    }
    let lock = fs::metadata(root.join("writer.lock")).unwrap();
    for name in ["first", "second"] {
        authority.rename(revision(&authority), name.into()).unwrap();
    }
    let after = fs::metadata(root.join("writer.lock")).unwrap();
    assert_eq!((after.dev(), after.ino()), (lock.dev(), lock.ino()));
    drop(authority);
    assert!(root.join("writer.lock").is_file());
    let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
    assert_eq!(reopened.projection().unwrap().name, "second");
}

#[test]
fn reopen_outlasts_the_lock_copy_a_spawning_child_holds_until_exec() {
    let (_temporary, root, authority) = persistent();
    let (mut forked, signal) = std::io::pipe().unwrap();
    let spawner = std::thread::spawn(move || {
        let fd = signal.as_raw_fd();
        let mut command = std::process::Command::new("true");
        unsafe {
            command.pre_exec(move || {
                libc::write(fd, [1u8].as_ptr().cast(), 1);
                std::thread::sleep(Duration::from_millis(50));
                Ok(())
            });
        }
        let status = command.status().unwrap();
        drop(signal);
        status
    });
    forked.read_exact(&mut [0]).unwrap();
    drop(authority);
    let reopened = PeerAuthority::open_persistent(&root, now()).err();
    assert!(spawner.join().unwrap().success());
    assert_eq!(reopened, None);
}

#[test]
fn insecure_existing_modes_are_rejected_without_chmod_repair() {
    for (name, mode) in [
        ("", 0o755),
        ("state.json", 0o644),
        ("writer.lock", 0o660),
        ("state.json", 0o4600),
    ] {
        let (_temporary, root, authority) = persistent();
        drop(authority);
        let path = if name.is_empty() {
            root.clone()
        } else {
            root.join(name)
        };
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            matches!(
                PeerAuthority::open_persistent(&root, now()),
                Err(AuthorityError::UnsafeStore)
            ),
            "{name} {mode:o}"
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, mode, "{name}");
    }
}

#[test]
fn symlinks_hardlinks_and_nonregular_authority_files_are_rejected() {
    for name in ["state.json", "writer.lock"] {
        for kind in ["symlink", "hardlink", "directory", "fifo"] {
            let (temporary, root, authority) = persistent();
            drop(authority);
            let path = root.join(name);
            let saved = temporary.path().join("saved");
            fs::rename(&path, &saved).unwrap();
            match kind {
                "symlink" => symlink(&saved, &path).unwrap(),
                "hardlink" => fs::hard_link(&saved, &path).unwrap(),
                "directory" => fs::create_dir(&path).unwrap(),
                "fifo" => {
                    use std::os::unix::ffi::OsStrExt;
                    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
                other => panic!("unknown fixture {other}"),
            }
            assert!(
                matches!(
                    PeerAuthority::open_persistent(&root, now()),
                    Err(AuthorityError::UnsafeStore)
                ),
                "{name} {kind}"
            );
            assert!(saved.is_file());
        }
    }
    let (temporary, root, authority) = persistent();
    drop(authority);
    let alias = temporary.path().join("root-link");
    symlink(&root, &alias).unwrap();
    assert!(matches!(
        PeerAuthority::open_persistent(&alias, now()),
        Err(AuthorityError::UnsafeStore)
    ));
}

#[test]
fn replaced_root_cannot_redirect_a_live_writer() {
    let (temporary, root, authority) = persistent();
    let saved = temporary.path().join("saved");
    fs::rename(&root, &saved).unwrap();
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        authority.rename(revision(&authority), "redirected".into()),
        Err(AuthorityError::UnsafeStore)
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    assert!(saved.join("state.json").is_file());
}
