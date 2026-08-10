use crate::{DiffError, DiffStatus, FileDiff, HeatLevel, Hunk, LineChange, LineKind, TokenSpan};

pub fn parse_patch(old_path: &str, new_path: &str, patch: &str) -> Result<FileDiff, DiffError> {
    if patch.trim().is_empty() {
        return Ok(FileDiff::empty());
    }
    if has_binary_marker(patch) {
        return Err(DiffError::Binary);
    }
    if has_conflict_marker(patch) {
        return Err(DiffError::Conflict);
    }
    if patch.contains('\u{FFFD}') {
        return Err(DiffError::Encoding);
    }
    Ok(FileDiff {
        old_path: old_path.to_string(),
        new_path: new_path.to_string(),
        status: detect_status(old_path, new_path, patch),
        hunks: parse_hunks(patch),
    })
}

pub fn apply_heat(diff: &mut FileDiff) {
    for hunk in &mut diff.hunks {
        let mut removed: Vec<usize> = Vec::new();
        let mut added: Vec<usize> = Vec::new();
        for idx in 0..hunk.lines.len() {
            hunk.lines[idx].token_spans.clear();
            match hunk.lines[idx].kind {
                LineKind::Removed => removed.push(idx),
                LineKind::Added => added.push(idx),
                LineKind::Context => {
                    pair_block(&mut hunk.lines, &removed, &added);
                    removed.clear();
                    added.clear();
                }
            }
        }
        pair_block(&mut hunk.lines, &removed, &added);
    }
}

fn pair_block(lines: &mut [LineChange], removed: &[usize], added: &[usize]) {
    for k in 0..removed.len().min(added.len()) {
        let old_text = lines[removed[k]].text.clone();
        let new_text = lines[added[k]].text.clone();
        let (old_spans, new_spans) = heat_spans(&old_text, &new_text);
        lines[removed[k]].token_spans = old_spans;
        lines[added[k]].token_spans = new_spans;
    }
}

fn heat_spans(old: &str, new: &str) -> (Vec<TokenSpan>, Vec<TokenSpan>) {
    let mut prefix = 0usize;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    let shorter = old.len().min(new.len());
    let mut suffix = 0usize;
    for (a, b) in old.chars().rev().zip(new.chars().rev()) {
        if a != b || prefix + suffix + a.len_utf8() > shorter {
            break;
        }
        suffix += a.len_utf8();
    }
    let mut old_spans = Vec::new();
    let mut new_spans = Vec::new();
    push_span(&mut old_spans, 0, prefix, HeatLevel::Cool);
    push_span(&mut new_spans, 0, prefix, HeatLevel::Cool);
    push_span(
        &mut old_spans,
        prefix,
        old.len() - prefix - suffix,
        HeatLevel::Hot,
    );
    push_span(
        &mut new_spans,
        prefix,
        new.len() - prefix - suffix,
        HeatLevel::Hot,
    );
    push_span(&mut old_spans, old.len() - suffix, suffix, HeatLevel::Cool);
    push_span(&mut new_spans, new.len() - suffix, suffix, HeatLevel::Cool);
    (old_spans, new_spans)
}

fn push_span(spans: &mut Vec<TokenSpan>, start: usize, len: usize, heat: HeatLevel) {
    if len > 0 {
        spans.push(TokenSpan { start, len, heat });
    }
}

fn has_binary_marker(patch: &str) -> bool {
    patch.lines().any(|l| {
        (l.starts_with("Binary files ") && l.ends_with(" differ"))
            || l.starts_with("GIT binary patch")
    })
}

fn has_conflict_marker(patch: &str) -> bool {
    patch
        .lines()
        .any(|l| l.starts_with("<<<<<<<") || l.starts_with("=======") || l.starts_with(">>>>>>>"))
}

fn detect_status(old_path: &str, new_path: &str, patch: &str) -> DiffStatus {
    if old_path == "/dev/null" {
        DiffStatus::Added
    } else if new_path == "/dev/null" {
        DiffStatus::Deleted
    } else if patch.contains("similarity index")
        && patch.contains("rename from")
        && patch.contains("rename to")
    {
        DiffStatus::Renamed
    } else {
        DiffStatus::Modified
    }
}

