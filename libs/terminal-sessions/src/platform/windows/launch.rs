use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::BOOL;
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT};

use crate::cli::CliToolId;
use crate::console::LAUNCH_COMMAND;
use crate::{SpawnIdentity, SpawnKey, SpawnSurface};

const FIELD_SEPARATOR: char = ':';
const PATH_SEPARATOR: char = ';';
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";
const VERBATIM_UNC: &str = r"\\?\UNC\";
const VERBATIM: &str = r"\\?\";
const NEW_TAB_WINDOW: &str = "0";
const NEW_WINDOW: &str = "new";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LaunchSpec {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<(String, String)>,
    pub(super) title: Option<String>,
}

impl LaunchSpec {
    fn path(&self) -> Option<&str> {
        self.env
            .iter()
            .rev()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LaunchTag {
    pub(super) nonce: String,
    pub(super) identity: SpawnIdentity,
}

impl LaunchTag {
    pub(super) fn encode(&self) -> String {
        [
            self.nonce.as_str(),
            surface_tag(self.identity.surface),
            self.identity.tool.as_str(),
            self.identity.key.as_str(),
        ]
        .join(&FIELD_SEPARATOR.to_string())
    }

    pub(super) fn parse(token: &str) -> Option<LaunchTag> {
        let mut fields = token.split(FIELD_SEPARATOR);
        let nonce = fields.next()?;
        let surface = surface_from_tag(fields.next()?)?;
        let tool = CliToolId::new(fields.next()?).ok()?;
        let key = SpawnKey::new(fields.next()?).ok()?;
        let valid_nonce = !nonce.is_empty()
            && nonce
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'-');
        (valid_nonce && fields.next().is_none()).then(|| LaunchTag {
            nonce: nonce.to_owned(),
            identity: SpawnIdentity { key, tool, surface },
        })
    }
}

pub(super) fn tag_in_command_line(command_line: &str) -> Option<LaunchTag> {
    let mut words = command_line.split_whitespace();
    words.find(|word| *word == LAUNCH_COMMAND)?;
    LaunchTag::parse(words.next()?)
}

fn surface_tag(surface: SpawnSurface) -> &'static str {
    match surface {
        SpawnSurface::Tab => "tab",
        SpawnSurface::OsWindow => "os_window",
    }
}

fn surface_from_tag(tag: &str) -> Option<SpawnSurface> {
    match tag {
        "tab" => Some(SpawnSurface::Tab),
        "os_window" => Some(SpawnSurface::OsWindow),
        _ => None,
    }
}

pub(super) fn launcher_args(tag: &str, spec: &str) -> Vec<String> {
    vec![LAUNCH_COMMAND.to_owned(), tag.to_owned(), spec.to_owned()]
}

pub(super) fn terminal_args(
    surface: SpawnSurface,
    title: Option<&str>,
    cwd: &str,
    executable: &str,
    launcher: &[String],
) -> Vec<String> {
    let window = match surface {
        SpawnSurface::Tab => NEW_TAB_WINDOW,
        SpawnSurface::OsWindow => NEW_WINDOW,
    };
    let mut args = vec!["-w".to_owned(), window.to_owned(), "new-tab".to_owned()];
    if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
        args.push("--title".to_owned());
        args.push(terminal_escape(title));
    }
    args.push("-d".to_owned());
    args.push(terminal_escape(cwd));
    args.push(terminal_escape(executable));
    args.extend(launcher.iter().map(|arg| terminal_escape(arg)));
    args
}

fn terminal_escape(arg: &str) -> String {
    arg.replace(';', r"\;")
}

pub(super) fn plain_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(VERBATIM_UNC) {
        return format!(r"\\{rest}");
    }
    match path.strip_prefix(VERBATIM) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => path.to_owned(),
    }
}

