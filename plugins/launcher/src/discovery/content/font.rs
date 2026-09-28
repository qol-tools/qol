use std::fs::File;
use std::path::Path;

use super::bytes::{be16, be32, read_at};

const DIRECTORY: usize = 4096;
const RECORD: usize = 16;

pub fn glyphs(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let mut head = read_at(&mut file, 0, DIRECTORY)?;
    if head.get(..4)? == b"ttcf" {
        let first = u64::from(be32(&head, 12)?);
        head = read_at(&mut file, first, DIRECTORY)?;
    }
    let tables = usize::from(be16(&head, 4)?);
    let maxp = (0..tables)
        .map(|table| 12 + table * RECORD)
        .find(|at| head.get(*at..*at + 4) == Some(b"maxp"))?;
    let offset = u64::from(be32(&head, maxp + 8)?);
    let count = be16(&read_at(&mut file, offset, 6)?, 4)?;
    (count > 0).then_some(u64::from(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(glyphs: u16) -> Vec<u8> {
        let mut data = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        for (tag, offset) in [(b"head", 0u32), (b"maxp", 44)] {
            data.extend_from_slice(tag);
            data.extend_from_slice(&[0; 4]);
            data.extend_from_slice(&offset.to_be_bytes());
            data.extend_from_slice(&6u32.to_be_bytes());
        }
        data.extend_from_slice(&[0, 0, 0x50, 0]);
        data.extend_from_slice(&glyphs.to_be_bytes());
        data
    }

    #[test]
    fn fonts_count_glyphs_in_the_maxp_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("Roboto-Regular.ttf");
        std::fs::write(&path, font(1294)).unwrap();
        assert_eq!(glyphs(&path), Some(1294));
        let collection = dir.path().join("Noto.ttc");
        let mut data = b"ttcf\0\x01\0\0\0\0\0\x01\0\0\0\x10".to_vec();
        data.extend(font(3000).into_iter().enumerate().map(|(at, byte)| {
            if at == 12 + 16 + 11 {
                44 + 16
            } else {
                byte
            }
        }));
        std::fs::write(&collection, data).unwrap();
        assert_eq!(glyphs(&collection), Some(3000));
    }
}
