use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const REVERSE_READ_CHUNK: u64 = 64 * 1024;

pub(super) struct CompleteLine {
    pub end: u64,
    pub bytes: Vec<u8>,
}

pub(super) fn last_complete_line(path: &Path) -> Option<CompleteLine> {
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let last_newline = last_byte_position(&mut file, length, b'\n')?;
    let start = last_byte_position(&mut file, last_newline, b'\n').map_or(0, |newline| newline + 1);
    let size = usize::try_from(last_newline - start).ok()?;
    let mut bytes = vec![0; size];
    file.seek(SeekFrom::Start(start)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(CompleteLine {
        end: last_newline + 1,
        bytes,
    })
}

fn last_byte_position(file: &mut fs::File, end: u64, needle: u8) -> Option<u64> {
    let mut cursor = end;
    while cursor > 0 {
        let start = cursor.saturating_sub(REVERSE_READ_CHUNK);
        let size = usize::try_from(cursor - start).ok()?;
        let mut chunk = vec![0; size];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        if let Some(relative) = chunk.iter().rposition(|byte| *byte == needle) {
            return Some(start + relative as u64);
        }
        cursor = start;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::last_complete_line;

    fn read(path: &std::path::Path) -> Option<(u64, Vec<u8>)> {
        last_complete_line(path).map(|line| (line.end, line.bytes))
    }

    #[test]
    fn empty_file_has_no_complete_line() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("empty.jsonl");
        std::fs::write(&path, "").unwrap();
        assert_eq!(read(&path), None);
    }

    #[test]
    fn a_single_line_without_a_trailing_newline_is_incomplete() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("partial.jsonl");
        std::fs::write(&path, "half-written entry").unwrap();
        assert_eq!(read(&path), None);
    }

    #[test]
    fn a_trailing_partial_line_is_discarded_in_favor_of_the_last_complete_one() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("mixed.jsonl");
        std::fs::write(&path, "complete\nhalf-written entry").unwrap();
        assert_eq!(read(&path), Some((9, b"complete".to_vec())));
    }

    #[test]
    fn a_final_line_longer_than_the_read_window_still_reveals_the_line_before_it() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("long.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"ok\n").unwrap();
        let long_tail = vec![b'x'; 5 * 1024 * 1024];
        file.write_all(&long_tail).unwrap();
        assert_eq!(read(&path), Some((3, b"ok".to_vec())));
    }

    #[test]
    fn a_file_ending_in_newlines_reads_the_final_empty_line_as_complete() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("blank.jsonl");
        std::fs::write(&path, "a\n\n").unwrap();
        assert_eq!(read(&path), Some((3, Vec::new())));
        std::fs::write(&path, "\n").unwrap();
        assert_eq!(read(&path), Some((1, Vec::new())));
    }

    #[test]
    fn the_end_offset_marks_the_byte_past_the_complete_lines_newline() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("ends.jsonl");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        assert_eq!(read(&path), Some((8, b"two".to_vec())));
    }
}