pub(super) fn resolve_program(
    program: &str,
    path: Option<&str>,
    pathext: Option<&str>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let extensions: Vec<&str> = pathext
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_PATHEXT)
        .split(PATH_SEPARATOR)
        .map(str::trim)
        .filter(|extension| extension.starts_with('.'))
        .collect();
    let named = Path::new(program);
    let has_extension = named.extension().is_some();
    let candidates = |base: PathBuf| {
        let name = base.as_os_str().to_owned();
        has_extension
            .then(|| base.clone())
            .into_iter()
            .chain(extensions.iter().map(move |extension| {
                let mut file = name.clone();
                file.push(extension);
                PathBuf::from(file)
            }))
    };
    if program.contains(['\\', '/']) {
        return candidates(named.to_path_buf()).find(|file| exists(file));
    }
    path.unwrap_or_default()
        .split(PATH_SEPARATOR)
        .map(|dir| dir.trim().trim_matches('"'))
        .filter(|dir| !dir.is_empty())
        .flat_map(|dir| candidates(Path::new(dir).join(program)))
        .find(|file| exists(file))
}

pub(super) fn run(args: &[String]) -> io::Result<String> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let [tag, spec_path] = args else {
        return Err(invalid(format!(
            "expected {LAUNCH_COMMAND} TAG SPEC, got {} arguments",
            args.len()
        )));
    };
    LaunchTag::parse(tag).ok_or_else(|| invalid(format!("invalid launch tag {tag:?}")))?;
    let text = std::fs::read_to_string(spec_path);
    let _ = std::fs::remove_file(spec_path);
    let spec: LaunchSpec = serde_json::from_str(&text?).map_err(io::Error::other)?;
    if let Some(title) = &spec.title {
        let _ = super::title::set(title);
    }
    let input = console_file("CONIN$")?;
    let mut output = console_file("CONOUT$")?;
    let status = start(&spec, input, output.try_clone()?).and_then(|mut child| {
        unsafe { SetConsoleCtrlHandler(Some(ignore_interrupts), 1) };
        child.wait()
    });
    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            let _ = writeln!(output, "{LAUNCH_COMMAND}: {}: {error}", spec.program);
            Err(error)
        }
    }
}

fn start(spec: &LaunchSpec, input: File, output: File) -> io::Result<std::process::Child> {
    let pathext = std::env::var("PATHEXT").ok();
    let inherited = std::env::var("PATH").ok();
    let path = spec.path().or(inherited.as_deref());
    let program = resolve_program(&spec.program, path, pathext.as_deref(), Path::is_file)
        .unwrap_or_else(|| PathBuf::from(&spec.program));
    Command::new(program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(name, value)| (name, value)))
        .stdin(input.try_clone()?)
        .stdout(output.try_clone()?)
        .stderr(output)
        .spawn()
}

fn console_file(name: &str) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(name)
}

