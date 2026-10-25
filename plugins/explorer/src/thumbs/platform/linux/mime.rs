use std::fs;
use std::sync::OnceLock;

struct Glob {
    weight: u32,
    mime: String,
    pattern: String,
    exact: bool,
    case_sensitive: bool,
}

impl Glob {
    fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut fields = line.splitn(4, ':');
        let weight = fields.next()?.parse().ok()?;
        let mime = fields.next()?.to_string();
        let pattern = fields.next()?.to_string();
        let case_sensitive = fields
            .next()
            .is_some_and(|flags| flags.split(',').any(|flag| flag == "cs"));
        let suffix = pattern.strip_prefix('*').unwrap_or(&pattern);
        if suffix.contains(['*', '?', '[']) {
            return None;
        }
        let exact = !pattern.starts_with('*');
        let pattern = if case_sensitive {
            pattern
        } else {
            pattern.to_lowercase()
        };
        Some(Self {
            weight,
            mime,
            pattern,
            exact,
            case_sensitive,
        })
    }

    fn matches(&self, name: &str, lowered: &str) -> bool {
        let name = if self.case_sensitive { name } else { lowered };
        if self.exact {
            return name == self.pattern;
        }
        name.ends_with(&self.pattern[1..])
    }
}

pub(super) fn of(name: &str) -> Option<String> {
    static GLOBS: OnceLock<Vec<Glob>> = OnceLock::new();
    let globs = GLOBS.get_or_init(|| {
        super::data_dirs()
            .iter()
            .filter_map(|dir| fs::read_to_string(dir.join("mime/globs2")).ok())
            .flat_map(|text| text.lines().filter_map(Glob::parse).collect::<Vec<_>>())
            .collect()
    });
    best(globs, name)
}

fn best(globs: &[Glob], name: &str) -> Option<String> {
    let lowered = name.to_lowercase();
    globs
        .iter()
        .filter(|glob| glob.matches(name, &lowered))
        .max_by_key(|glob| {
            (
                glob.exact,
                glob.weight,
                glob.case_sensitive,
                glob.pattern.len(),
            )
        })
        .map(|glob| glob.mime.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heaviest_longest_glob_names_the_type() {
        let text = "\
# comment
50:video/webm:*.webm
50:application/gzip:*.gz
50:application/x-compressed-tar:*.tar.gz
80:image/png:*.png
50:text/x-makefile:makefile
50:text/x-c++src:*.C:cs
50:text/x-csrc:*.c
10:text/x-readme:README*
";
        let globs: Vec<Glob> = text.lines().filter_map(Glob::parse).collect();
        let cases = [
            ("clip.webm", Some("video/webm")),
            ("CLIP.WEBM", Some("video/webm")),
            ("a.tar.gz", Some("application/x-compressed-tar")),
            ("a.gz", Some("application/gzip")),
            ("photo.PNG", Some("image/png")),
            ("Makefile", Some("text/x-makefile")),
            ("main.C", Some("text/x-c++src")),
            ("main.c", Some("text/x-csrc")),
            ("README.md", None),
            ("noext", None),
        ];
        for (name, expected) in cases {
            assert_eq!(best(&globs, name).as_deref(), expected, "{name}");
        }
    }
}
