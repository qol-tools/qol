use std::fs;
use std::io::ErrorKind;

use qol_desktop_settings::gsettings;

use super::privileged::run_script;
use super::READ;
use crate::updates::Refusal;

const SCHEMA: &str = "com.linuxmint.updates";
const KEY: &str = "blacklisted-packages";
const SYSTEM: &str = "/etc/mintupdate.blacklist";

const SYSTEM_SH: &str = r#"f=/etc/mintupdate.blacklist
default=/usr/share/linuxmint/mintupdate/automation/blacklist.default
if [ ! -e "$f" ]; then
  if [ -f "$default" ]; then cp "$default" "$f" || exit 3; else : > "$f" || exit 3; fi
fi
for change in "$@"; do
  entry=${change#?}
  case "$change" in
    +*) grep -qxF -- "$entry" "$f" || printf '%s\n' "$entry" >> "$f" || exit 3 ;;
    -*) kept=$(mktemp) || exit 3
        grep -vxF -- "$entry" "$f" > "$kept"
        [ "$?" -le 1 ] || { rm -f "$kept"; exit 3; }
        cat "$kept" > "$f" || exit 3
        rm -f "$kept" ;;
  esac
done
exit 0
"#;

pub(super) fn read() -> Vec<String> {
    let mut entries = gsettings::get_within(SCHEMA, KEY, READ)
        .map(|value| parse_strv(&value))
        .unwrap_or_default();
    for line in system_lines(&fs::read_to_string(SYSTEM).unwrap_or_default()) {
        if !entries.contains(&line) {
            entries.push(line);
        }
    }
    entries
}

pub(super) fn set(add: &[String], remove: &[String], system: bool) -> Result<(), Refusal> {
    let current = gsettings::get_within(SCHEMA, KEY, READ)
        .map(|value| parse_strv(&value))
        .map_err(|error| Refusal::Failed(format!("{error:#}")))?;
    let file = fs::read_to_string(SYSTEM);
    if system {
        let changes = system_changes(add, remove, file);
        if !changes.is_empty() {
            run_script(SYSTEM_SH, &changes)?;
        }
    } else {
        let lines = system_lines(&file.unwrap_or_default());
        let stuck = held_by_system(&lines, add, remove);
        if !stuck.is_empty() {
            return Err(Refusal::Failed(format!(
                "an administrator skips {} in {SYSTEM}, and only an administrator can change that",
                stuck.join(", ")
            )));
        }
    }
    let next = merged(&current, add, remove);
    if next == current {
        return Ok(());
    }
    gsettings::set_within(SCHEMA, KEY, &format_strv(&next), READ)
        .map_err(|error| Refusal::Failed(format!("{error:#}")))
}

fn system_changes(add: &[String], remove: &[String], file: std::io::Result<String>) -> Vec<String> {
    let lines = match file {
        Ok(text) => Some(system_lines(&text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(Vec::new()),
        Err(_) => None,
    };
    let known = |entry: &String| lines.as_ref().map(|lines| lines.contains(entry));
    add.iter()
        .filter(|entry| known(entry) != Some(true))
        .map(|entry| format!("+{entry}"))
        .chain(
            remove
                .iter()
                .filter(|entry| known(entry) != Some(false))
                .map(|entry| format!("-{entry}")),
        )
        .collect()
}

fn held_by_system<'a>(lines: &[String], add: &[String], remove: &'a [String]) -> Vec<&'a str> {
    let covered = |entry: &String| {
        let source = entry
            .split_once('=')
            .map_or(entry.as_str(), |(source, _)| source);
        add.iter().any(|added| added == entry || added == source)
    };
    remove
        .iter()
        .filter(|entry| lines.contains(entry) && !covered(entry))
        .map(String::as_str)
        .collect()
}

pub(super) fn merged(current: &[String], add: &[String], remove: &[String]) -> Vec<String> {
    let mut next: Vec<String> = current
        .iter()
        .filter(|entry| !remove.contains(entry))
        .cloned()
        .collect();
    for entry in add {
        if !next.contains(entry) {
            next.push(entry.clone());
        }
    }
    next
}