fn parse_hunks(patch: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    for line in patch.lines() {
        if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(line) {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            current = Some(Hunk {
                old_start,
                old_lines: old_count,
                new_start,
                new_lines: new_count,
                lines: Vec::new(),
            });
            old_no = old_start;
            new_no = new_start;
            continue;
        }
        let Some(hunk) = current.as_mut() else {
            continue;
        };
        match line.as_bytes().first() {
            Some(b' ') => {
                hunk.lines.push(line_change(
                    LineKind::Context,
                    &line[1..],
                    Some(old_no),
                    Some(new_no),
                ));
                old_no += 1;
                new_no += 1;
            }
            Some(b'+') => {
                hunk.lines
                    .push(line_change(LineKind::Added, &line[1..], None, Some(new_no)));
                new_no += 1;
            }
            Some(b'-') => {
                hunk.lines.push(line_change(
                    LineKind::Removed,
                    &line[1..],
                    Some(old_no),
                    None,
                ));
                old_no += 1;
            }
            Some(b'\\') => {}
            _ => {
                current = None;
            }
        }
    }
    if let Some(hunk) = current {
        hunks.push(hunk);
    }
    hunks
}

fn line_change(kind: LineKind, text: &str, old_no: Option<u32>, new_no: Option<u32>) -> LineChange {
    LineChange {
        kind,
        text: text.to_string(),
        token_spans: Vec::new(),
        old_line_no: old_no,
        new_line_no: new_no,
    }
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let mut parts = line.strip_prefix("@@")?.split_whitespace();
    let old_range = parts.next()?.strip_prefix('-')?;
    let new_range = parts.next()?.strip_prefix('+')?;
    let (old_start, old_count) = parse_range(old_range)?;
    let (new_start, new_count) = parse_range(new_range)?;
    Some((old_start, old_count, new_start, new_count))
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_heat, parse_patch};
    use crate::{DiffError, DiffStatus, FileDiff, HeatLevel, LineChange, LineKind, TokenSpan};

    const SIMPLE: &str = "\
diff --git a/app.rs b/app.rs
index 3cdd9dc..447dc16 100644
--- a/app.rs
+++ b/app.rs
@@ -1,5 +1,5 @@
 fn main() {
     println!(\"hello\");
-    let x = 1;
+    let x = 2;
     println!(\"done\");
 }
";

    const ADDED: &str = "\
diff --git a/added.rs b/added.rs
new file mode 100644
index 0000000..034fe1f
--- /dev/null
+++ b/added.rs
@@ -0,0 +1 @@
+pub fn new_fn() {}
";

    const DELETED: &str = "\
diff --git a/consts.rs b/consts.rs
deleted file mode 100644
index dbfb096..0000000
--- a/consts.rs
+++ /dev/null
@@ -1 +0,0 @@
-const OLD: u32 = 1;
";

    const RENAMED: &str = "\
diff --git a/rename-me.txt b/rename-target.txt
similarity index 75%
rename from rename-me.txt
rename to rename-target.txt
index 55c1833..5c72634 100644
--- a/rename-me.txt
+++ b/rename-target.txt
@@ -1,2 +1,3 @@
 old content
 second line
+changed
";

    const PURE_RENAME: &str = "\
diff --git a/orig.txt b/renamed.txt
similarity index 100%
rename from orig.txt
rename to renamed.txt
";

    const BINARY: &str = "\
diff --git a/blob.bin b/blob.bin
index 27501de..70718b0 100644
Binary files a/blob.bin and b/blob.bin differ
";

    const BINARY_PATCH: &str = "\
diff --git a/blob.bin b/blob.bin
index 27501de3bc2ac77fab14f04a5344bcb6bc4d3aeb..70718b0c28dfde0ef8efd89da413463523ce1446 100644
GIT binary patch
literal 14
VcmZQzWODNKa}0|7|4*UJ2ml#w1dadz

literal 11
ScmZQzWODNKa}0|7{|^8Ro&&}J
";

    const CONFLICT: &str = "\
diff --git a/f.txt b/f.txt
index 1e427e4..36a66d7 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,7 @@
 line1
<<<<<<< HEAD
 main change
=======
side change
>>>>>>> side
 line3
";

    const ENCODING: &str = "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1 +1 @@
-old
+new\u{FFFD}
";

    const NO_NEWLINE: &str = "\
diff --git a/f.txt b/f.txt
index d7b6fdc..75e11c0 100644
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
 line one
-no trailing newline
\\ No newline at end of file
+no trailing newline
";

    const MULTIHUNK: &str = "\
diff --git a/f.txt b/f.txt
index ac9837c..78be9d2 100644
--- a/f.txt
+++ b/f.txt
@@ -1,6 +1,6 @@
 line 1
 line 2
-line 3
+line THREE
 line 4
 line 5
 line 6
@@ -12,7 +12,7 @@ line 11
 line 12
 line 13
 line 14
-line 15
+line FIFTEEN
 line 16
 line 17
 line 18
@@ -25,6 +25,6 @@ line 24
 line 25
 line 26
 line 27
