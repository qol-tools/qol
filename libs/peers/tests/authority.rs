use std::{
    process::Command,
    time::{Duration, SystemTime},
};

use qol_peers::service::PeerAuthority;
use qol_peers::{AuthorityLifetime, StoreRevision};

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

#[test]
fn session_probe() {
    if std::env::var_os("QOL_PEERS_SESSION_PROBE").is_none() {
        return;
    }
    let authority = PeerAuthority::session("session".into(), now()).unwrap();
    let clone = authority.clone();
    authority
        .rename(StoreRevision::INITIAL, "renamed".into())
        .unwrap();
    assert_eq!(clone.projection().unwrap().name, "renamed");
    assert_eq!(
        clone.projection().unwrap().lifetime,
        AuthorityLifetime::Session
    );
    drop(authority);
    drop(clone);
    assert_eq!(
        std::fs::read_dir(std::env::current_dir().unwrap())
            .unwrap()
            .count(),
        0
    );
    println!("authority-session-probe:empty");
}

#[test]
fn session_only_lifecycle_creates_no_credential_files() {
    let temporary = tempfile::tempdir().unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session_probe", "--nocapture"])
        .env("QOL_PEERS_SESSION_PROBE", "1")
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("authority-session-probe:empty"));
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod persistent {
    use std::{fs, os::unix::fs::symlink, path::Path, process::Command};

    use super::now;
    use qol_peers::service::{AuthorityError, PeerAuthority};

    #[test]
    fn writer_probe() {
        let Some(root) = std::env::var_os("QOL_PEERS_WRITER_PROBE_ROOT") else {
            return;
        };
        let expected = std::env::var("QOL_PEERS_WRITER_PROBE_EXPECT").unwrap();
        let result = PeerAuthority::open_persistent(Path::new(&root), now());
        if expected == "busy" {
            assert!(matches!(result, Err(AuthorityError::WriterBusy)));
            println!("authority-writer-probe:busy");
            return;
        }
        assert_eq!(result.unwrap().projection().unwrap().name, "local");
        println!("authority-writer-probe:open");
    }

    fn probe(root: &Path, expected: &str) {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "persistent::writer_probe", "--nocapture"])
            .env("QOL_PEERS_WRITER_PROBE_ROOT", root)
            .env("QOL_PEERS_WRITER_PROBE_EXPECT", expected)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "expected {expected}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout)
            .contains(&format!("authority-writer-probe:{expected}")));
    }

    #[test]
    fn canonical_alias_and_another_process_contend_until_last_clone_drops() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = temporary.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let root = parent.join("authority");
        let authority = PeerAuthority::create_persistent(&root, "local".into(), now()).unwrap();
        let clone = authority.clone();
        let alias = temporary.path().join("alias");
        symlink(&parent, &alias).unwrap();
        let alias = alias.join("authority");
        assert!(matches!(
            PeerAuthority::open_persistent(&alias, now()),
            Err(AuthorityError::WriterBusy)
        ));
        probe(&root, "busy");
        probe(&alias, "busy");
        drop(authority);
        probe(&alias, "busy");
        drop(clone);
        probe(&alias, "open");
        let reopened = PeerAuthority::open_persistent(&root, now()).unwrap();
        assert_eq!(reopened.projection().unwrap().name, "local");
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn persistence_is_explicitly_unsupported_without_creating_files() {
    use qol_peers::service::AuthorityError;
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    assert!(matches!(
        PeerAuthority::create_persistent(&root, "local".into(), now()),
        Err(AuthorityError::UnsupportedPlatform)
    ));
    assert!(matches!(
        PeerAuthority::open_persistent(&root, now()),
        Err(AuthorityError::UnsupportedPlatform)
    ));
    assert!(!root.exists());
}
