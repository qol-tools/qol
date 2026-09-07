use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const REVERSE_READ_CHUNK: u64 = 64 * 1024;

pub(super) fn complete_length(path: &Path) -> Option<u64> {
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    last_byte_position(&mut file, length, b'\n').map(|position| position + 1)
}

pub(super) fn latest_runtime(
    path: &Path,
    classify: impl Fn(&serde_json::Value) -> Option<super::CliRuntimeState>,
) -> super::CliRuntimeState {
    runtime_from_complete_lines(path, classify).unwrap_or_default()
}

fn runtime_from_complete_lines(
    path: &Path,
    classify: impl Fn(&serde_json::Value) -> Option<super::CliRuntimeState>,
) -> Option<super::CliRuntimeState> {
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut cursor = last_byte_position(&mut file, length, b'\n')?;
    let mut suffix = Vec::new();
    while cursor > 0 {
        let start = cursor.saturating_sub(REVERSE_READ_CHUNK);
        let mut chunk = vec![0; usize::try_from(cursor - start).ok()?];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&suffix);
        let lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        for line in lines[usize::from(start > 0)..].iter().rev() {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let value = serde_json::from_slice(line).ok()?;
            if let Some(runtime) = classify(&value) {
                return Some(runtime);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    None
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

    use super::complete_length;

    fn read(path: &std::path::Path) -> Option<u64> {
        complete_length(path)
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
        assert_eq!(read(&path), Some(9));
    }

    #[test]
    fn a_final_line_longer_than_the_read_window_still_reveals_the_line_before_it() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("long.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"ok\n").unwrap();
        let long_tail = vec![b'x'; 5 * 1024 * 1024];
        file.write_all(&long_tail).unwrap();
        assert_eq!(read(&path), Some(3));
    }

    #[test]
    fn a_file_ending_in_newlines_reads_the_final_empty_line_as_complete() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("blank.jsonl");
        std::fs::write(&path, "a\n\n").unwrap();
        assert_eq!(read(&path), Some(3));
        std::fs::write(&path, "\n").unwrap();
        assert_eq!(read(&path), Some(1));
    }

    #[test]
    fn the_end_offset_marks_the_byte_past_the_complete_lines_newline() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("ends.jsonl");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        assert_eq!(read(&path), Some(8));
    }
    #[test]
    fn runtime_scan_crosses_chunk_boundaries_and_ignores_partial_records() {
        use crate::cli::CliRuntimeState;
        let root = TempDir::new().unwrap();
        let path = root.path().join("runtime.jsonl");
        for size in [0, 65530, 65536, 131072] {
            let metadata = serde_json::json!({"padding": "x".repeat(size)});
            let text = format!("{{\"ready\":true}}\n{metadata}\n{{\"working\":true");
            std::fs::write(&path, text).unwrap();
            let actual = super::latest_runtime(&path, |value| {
                value.get("ready").map(|_| CliRuntimeState::Ready)
            });
            assert_eq!(actual, CliRuntimeState::Ready, "padding={size}");
        }
    }
}