unsafe extern "system" fn ignore_interrupts(kind: u32) -> BOOL {
    BOOL::from(kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(surface: SpawnSurface) -> LaunchTag {
        LaunchTag {
            nonce: "41-1700000000".to_owned(),
            identity: SpawnIdentity {
                key: SpawnKey::new("lane-1.a").unwrap(),
                tool: CliToolId::new("claude").unwrap(),
                surface,
            },
        }
    }

    #[test]
    fn launch_tags_round_trip_and_reject_malformed_tokens() {
        for surface in [SpawnSurface::Tab, SpawnSurface::OsWindow] {
            let tag = tag(surface);
            assert_eq!(LaunchTag::parse(&tag.encode()), Some(tag));
        }
        assert_eq!(
            tag(SpawnSurface::Tab).encode(),
            "41-1700000000:tab:claude:lane-1.a"
        );
        let rejected = [
            "",
            "41:tab:claude",
            "41:tab:claude:lane:extra",
            ":tab:claude:lane",
            "4a:tab:claude:lane",
            "41:pane:claude:lane",
            "41:tab:cl aude:lane",
            "41:tab:claude:",
            "41:os-window:claude:lane",
        ];
        for token in rejected {
            assert_eq!(LaunchTag::parse(token), None, "{token:?}");
        }
    }

    #[test]
    fn tags_are_read_from_the_launcher_command_line_only() {
        let expected = Some(tag(SpawnSurface::Tab));
        let cases = [
            (
                r#""C:\Program Files\qol\qol.exe" console-launch 41-1700000000:tab:claude:lane-1.a "C:\Users\a b\Temp\spec.json""#,
                expected.clone(),
            ),
            (
                r"C:\qol\qol.exe console-launch 41-1700000000:tab:claude:lane-1.a C:\spec.json",
                expected,
            ),
            (r"C:\qol\qol.exe console-launch", None),
            (
                r"C:\qol\qol.exe console-probe 41-1700000000:tab:claude:lane-1.a",
                None,
            ),
            (r"pwsh.exe -NoLogo", None),
        ];
        for (command_line, expected) in cases {
            assert_eq!(
                tag_in_command_line(command_line),
                expected,
                "{command_line}"
            );
        }
    }

    #[test]
    fn terminal_args_open_a_tab_or_window_and_escape_command_separators() {
        let launcher = launcher_args("41-1:tab:claude:lane", r"C:\Temp\spec;1.json");
        let cases = [
            (
                SpawnSurface::Tab,
                Some("Lane; one"),
                vec![
                    "-w",
                    "0",
                    "new-tab",
                    "--title",
                    r"Lane\; one",
                    "-d",
                    r"C:\work\a b",
                    r"C:\qol\qol.exe",
                    "console-launch",
                    "41-1:tab:claude:lane",
                    r"C:\Temp\spec\;1.json",
                ],
            ),
            (
                SpawnSurface::OsWindow,
                Some("  "),
                vec![
                    "-w",
                    "new",
                    "new-tab",
                    "-d",
                    r"C:\work\a b",
                    r"C:\qol\qol.exe",
                    "console-launch",
                    "41-1:tab:claude:lane",
                    r"C:\Temp\spec\;1.json",
                ],
            ),
        ];
        for (surface, title, expected) in cases {
            assert_eq!(
                terminal_args(surface, title, r"C:\work\a b", r"C:\qol\qol.exe", &launcher),
                expected,
                "{surface}"
            );
        }
    }

    #[test]
    fn verbatim_prefixes_are_stripped_from_working_directories() {
        let cases = [
            (r"\\?\C:\work\proj", r"C:\work\proj"),
            (r"\\?\UNC\server\share\dir", r"\\server\share\dir"),
            (r"C:\work", r"C:\work"),
            (r"\\server\share", r"\\server\share"),
            (r"\\?\Volume{abc}\dir", r"\\?\Volume{abc}\dir"),
        ];
        for (path, expected) in cases {
            assert_eq!(plain_path(path), expected, "{path}");
        }
    }

    #[test]
    fn programs_resolve_through_path_and_pathext() {
        let files = [
            r"C:\bin\codex.cmd",
            r"C:\tools\claude.exe",
            r"C:\tools\codex.exe",
            r"C:\abs\pi.bat",
        ];
        let exists = |path: &Path| {
            files
                .iter()
                .any(|file| file.eq_ignore_ascii_case(&path.to_string_lossy()))
        };
        let path = Some(r#"C:\bin;"C:\tools";;"#);
        let cases = [
            ("codex", None, Some(r"C:\bin\codex.CMD")),
            ("codex", Some(".EXE"), Some(r"C:\tools\codex.EXE")),
            ("claude", None, Some(r"C:\tools\claude.EXE")),
            ("claude.exe", None, Some(r"C:\tools\claude.exe")),
            (r"C:\abs\pi", None, Some(r"C:\abs\pi.BAT")),
            (r"C:\abs\pi.bat", None, Some(r"C:\abs\pi.bat")),
            ("missing", None, None),
        ];
        for (program, pathext, expected) in cases {
            assert_eq!(
                resolve_program(program, path, pathext, exists),
                expected.map(PathBuf::from),
                "{program}"
            );
        }
    }

    #[test]
    fn the_last_path_entry_in_the_launch_environment_wins() {
        let spec = LaunchSpec {
            program: "codex".to_owned(),
            args: Vec::new(),
            env: vec![
                ("PATH".to_owned(), r"C:\first".to_owned()),
                ("CLAUDE_CONFIG_DIR".to_owned(), r"C:\c".to_owned()),
                ("Path".to_owned(), r"C:\second".to_owned()),
            ],
            title: None,
        };
        assert_eq!(spec.path(), Some(r"C:\second"));
    }
}