pub(super) fn system_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect()
}

pub(super) fn parse_strv(text: &str) -> Vec<String> {
    let Some(start) = text.find('[') else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut characters = text[start + 1..].chars();
    while let Some(character) = characters.next() {
        if character == ']' {
            break;
        }
        if character != '\'' && character != '"' {
            continue;
        }
        let mut entry = String::new();
        while let Some(next) = characters.next() {
            match next {
                '\\' => entry.extend(characters.next()),
                quote if quote == character => break,
                other => entry.push(other),
            }
        }
        entries.push(entry);
    }
    entries
}

pub(super) fn format_strv(entries: &[String]) -> String {
    if entries.is_empty() {
        return "@as []".into();
    }
    let quoted: Vec<String> = entries
        .iter()
        .map(|entry| format!("'{}'", entry.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[&str]) -> Vec<String> {
        entries.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn mints_list_reads_and_writes_as_gsettings_prints_it() {
        let text = "['google-chrome-stable', 'brave-browser', \"it's\", 'ulauncher=6.0.0~beta30-0ubuntu1ppa1~noble']\n";
        let entries = parse_strv(text);
        assert_eq!(
            entries,
            list(&[
                "google-chrome-stable",
                "brave-browser",
                "it's",
                "ulauncher=6.0.0~beta30-0ubuntu1ppa1~noble",
            ])
        );
        assert_eq!(parse_strv(&format_strv(&entries)), entries);
        assert_eq!(parse_strv("@as []"), Vec::<String>::new());
        assert_eq!(format_strv(&[]), "@as []");
        assert_eq!(format_strv(&list(&["a'b"])), r"['a\'b']");
    }

    #[test]
    fn the_system_list_skips_comments_and_changes_only_what_differs() {
        let text = "# Source packages in this list are ignored by automatic updates.\n# Add one name per line.\nthunderbird\n\nbrave-browser\n";
        assert_eq!(system_lines(text), list(&["thunderbird", "brave-browser"]));
        assert_eq!(
            system_changes(
                &list(&["brave-browser", "signal-desktop"]),
                &list(&["thunderbird", "antigravity"]),
                Ok(text.to_string())
            ),
            list(&["+signal-desktop", "-thunderbird"])
        );
        assert_eq!(
            system_changes(
                &list(&["brave-browser"]),
                &list(&["thunderbird"]),
                Err(std::io::Error::from(ErrorKind::NotFound))
            ),
            list(&["+brave-browser"])
        );
        assert_eq!(
            system_changes(
                &list(&["brave-browser"]),
                &list(&["thunderbird"]),
                Err(std::io::Error::from(ErrorKind::PermissionDenied))
            ),
            list(&["+brave-browser", "-thunderbird"])
        );
        assert_eq!(
            merged(&list(&["a", "b"]), &list(&["c", "a"]), &list(&["b"])),
            list(&["a", "c"])
        );
        assert!(SYSTEM_SH.starts_with(&format!("f={SYSTEM}\n")));
    }

    #[test]
    fn without_admin_removing_what_the_system_list_holds_is_refused() {
        let lines = list(&["thunderbird", "signal-desktop=7.1", "code=1.95"]);
        let cases: [(&[&str], &[&str], &[&str]); 6] = [
            (&[], &["thunderbird"], &["thunderbird"]),
            (&[], &["brave-browser"], &[]),
            (&[], &["signal-desktop=7.1"], &["signal-desktop=7.1"]),
            (&["code"], &["code=1.95"], &[]),
            (&["code=1.95"], &["code=1.95"], &[]),
            (
                &[],
                &["code=1.95", "gone", "thunderbird"],
                &["code=1.95", "thunderbird"],
            ),
        ];
        for (add, remove, stuck) in cases {
            let (add, remove) = (list(add), list(remove));
            assert_eq!(
                held_by_system(&lines, &add, &remove),
                stuck.to_vec(),
                "add {add:?} remove {remove:?}"
            );
        }
    }
}
