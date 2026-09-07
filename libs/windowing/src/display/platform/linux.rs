use std::path::Path;

use sha2::{Digest, Sha256};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr;
use x11rb::protocol::xproto;

use super::DisplayEnumerator;
use crate::display::{DisplayError, DisplayHandle};

const BASE_EDID_BYTES: usize = 128;

pub struct Platform;

impl DisplayEnumerator for Platform {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        match enumerate_randr() {
            Ok(handles) if !handles.is_empty() => Ok(handles),
            _ => enumerate_from(Path::new("/sys/class/drm")),
        }
    }
}

fn enumerate_from(root: &Path) -> Result<Vec<DisplayHandle>, DisplayError> {
    let mut handles = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(connector) = connector_from_sys_name(&name) else {
            continue;
        };
        let connected = std::fs::read_to_string(entry.path().join("status"))
            .map(|status| status.trim() == "connected")
            .unwrap_or(false);
        if !connected {
            continue;
        }
        let (id, edid_sha256, identity_unstable) = identity_from(
            connector.as_str(),
            std::fs::read(entry.path().join("edid")).ok().as_deref(),
        );
        handles.push(DisplayHandle::new(
            id,
            connector,
            edid_sha256,
            identity_unstable,
        ));
    }
    handles.sort_by(|a, b| a.connector().cmp(b.connector()));
    Ok(handles)
}

fn identity_from(connector: &str, edid: Option<&[u8]>) -> (String, Option<[u8; 32]>, bool) {
    match edid {
        Some(base) => {
            let base = &base[..base.len().min(BASE_EDID_BYTES)];
            let digest: [u8; 32] = Sha256::digest(base).into();
            let mut hasher = Sha256::new();
            hasher.update(connector.as_bytes());
            hasher.update(base);
            let bound: [u8; 32] = hasher.finalize().into();
            (hex(&bound), Some(digest), false)
        }
        None => (
            hex(&Sha256::digest(connector.as_bytes()).into()),
            None,
            true,
        ),
    }
}

fn enumerate_randr() -> Result<Vec<DisplayHandle>, DisplayError> {
    let (conn, screen) = x11rb::connect(None).map_err(randr_failed)?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .map(|root| root.root)
        .ok_or(DisplayError::UnsupportedPlatform)?;
    let resources = randr::get_screen_resources_current(&conn, root)
        .map_err(randr_failed)?
        .reply()
        .map_err(randr_failed)?;
    let edid_atom = xproto::intern_atom(&conn, false, b"EDID")
        .map_err(randr_failed)?
        .reply()
        .map_err(randr_failed)?
        .atom;
    let mut handles = Vec::new();
    for output in resources.outputs {
        let output_info = randr::get_output_info(&conn, output, resources.config_timestamp)
            .map_err(randr_failed)?
            .reply()
            .map_err(randr_failed)?;
        if output_info.connection != randr::Connection::CONNECTED || output_info.crtc == 0 {
            continue;
        }
        let connector = String::from_utf8_lossy(&output_info.name).into_owned();
        let property = randr::get_output_property(
            &conn,
            output,
            edid_atom,
            xproto::AtomEnum::ANY,
            0,
            32,
            false,
            false,
        )
        .map_err(randr_failed)?
        .reply()
        .map_err(randr_failed)?;
        let edid = property.data;
        let base = (!edid.is_empty()).then_some(edid.as_slice());
        let (id, edid_sha256, identity_unstable) = identity_from(&connector, base);
        handles.push(DisplayHandle::new(
            id,
            connector,
            edid_sha256,
            identity_unstable,
        ));
    }
    handles.sort_by(|a, b| a.connector().cmp(b.connector()));
    Ok(handles)
}

fn randr_failed(error: impl std::fmt::Display) -> DisplayError {
    DisplayError::Io(std::io::Error::other(error.to_string()))
}

