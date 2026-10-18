use std::path::{Path, PathBuf};

use super::{mountinfo, user_dirs};
use crate::places::{Mount, PlaceKind};

#[test]
fn user_dirs_resolve_home_relative_and_absolute_folders() {
    let home = Path::new("/home/me");
    let contents = r#"
# This file is written by xdg-user-dirs-update
XDG_DESKTOP_DIR="$HOME/Desktop"
XDG_DOWNLOAD_DIR="$HOME/Downloads"
XDG_TEMPLATES_DIR="$HOME/Templates"
XDG_PUBLICSHARE_DIR="$HOME/Public"
XDG_DOCUMENTS_DIR="$HOME/My \"Docs\""
XDG_MUSIC_DIR="/srv/music"
XDG_PICTURES_DIR="$HOME/"
XDG_VIDEOS_DIR=$HOME/Videos
"#;

    assert_eq!(
        user_dirs::parse(contents, home),
        [
            (PlaceKind::Desktop, home.join("Desktop")),
            (PlaceKind::Downloads, home.join("Downloads")),
            (PlaceKind::Documents, home.join("My \"Docs\"")),
            (PlaceKind::Music, PathBuf::from("/srv/music")),
            (PlaceKind::Pictures, home.to_path_buf()),
            (PlaceKind::Videos, home.join("Videos")),
        ]
    );
}

#[test]
fn user_dirs_skip_relative_values_and_malformed_lines() {
    let home = Path::new("/home/me");
    let contents = "XDG_DESKTOP_DIR=\"Desktop\"\nXDG_MUSIC_DIR\n  # XDG_VIDEOS_DIR=\"$HOME/Videos\"\nXDG_PICTURES_DIR=\"$HOMEPICS\"\n";

    assert!(user_dirs::parse(contents, home).is_empty());
}

fn mount(label: &str, path: &str) -> Mount {
    Mount {
        label: label.to_string(),
        path: PathBuf::from(path),
    }
}

#[test]
fn mountinfo_keeps_block_devices_on_drive_paths() {
    let contents = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw,errors=remount-ro
23 22 259:1 / /boot/efi rw,relatime shared:2 - vfat /dev/nvme0n1p1 rw
24 22 0:5 / /dev rw,nosuid shared:3 - devtmpfs udev rw
25 22 0:22 / /proc rw,nosuid shared:4 - proc proc rw
26 22 0:24 / /run rw,nosuid shared:5 - tmpfs tmpfs rw
27 22 7:0 / /snap/core/1 ro,nodev shared:6 - squashfs /dev/loop0 ro
28 22 259:5 / /media/me/WD_SN850X rw,nosuid shared:7 - ext4 /dev/nvme1n1p1 rw
29 22 8:17 / /run/media/me/USB\\040STICK rw,nosuid shared:8 - vfat /dev/sdb1 rw
30 22 8:33 / /mnt/backup rw,relatime shared:9 master:2 - btrfs /dev/sdc1 rw
31 22 0:50 / /mnt/share rw,relatime shared:10 - nfs server:/export rw
32 22 8:49 / /mediaplayer rw shared:11 - ext4 /dev/sdd1 rw
";

    assert_eq!(
        mountinfo::parse(contents),
        [
            mount("/", "/"),
            mount("WD_SN850X", "/media/me/WD_SN850X"),
            mount("USB STICK", "/run/media/me/USB STICK"),
            mount("backup", "/mnt/backup"),
        ]
    );
}

#[test]
fn mountinfo_lists_a_stacked_path_once_and_skips_short_lines() {
    let contents = "\
40 22 8:1 / /mnt/disk rw - ext4 /dev/sda1 rw
41 22 8:2 / /mnt/disk rw - ext4 /dev/sda2 rw
42 22 8:3 /
43 22 8:4 / /mnt/other rw - ext4
";

    assert_eq!(mountinfo::parse(contents), [mount("disk", "/mnt/disk")]);
}

#[test]
fn mountinfo_unescapes_octal_and_keeps_stray_backslashes() {
    let contents = "\
50 22 8:1 / /media/me/a\\011b\\134c rw - ext4 /dev/sda1 rw
51 22 8:2 / /media/me/odd\\9x rw - ext4 /dev/sda2 rw
";

    assert_eq!(
        mountinfo::parse(contents),
        [
            mount("a\tb\\c", "/media/me/a\tb\\c"),
            mount("odd\\9x", "/media/me/odd\\9x"),
        ]
    );
}
