use std::path::{Path, PathBuf};

use super::paths::conversations_dir;

pub(super) const PART_CHARS: usize = 8000;
pub(super) const PART_ENCODED: usize = 9000;
pub(super) const PART_SLOTS: usize = 4;
const INSTRUCTION_FILES: [&str; 3] = ["CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"];
const MEMORY_INDEX: &str = "MEMORY.md";

struct Section {
    label: Option<PathBuf>,
    text: String,
}

fn read_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .filter(|text| !text.trim().is_empty())
}

fn ends_with_newline(text: String) -> String {
    if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    }
}

fn sections(cwd: &Path, config_home: &Path, switchboard_dir: Option<&Path>) -> Vec<Section> {
    let mut sections = vec![Section {
        label: None,
        text: format!(
            "Your project folder is {}. Its instructions and memory index follow; follow them as you would in your own session.\n\n",
            cwd.display()
        ),
    }];
    let mut ancestors: Vec<&Path> = cwd.ancestors().collect();
    ancestors.reverse();
    for dir in ancestors {
        if switchboard_dir.is_some_and(|own| own.starts_with(dir)) {
            continue;
        }
        for name in INSTRUCTION_FILES {
            let path = dir.join(name);
            if let Some(text) = read_text(&path) {
                sections.push(Section {
                    text: format!(
                        "Contents of {}:\n\n{}\n",
                        path.display(),
                        ends_with_newline(text)
                    ),
                    label: Some(path),
                });
            }
        }
    }
    let memory_dir = conversations_dir(config_home, cwd).join("memory");
    let index = memory_dir.join(MEMORY_INDEX);
    if let Some(text) = read_text(&index) {
        sections.push(Section {
            text: format!(
                "Contents of {} (your auto-memory index):\n\n{}\n",
                index.display(),
                ends_with_newline(text)
            ),
            label: Some(index.clone()),
        });
    }
    sections.push(Section {
        text: format!(
            "Your auto-memory folder is {}. Save a memory there as one file per fact, plus a one-line pointer to it in {MEMORY_INDEX}.\n",
            memory_dir.display()
        ),
        label: Some(memory_dir),
    });
    sections
}

fn encoded_len(text: &str) -> usize {
    serde_json::to_string(text).map_or(usize::MAX, |encoded| encoded.len() - 2)
}

fn fits(chars: usize, encoded: usize) -> bool {
    chars <= PART_CHARS && encoded <= PART_ENCODED
}

pub(super) fn split_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut chars = 0;
    let mut encoded = 0;
    for line in text.split_inclusive('\n') {
        let line_chars = line.chars().count();
        let line_encoded = encoded_len(line);
        if fits(chars + line_chars, encoded + line_encoded) {
            part.push_str(line);
            chars += line_chars;
            encoded += line_encoded;
            continue;
        }
        if !part.is_empty() {
            parts.push(std::mem::take(&mut part));
            chars = 0;
            encoded = 0;
        }
        if fits(line_chars, line_encoded) {
            part.push_str(line);
            chars = line_chars;
            encoded = line_encoded;
            continue;
        }
        for c in line.chars() {
            let width = encoded_len(c.encode_utf8(&mut [0; 4]));
            if !fits(chars + 1, encoded + width) {
                parts.push(std::mem::take(&mut part));
                chars = 0;
                encoded = 0;
            }
            part.push(c);
            chars += 1;
            encoded += width;
        }
    }
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

fn cut_notice(undelivered: &[&Path]) -> String {
    let files = undelivered
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "The rest of your project context was cut at the hook size limit. Read these yourself for what is missing: {files}\n"
    )
}