fn connector_from_sys_name(name: &str) -> Option<String> {
    let (card, connector) = name.split_once('-')?;
    if !card.starts_with("card") {
        return None;
    }
    (!connector.is_empty()).then(|| name.to_string())
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn identity_helper_matches_enumerate_from_construction() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();
        let edid = vec![0x7eu8; 200];
        fs::write(connector_dir.join("edid"), &edid).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        let (id, edid_sha256, identity_unstable) = identity_from("card0-DP-1", Some(&edid));
        assert_eq!(handles[0].id(), id);
        assert_eq!(handles[0].edid_sha256(), edid_sha256);
        assert_eq!(handles[0].identity_unstable(), identity_unstable);
        assert!(!identity_unstable);
    }

    #[test]
    fn identity_helper_matches_enumerate_from_without_edid() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        let (id, edid_sha256, identity_unstable) = identity_from("card0-DP-1", None);
        assert_eq!(handles[0].id(), id);
        assert_eq!(handles[0].edid_sha256(), edid_sha256);
        assert_eq!(handles[0].identity_unstable(), identity_unstable);
        assert!(identity_unstable);
        assert_eq!(edid_sha256, None);
    }

    #[test]
    fn connector_from_sys_name_keeps_card_prefix() {
        assert_eq!(
            connector_from_sys_name("card0-DP-1").as_deref(),
            Some("card0-DP-1")
        );
        assert_eq!(
            connector_from_sys_name("card1-HDMI-A-1").as_deref(),
            Some("card1-HDMI-A-1")
        );
        assert_eq!(
            connector_from_sys_name("card0-eDP-1").as_deref(),
            Some("card0-eDP-1")
        );
        assert_eq!(connector_from_sys_name("card0"), None);
        assert_eq!(connector_from_sys_name("card0-"), None);
    }

    #[test]
    fn enumerates_connected_connectors_with_edid() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();
        let edid = vec![0x42; 128];
        fs::write(connector_dir.join("edid"), &edid).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].connector(), "card0-DP-1");
        assert!(!handles[0].identity_unstable());
        let digest: [u8; 32] = Sha256::digest(&edid).into();
        assert_eq!(handles[0].edid_sha256(), Some(digest));
        let mut hasher = Sha256::new();
        hasher.update(b"card0-DP-1");
        hasher.update(&edid);
        let bound: [u8; 32] = hasher.finalize().into();
        assert_eq!(handles[0].id(), hex(&bound));
    }

    #[test]
    fn identical_edids_on_different_connectors_diverge() {
        let dir = tempdir().unwrap();
        for name in ["card0-DP-1", "card0-DP-2"] {
            let connector_dir = dir.path().join(name);
            fs::create_dir(&connector_dir).unwrap();
            fs::write(connector_dir.join("status"), "connected\n").unwrap();
            fs::write(connector_dir.join("edid"), [0x42; 128]).unwrap();
        }

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0].edid_sha256(), handles[1].edid_sha256());
        assert_ne!(handles[0].id(), handles[1].id());
    }

    #[test]
    fn identical_edids_on_different_cards_diverge() {
        let dir = tempdir().unwrap();
        for name in ["card0-DP-1", "card1-DP-1"] {
            let connector_dir = dir.path().join(name);
            fs::create_dir(&connector_dir).unwrap();
            fs::write(connector_dir.join("status"), "connected\n").unwrap();
            fs::write(connector_dir.join("edid"), [0x42; 128]).unwrap();
        }

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0].edid_sha256(), handles[1].edid_sha256());
        assert_ne!(handles[0].id(), handles[1].id());
    }

    #[test]
    fn identity_ignores_edid_extension_blocks() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();
        fs::write(connector_dir.join("edid"), [0x42; 256]).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        let digest: [u8; 32] = Sha256::digest([0x42; 128]).into();
        assert_eq!(handles[0].edid_sha256(), Some(digest));
        let mut hasher = Sha256::new();
        hasher.update(b"card0-DP-1");
        hasher.update([0x42; 128]);
        assert_eq!(handles[0].id(), hex(&hasher.finalize().into()));
    }

    #[test]
    fn unreadable_edid_marks_identity_unstable() {
        let dir = tempdir().unwrap();
        let connector_dir = dir.path().join("card0-DP-1");
        fs::create_dir(&connector_dir).unwrap();
        fs::write(connector_dir.join("status"), "connected\n").unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].connector(), "card0-DP-1");
        assert!(handles[0].identity_unstable());
        assert_eq!(handles[0].edid_sha256(), None);
        let digest: [u8; 32] = Sha256::digest(b"card0-DP-1").into();
        assert_eq!(handles[0].id(), hex(&digest));
    }

    #[test]
    fn skips_disconnected_and_non_connector_entries() {
        let dir = tempdir().unwrap();
        let connected = dir.path().join("card0-HDMI-A-1");
        fs::create_dir(&connected).unwrap();
        fs::write(connected.join("status"), "connected\n").unwrap();
        fs::write(connected.join("edid"), [0u8; 128]).unwrap();
        let disconnected = dir.path().join("card0-DP-2");
        fs::create_dir(&disconnected).unwrap();
        fs::write(disconnected.join("status"), "disconnected\n").unwrap();
        fs::create_dir(dir.path().join("card0")).unwrap();
        fs::write(dir.path().join("card0").join("status"), "connected\n").unwrap();
        fs::write(dir.path().join("card0").join("edid"), [0u8; 128]).unwrap();

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].connector(), "card0-HDMI-A-1");
    }

    #[test]
    fn sorts_by_connector_name() {
        let dir = tempdir().unwrap();
        for (card, connector) in [("card0-DP-2", "DP-2"), ("card0-DP-1", "DP-1")] {
            let connector_dir = dir.path().join(card);
            fs::create_dir(&connector_dir).unwrap();
            fs::write(connector_dir.join("status"), "connected\n").unwrap();
            fs::write(connector_dir.join("edid"), [0u8; 128]).unwrap();
            let _ = connector;
        }

        let handles = enumerate_from(dir.path()).unwrap();
        assert_eq!(
            handles.iter().map(|h| h.connector()).collect::<Vec<_>>(),
            vec!["card0-DP-1", "card0-DP-2"]
        );
    }
}