-line 28
+line TWENTY-EIGHT
 line 29
 line 30
";

    const UNPAIRED: &str = "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,2 @@
 a
-b
-c
+d
";

    const TWO_BLOCKS: &str = "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,4 +1,3 @@
-a
-b
+c
 x
-d
+e
";

    fn lines_of(diff: &FileDiff, hunk_idx: usize) -> &[LineChange] {
        &diff.hunks[hunk_idx].lines
    }

    #[test]
    fn simple_hunk_carries_line_numbers() {
        let diff = parse_patch("app.rs", "app.rs", SIMPLE).expect("parse");
        assert_eq!(diff.status, DiffStatus::Modified);
        assert_eq!(diff.old_path, "app.rs");
        assert_eq!(diff.new_path, "app.rs");
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines
            ),
            (1, 5, 1, 5)
        );
        let lines = lines_of(&diff, 0);
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].kind, LineKind::Context);
        assert_eq!(
            (lines[0].old_line_no, lines[0].new_line_no),
            (Some(1), Some(1))
        );
        assert_eq!(
            (lines[1].old_line_no, lines[1].new_line_no),
            (Some(2), Some(2))
        );
        assert_eq!(lines[2].kind, LineKind::Removed);
        assert_eq!(
            (lines[2].old_line_no, lines[2].new_line_no),
            (Some(3), None)
        );
        assert_eq!(lines[2].text, "    let x = 1;");
        assert_eq!(lines[3].kind, LineKind::Added);
        assert_eq!(
            (lines[3].old_line_no, lines[3].new_line_no),
            (None, Some(3))
        );
        assert_eq!(lines[3].text, "    let x = 2;");
        assert_eq!(
            (lines[4].old_line_no, lines[4].new_line_no),
            (Some(4), Some(4))
        );
    }

    #[test]
    fn added_file_parses_with_new_side_numbers() {
        let diff = parse_patch("/dev/null", "added.rs", ADDED).expect("parse");
        assert_eq!(diff.status, DiffStatus::Added);
        let hunk = &diff.hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines
            ),
            (0, 0, 1, 1)
        );
        let line = &hunk.lines[0];
        assert_eq!(line.kind, LineKind::Added);
        assert_eq!((line.old_line_no, line.new_line_no), (None, Some(1)));
        assert_eq!(line.text, "pub fn new_fn() {}");
    }

    #[test]
    fn deleted_file_parses_with_old_side_numbers() {
        let diff = parse_patch("consts.rs", "/dev/null", DELETED).expect("parse");
        assert_eq!(diff.status, DiffStatus::Deleted);
        let hunk = &diff.hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines
            ),
            (1, 1, 0, 0)
        );
        let line = &hunk.lines[0];
        assert_eq!(line.kind, LineKind::Removed);
        assert_eq!((line.old_line_no, line.new_line_no), (Some(1), None));
        assert_eq!(line.text, "const OLD: u32 = 1;");
    }

    #[test]
    fn renamed_file_detected_from_similarity_headers() {
        let diff = parse_patch("rename-me.txt", "rename-target.txt", RENAMED).expect("parse");
        assert_eq!(diff.status, DiffStatus::Renamed);
        assert_eq!(
            (diff.old_path.as_str(), diff.new_path.as_str()),
            ("rename-me.txt", "rename-target.txt")
        );
        let hunk = &diff.hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines
            ),
            (1, 2, 1, 3)
        );
        assert_eq!(lines_of(&diff, 0).len(), 3);
        assert_eq!(lines_of(&diff, 0)[2].kind, LineKind::Added);
        assert_eq!(lines_of(&diff, 0)[2].text, "changed");
    }

    #[test]
    fn pure_rename_keeps_status_without_hunks() {
        let diff = parse_patch("orig.txt", "renamed.txt", PURE_RENAME).expect("parse");
        assert_eq!(diff.status, DiffStatus::Renamed);
        assert!(diff.hunks.is_empty());
        assert!(diff.is_empty());
    }

    #[test]
    fn binary_files_differ_is_rejected() {
        assert_eq!(
            parse_patch("blob.bin", "blob.bin", BINARY),
            Err(DiffError::Binary)
        );
    }

    #[test]
    fn git_binary_patch_is_rejected() {
        assert_eq!(
            parse_patch("blob.bin", "blob.bin", BINARY_PATCH),
            Err(DiffError::Binary)
        );
    }

    #[test]
    fn conflict_markers_are_rejected() {
        assert_eq!(
            parse_patch("f.txt", "f.txt", CONFLICT),
            Err(DiffError::Conflict)
        );
    }

    #[test]
    fn empty_patch_is_the_empty_state() {
        assert_eq!(parse_patch("a.rs", "a.rs", ""), Ok(FileDiff::empty()));
        assert_eq!(
            parse_patch("a.rs", "a.rs", "   \n  "),
            Ok(FileDiff::empty())
        );
    }

    #[test]
    fn replacement_char_is_an_encoding_error() {
        assert_eq!(
            parse_patch("f.txt", "f.txt", ENCODING),
            Err(DiffError::Encoding)
        );
    }

    #[test]
    fn no_newline_marker_is_skipped() {
        let diff = parse_patch("f.txt", "f.txt", NO_NEWLINE).expect("parse");
        let lines = lines_of(&diff, 0);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "line one");
        assert_eq!(
            (lines[0].old_line_no, lines[0].new_line_no),
            (Some(1), Some(1))
        );
        assert_eq!(lines[1].kind, LineKind::Removed);
        assert_eq!(lines[1].text, "no trailing newline");
        assert_eq!(
            (lines[1].old_line_no, lines[1].new_line_no),
            (Some(2), None)
        );
        assert_eq!(lines[2].kind, LineKind::Added);
        assert_eq!(lines[2].text, "no trailing newline");
        assert_eq!(
            (lines[2].old_line_no, lines[2].new_line_no),
            (None, Some(2))
        );
    }

    #[test]
    fn multi_hunk_file_parses_every_hunk() {
        let diff = parse_patch("f.txt", "f.txt", MULTIHUNK).expect("parse");
        assert_eq!(diff.hunks.len(), 3);
        let first = &diff.hunks[0];
        assert_eq!(
            (
                first.old_start,
                first.old_lines,
                first.new_start,
                first.new_lines
            ),
            (1, 6, 1, 6)
        );
        let second = &diff.hunks[1];
        assert_eq!(
            (
                second.old_start,
                second.old_lines,
                second.new_start,
                second.new_lines
            ),
            (12, 7, 12, 7)
        );
        let third = &diff.hunks[2];
        assert_eq!(
            (
                third.old_start,
                third.old_lines,
                third.new_start,
                third.new_lines
            ),
            (25, 6, 25, 6)
        );
        assert_eq!(lines_of(&diff, 0).len(), 7);
        assert_eq!(lines_of(&diff, 1).len(), 8);
        assert_eq!(lines_of(&diff, 2).len(), 7);
        let removed = &lines_of(&diff, 1)[3];
        assert_eq!(removed.kind, LineKind::Removed);
        assert_eq!(removed.text, "line 15");
        assert_eq!(removed.old_line_no, Some(15));
        let added = &lines_of(&diff, 1)[4];
        assert_eq!(added.kind, LineKind::Added);
        assert_eq!(added.text, "line FIFTEEN");
        assert_eq!(added.new_line_no, Some(15));
        let removed = &lines_of(&diff, 2)[3];
        assert_eq!(removed.kind, LineKind::Removed);
        assert_eq!(removed.text, "line 28");
        assert_eq!(removed.old_line_no, Some(28));
        let added = &lines_of(&diff, 2)[4];
        assert_eq!(added.kind, LineKind::Added);
        assert_eq!(added.text, "line TWENTY-EIGHT");
        assert_eq!(added.new_line_no, Some(28));
    }

    #[test]
    fn heat_marks_changed_middle_between_cool_edges() {
        let mut diff = parse_patch("app.rs", "app.rs", SIMPLE).expect("parse");
        apply_heat(&mut diff);
        let lines = lines_of(&diff, 0);
        let removed = &lines[2];
        assert_eq!(
            removed.token_spans,
            vec![
                TokenSpan {
                    start: 0,
                    len: 12,
                    heat: HeatLevel::Cool
                },
                TokenSpan {
                    start: 12,
                    len: 1,
                    heat: HeatLevel::Hot
                },
                TokenSpan {
                    start: 13,
                    len: 1,
                    heat: HeatLevel::Cool
                },
            ]
        );
        let added = &lines[3];
        assert_eq!(
            added.token_spans,
            vec![
                TokenSpan {
                    start: 0,
                    len: 12,
                    heat: HeatLevel::Cool
                },
                TokenSpan {
                    start: 12,
                    len: 1,
                    heat: HeatLevel::Hot
                },
                TokenSpan {
                    start: 13,
                    len: 1,
                    heat: HeatLevel::Cool
                },
            ]
        );
        assert!(lines[0].token_spans.is_empty());
        assert!(lines[4].token_spans.is_empty());
    }

    #[test]
    fn unpaired_lines_get_empty_spans() {
        let mut diff = parse_patch("f.txt", "f.txt", UNPAIRED).expect("parse");
        apply_heat(&mut diff);
        let lines = lines_of(&diff, 0);
        assert_eq!(lines[0].kind, LineKind::Context);
        assert!(lines[0].token_spans.is_empty());
        let paired_removed = &lines[1];
        assert_eq!(paired_removed.text, "b");
        assert_eq!(
            paired_removed.token_spans,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot
            }]
        );
        let unpaired = &lines[2];
        assert_eq!(unpaired.text, "c");
        assert!(unpaired.token_spans.is_empty());
        let paired_added = &lines[3];
        assert_eq!(paired_added.text, "d");
        assert_eq!(
            paired_added.token_spans,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot
            }]
        );
    }

    #[test]
    fn pairing_restarts_at_each_changed_block() {
        let mut diff = parse_patch("f.txt", "f.txt", TWO_BLOCKS).expect("parse");
        apply_heat(&mut diff);
        let lines = lines_of(&diff, 0);
        assert_eq!(
            lines[0].token_spans,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot
            }]
        );
        assert_eq!(lines[1].text, "b");
        assert!(lines[1].token_spans.is_empty());
        assert_eq!(lines[2].text, "c");
        assert_eq!(
            lines[2].token_spans,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot
            }]
        );
        assert_eq!(lines[3].text, "x");
        assert!(lines[3].token_spans.is_empty());
        assert_eq!(
            lines[4].token_spans,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot
            }]
        );
        assert_eq!(
            lines[5].token_spans,
            vec![TokenSpan {
                start: 0,
                len: 1,
                heat: HeatLevel::Hot
            }]
        );
    }

    #[test]
    fn apply_heat_is_idempotent() {
        let mut once = parse_patch("app.rs", "app.rs", SIMPLE).expect("parse");
        apply_heat(&mut once);
        let twice = once.clone();
        let mut again = parse_patch("app.rs", "app.rs", SIMPLE).expect("parse");
        apply_heat(&mut again);
        apply_heat(&mut again);
        assert_eq!(once, twice);
        assert_eq!(again, twice);
    }

    #[test]
    fn multi_hunk_heat_stays_within_hunks() {
        let mut diff = parse_patch("f.txt", "f.txt", MULTIHUNK).expect("parse");
        apply_heat(&mut diff);
        for hunk in &diff.hunks {
            for line in &hunk.lines {
                if line.kind == LineKind::Context {
                    assert!(line.token_spans.is_empty());
                }
            }
        }
        let removed = &lines_of(&diff, 0)[2];
        assert_eq!(removed.text, "line 3");
        assert_eq!(
            removed.token_spans,
            vec![
                TokenSpan {
                    start: 0,
                    len: 5,
                    heat: HeatLevel::Cool
                },
                TokenSpan {
                    start: 5,
                    len: 1,
                    heat: HeatLevel::Hot
                },
            ]
        );
        let added = &lines_of(&diff, 0)[3];
        assert_eq!(added.text, "line THREE");
        assert_eq!(
            added.token_spans,
            vec![
                TokenSpan {
                    start: 0,
                    len: 5,
                    heat: HeatLevel::Cool
                },
                TokenSpan {
                    start: 5,
                    len: 5,
                    heat: HeatLevel::Hot
                },
            ]
        );
    }

    #[test]
    fn heat_handles_utf8_middle_offsets() {
        let patch = "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
-x\u{00E9}a
+x\u{00EA}a
";
        let mut diff = parse_patch("f.txt", "f.txt", patch).expect("parse");
        apply_heat(&mut diff);
        let lines = lines_of(&diff, 0);
        assert_eq!(lines[0].text, "x\u{00E9}a");
        assert_eq!(
            lines[0].token_spans,
            vec![
                TokenSpan {
                    start: 0,
                    len: 1,
                    heat: HeatLevel::Cool
                },
                TokenSpan {
                    start: 1,
                    len: 2,
                    heat: HeatLevel::Hot
                },
                TokenSpan {
                    start: 3,
                    len: 1,
                    heat: HeatLevel::Cool
                },
            ]
        );
        assert_eq!(lines[1].text, "x\u{00EA}a");
        assert_eq!(
            lines[1].token_spans,
            vec![
                TokenSpan {
                    start: 0,
                    len: 1,
                    heat: HeatLevel::Cool
                },
                TokenSpan {
                    start: 1,
                    len: 2,
                    heat: HeatLevel::Hot
                },
                TokenSpan {
                    start: 3,
                    len: 1,
                    heat: HeatLevel::Cool
                },
            ]
        );
    }
}
