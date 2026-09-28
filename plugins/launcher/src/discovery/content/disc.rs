use std::fs::File;
use std::path::Path;

use super::bytes::read_at;

const SECTOR: usize = 2048;
const VOLUME_START: u64 = 16 * SECTOR as u64;
const BOOT_SYSTEM: &[u8] = b"EL TORITO SPECIFICATION";

pub fn bootable(path: &Path) -> bool {
    let Some(sectors) = File::open(path)
        .ok()
        .and_then(|mut file| read_at(&mut file, VOLUME_START, 2 * SECTOR))
    else {
        return false;
    };
    sectors.get(1..6) == Some(b"CD001")
        && sectors.get(SECTOR) == Some(&0)
        && sectors.get(SECTOR + 1..SECTOR + 6) == Some(b"CD001")
        && sectors.get(SECTOR + 7..SECTOR + 7 + BOOT_SYSTEM.len()) == Some(BOOT_SYSTEM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discs_with_an_el_torito_record_are_bootable() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut image = vec![0; VOLUME_START as usize + 2 * SECTOR];
        let volume = VOLUME_START as usize;
        image[volume] = 1;
        image[volume + 1..volume + 6].copy_from_slice(b"CD001");
        let path = dir.path().join("data.iso");
        std::fs::write(&path, &image).unwrap();
        assert!(!bootable(&path));
        image[volume + SECTOR + 1..volume + SECTOR + 6].copy_from_slice(b"CD001");
        image[volume + SECTOR + 7..volume + SECTOR + 7 + BOOT_SYSTEM.len()]
            .copy_from_slice(BOOT_SYSTEM);
        let path = dir.path().join("recovery.iso");
        std::fs::write(&path, &image).unwrap();
        assert!(bootable(&path));
    }
}