pub(super) fn parts(cwd: &Path, config_home: &Path, switchboard_dir: Option<&Path>) -> Vec<String> {
    let sections = sections(cwd, config_home, switchboard_dir);
    let text: String = sections
        .iter()
        .map(|section| section.text.as_str())
        .collect();
    let mut parts = split_parts(&text);
    if parts.len() <= PART_SLOTS {
        return parts;
    }
    parts.truncate(PART_SLOTS - 1);
    let delivered: usize = parts.iter().map(|part| part.chars().count()).sum();
    let mut end = 0;
    let mut undelivered = Vec::new();
    for section in &sections {
        end += section.text.chars().count();
        if end > delivered {
            if let Some(label) = &section.label {
                undelivered.push(label.as_path());
            }
        }
    }
    parts.push(cut_notice(&undelivered));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn the_chain_runs_from_the_outermost_ancestor_to_the_project() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("work");
        let project = outer.join("app");
        write(&outer.join("CLAUDE.md"), "outer rules");
        write(&project.join("CLAUDE.md"), "project rules");
        write(&project.join("CLAUDE.local.md"), "local rules");
        write(&project.join(".claude/CLAUDE.md"), "nested rules");
        let home = root.path().join("home");
        let text = parts(&project, &home, None).concat();
        let positions: Vec<usize> = [
            "outer rules",
            "project rules",
            "local rules",
            "nested rules",
        ]
        .iter()
        .map(|needle| text.find(needle).unwrap())
        .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert!(text.starts_with("Your project folder is "), "{text}");
    }

    #[test]
    fn ancestors_of_the_switchboard_folder_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        let shared = root.path().join("data");
        let own = shared.join("sessions/switchboard");
        let project = shared.join("app");
        write(&shared.join("CLAUDE.md"), "shared rules");
        write(&project.join("CLAUDE.md"), "project rules");
        fs::create_dir_all(&own).unwrap();
        let home = root.path().join("home");
        let text = parts(&project, &home, Some(&own)).concat();
        assert!(!text.contains("shared rules"), "{text}");
        assert!(text.contains("project rules"), "{text}");
        let without = parts(&project, &home, None).concat();
        assert!(without.contains("shared rules"), "{without}");
    }

    #[test]
    fn the_memory_index_and_the_save_instruction_come_last() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("app");
        fs::create_dir_all(&project).unwrap();
        let home = root.path().join("home");
        let memory = conversations_dir(&home, &project).join("memory");
        write(&memory.join("MEMORY.md"), "- [A](a.md) - first");
        let text = parts(&project, &home, None).concat();
        let index = text.find("- [A](a.md) - first").unwrap();
        let save = text.find("one file per fact").unwrap();
        assert!(index < save, "{text}");
        assert!(text.contains(&memory.display().to_string()), "{text}");
        assert!(text.ends_with("MEMORY.md.\n"), "{text}");
    }

    #[test]
    fn a_missing_memory_index_still_names_the_memory_folder() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("app");
        fs::create_dir_all(&project).unwrap();
        let text = parts(&project, &root.path().join("home"), None).concat();
        assert!(!text.contains("auto-memory index):"), "{text}");
        assert!(text.contains("one file per fact"), "{text}");
    }

    #[test]
    fn parts_split_on_line_boundaries_within_both_limits() {
        let line = format!("{}\n", "x".repeat(999));
        let text = line.repeat(20);
        let parts = split_parts(&text);
        assert!(parts.len() > 1);
        assert_eq!(parts.concat(), text);
        for part in &parts {
            assert!(part.chars().count() <= PART_CHARS);
            assert!(part.ends_with('\n'), "{}", part.len());
        }
    }

    #[test]
    fn escaped_length_counts_against_the_hook_output_limit() {
        let line = format!("{}\n", "\"".repeat(900));
        let text = line.repeat(10);
        for part in split_parts(&text) {
            assert!(encoded_len(&part) <= PART_ENCODED);
        }
        assert_eq!(split_parts(&text).concat(), text);
    }

    #[test]
    fn a_line_longer_than_a_part_is_chopped() {
        let text = "y".repeat(PART_CHARS * 2 + 5);
        let parts = split_parts(&text);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts.concat(), text);
        assert_eq!(parts[0].chars().count(), PART_CHARS);
    }

    #[test]
    fn more_than_four_parts_end_with_a_notice_naming_the_files_not_delivered() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("app");
        write(&project.join("CLAUDE.md"), &"a line of rules\n".repeat(600));
        write(
            &project.join("CLAUDE.local.md"),
            &"more rules\n".repeat(4000),
        );
        let home = root.path().join("home");
        let parts = parts(&project, &home, None);
        assert_eq!(parts.len(), PART_SLOTS);
        let notice = parts.last().unwrap();
        assert!(notice.contains("cut at the hook size limit"), "{notice}");
        assert!(
            notice.contains(&project.join("CLAUDE.local.md").display().to_string()),
            "{notice}"
        );
        assert!(
            !notice.contains(&project.join("CLAUDE.md").display().to_string()),
            "{notice}"
        );
    }

    #[test]
    fn a_small_context_is_one_part_and_later_slots_are_empty() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("app");
        fs::create_dir_all(&project).unwrap();
        let parts = parts(&project, &root.path().join("home"), None);
        assert_eq!(parts.len(), 1);
        assert!(parts.get(1).is_none());
    }
}
