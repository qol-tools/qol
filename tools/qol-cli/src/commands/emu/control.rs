use anyhow::{anyhow, bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;

use super::guest::{DebianNocloud, GuestOs};
use super::{find_on_path, live, machine, serial};

const SH_TIMEOUT: Duration = Duration::from_secs(30);
const SERIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const GUEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const GUEST_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
const GUEST_REQUEST_SLACK: Duration = Duration::from_secs(5);
const DRAG_STEPS: u32 = 12;
const DRAG_STEP_DELAY: Duration = Duration::from_millis(50);
const DRAG_SETTLE: Duration = Duration::from_millis(200);

pub(crate) fn cmd_shot(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, run_roots) = single_id(args, "shot")?;
    print_title("qol emu shot");
    print_hint(verbose);
    let live::VerifiedRun { run, mut qmp } = find_run(&run_roots, &id)?;
    let path = run
        .run_dir
        .join(format!("screenshot-{}.ppm", qol_dev_env::unix_millis()?));
    qmp.screendump(&path)?;
    step_label("shot", StepKind::Success, &path.display().to_string());
    Ok(())
}

pub(crate) fn cmd_key(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, keys, run_roots) = id_and_rest(args, "key", "<qcode>...")?;
    print_title("qol emu key");
    print_hint(verbose);
    let live::VerifiedRun { mut qmp, .. } = find_run(&run_roots, &id)?;
    qmp.send_keys(&keys)?;
    step_label("key", StepKind::Success, &keys.join("+"));
    Ok(())
}

pub(crate) fn cmd_insert(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, run_roots) = single_id(args, "insert")?;
    print_title("qol emu insert");
    print_hint(verbose);
    let live::VerifiedRun { run, mut qmp } = find_run(&run_roots, &id)?;
    let qemu_img = find_on_path("qemu-img").ok_or_else(|| anyhow!("missing qemu-img on PATH"))?;
    let stick = machine::ensure_usb_stick(&run.run_dir, &qemu_img)?;
    qmp.attach_usb_stick(&stick)?;
    step_label("insert", StepKind::Success, &stick.display().to_string());
    Ok(())
}

pub(crate) fn cmd_sh(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, words, run_roots) = id_and_rest(args, "sh", "<command>...")?;
    print_title("qol emu sh");
    print_hint(verbose);
    let live::VerifiedRun {
        run,
        qmp: _identity_guard,
    } = find_run(&run_roots, &id)?;
    let port = run
        .serial_port
        .ok_or_else(|| anyhow!("run has no serial console; rerun `qol emu up {id}`"))?;
    let mut serial = serial::connect(port, SERIAL_CONNECT_TIMEOUT)?;
    DebianNocloud.ensure_root_shell(&mut serial)?;
    let command = guest_shell_command(&words);
    let output = serial.run_command(&command, SH_TIMEOUT)?;
    print!("{output}");
    println!();
    step_label("sh", StepKind::Success, &command);
    Ok(())
}

fn guest_shell_command(words: &[String]) -> String {
    shell_words::join(words)
}

pub(crate) fn cmd_exec(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, words, run_roots) = id_and_rest(args, "exec", "<absolute-program> [args...]")?;
    print_title("qol emu exec");
    print_hint(verbose);
    let live::VerifiedRun {
        run,
        qmp: _identity_guard,
    } = find_run(&run_roots, &id)?;
    let mut guest = connect_guest_control(&run)?;
    let mut env = std::collections::BTreeMap::new();
    if let Some(display) = guest.hello().session.display.clone() {
        env.insert("DISPLAY".to_string(), display);
    }
    let command = qol_dev_guest::CommandSpec {
        program: words[0].clone(),
        args: words[1..].to_vec(),
        cwd: None,
        env,
    };
    command.validate()?;
    let timeout_ms = u64::try_from(GUEST_EXEC_TIMEOUT.as_millis())
        .map_err(|_| anyhow!("guest exec timeout overflow"))?;
    let result = guest.request(
        qol_dev_guest::RequestAction::Exec {
            command,
            timeout_ms,
        },
        GUEST_EXEC_TIMEOUT + GUEST_REQUEST_SLACK,
    )?;
    let qol_dev_guest::ResponseResult::Process { outcome } = result else {
        bail!("guest exec failed: {result:?}");
    };
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    match (&outcome.state, outcome.exit_code) {
        (qol_dev_guest::ProcessState::Exited, Some(0)) => {
            step_label("exec", StepKind::Success, &words.join(" "));
            Ok(())
        }
        (state, code) => bail!("guest command state={state:?} exit_code={code:?}"),
    }
}

fn connect_guest_control(run: &live::LiveRun) -> Result<qol_dev_guest::GuestControlClient> {
    let report_path = run.run_dir.join("report.json");
    let content = std::fs::read(&report_path)
        .with_context(|| format!("failed to read run report {}", report_path.display()))?;
    let report: serde_json::Value = serde_json::from_slice(&content)
        .with_context(|| format!("invalid run report {}", report_path.display()))?;
    let port = report
        .get("guest_control")
        .and_then(|control| control.get("port"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .context("run has no guest-control channel; only prepared desktop guests expose one")?;
    let image_revision = report
        .get("launch")
        .and_then(|launch| launch.get("guest_image_revision"))
        .and_then(serde_json::Value::as_str)
        .context("run report has no guest image revision")?;
    let address =
        std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port);
    qol_dev_guest::GuestControlClient::connect_verified_identity(
        address,
        GUEST_CONNECT_TIMEOUT,
        GUEST_CONNECT_TIMEOUT,
        &run.environment_id,
        image_revision,
        &run.run_id,
    )
}

pub(crate) fn cmd_drag(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, words, run_roots) = id_and_rest(args, "drag", "<x1,y1> <x2,y2>")?;
    let [from, to] = words.as_slice() else {
        bail!("usage: qol emu drag [--run-root <path>] <run-id|environment> <x1,y1> <x2,y2>");
    };
    let from = parse_point(from)?;
    let to = parse_point(to)?;
    print_title("qol emu drag");
    print_hint(verbose);
    let live::VerifiedRun { run, mut qmp } = find_run(&run_roots, &id)?;
    let (width, height) = framebuffer_size(&mut qmp, &run.run_dir)?;
    qmp.move_pointer_absolute(from.0, from.1, width, height)?;
    std::thread::sleep(DRAG_SETTLE);
    qmp.set_left_button(true)?;
    std::thread::sleep(DRAG_SETTLE);
    for (x, y) in drag_waypoints(from, to, DRAG_STEPS) {
        qmp.move_pointer_absolute(x, y, width, height)?;
        std::thread::sleep(DRAG_STEP_DELAY);
    }
    std::thread::sleep(DRAG_SETTLE);
    qmp.set_left_button(false)?;
    step_label(
        "drag",
        StepKind::Success,
        &format!("{},{} -> {},{}", from.0, from.1, to.0, to.1),
    );
    Ok(())
}

fn parse_point(value: &str) -> Result<(u32, u32)> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| anyhow!("point must be <x,y>, got `{value}`"))?;
    let x = x
        .trim()
        .parse()
        .with_context(|| format!("invalid x in `{value}`"))?;
    let y = y
        .trim()
        .parse()
        .with_context(|| format!("invalid y in `{value}`"))?;
    Ok((x, y))
}

pub(crate) fn cmd_click(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, words, run_roots) = id_and_rest(args, "click", "<x,y> [--button left|middle|right]")?;
    let (point, button) = parse_click_args(&words)?;
    print_title("qol emu click");
    print_hint(verbose);
    let live::VerifiedRun { run, mut qmp } = find_run(&run_roots, &id)?;
    let (width, height) = framebuffer_size(&mut qmp, &run.run_dir)?;
    qmp.move_pointer_absolute(point.0, point.1, width, height)?;
    std::thread::sleep(DRAG_SETTLE);
    qmp.click_button(button)?;
    step_label(
        "click",
        StepKind::Success,
        &format!("{},{} {}", point.0, point.1, button),
    );
    Ok(())
}

fn parse_click_args(words: &[String]) -> Result<((u32, u32), &'static str)> {
    let [point, rest @ ..] = words else {
        bail!("usage: qol emu click [--run-root <path>] <run-id|environment> <x,y> [--button left|middle|right]");
    };
    let point = parse_point(point)?;
    let button = match rest {
        [] => "left",
        [flag, name] if flag == "--button" => match name.as_str() {
            "left" => "left",
            "middle" => "middle",
            "right" => "right",
            other => bail!(
                "unknown button `{other}`; usage: qol emu click [--run-root <path>] <run-id|environment> <x,y> [--button left|middle|right]"
            ),
        },
        _ => bail!(
            "usage: qol emu click [--run-root <path>] <run-id|environment> <x,y> [--button left|middle|right]"
        ),
    };
    Ok((point, button))
}

fn drag_waypoints(from: (u32, u32), to: (u32, u32), steps: u32) -> Vec<(u32, u32)> {
    (1..=steps)
        .map(|step| {
            let lerp = |a: u32, b: u32| {
                (i64::from(a) + (i64::from(b) - i64::from(a)) * i64::from(step) / i64::from(steps))
                    .max(0) as u32
            };
            (lerp(from.0, to.0), lerp(from.1, to.1))
        })
        .collect()
}

fn framebuffer_size(qmp: &mut super::qmp::QmpClient, run_dir: &Path) -> Result<(u32, u32)> {
    let probe = run_dir.join(format!("pointer-probe-{}.ppm", qol_dev_env::unix_millis()?));
    qmp.screendump(&probe)?;
    let header = std::fs::read(&probe)
        .with_context(|| format!("failed to read screendump {}", probe.display()))?;
    let _ = std::fs::remove_file(&probe);
    parse_ppm_size(&header).context("screendump is not a parsable PPM; cannot scale pointer")
}

fn parse_ppm_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(64)]);
    let mut fields = text.split_ascii_whitespace();
    if fields.next()? != "P6" {
        return None;
    }
    let width = fields.next()?.parse().ok()?;
    let height = fields.next()?.parse().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

pub(crate) fn cmd_pull(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, run_roots) = single_id(args, "pull")?;
    print_title("qol emu pull");
    print_hint(verbose);
    let live::VerifiedRun { mut qmp, .. } = find_run(&run_roots, &id)?;
    qmp.detach_usb_stick()?;
    step_label("pull", StepKind::Success, "usb stick detached");
    Ok(())
}

pub(crate) fn cmd_down(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, run_roots) = single_id(args, "down")?;
    print_title("qol emu down");
    print_hint(verbose);
    let live::VerifiedRun { mut qmp, .. } = find_run(&run_roots, &id)?;
    qmp.fire("quit")?;
    step_label(
        "down",
        StepKind::Success,
        "quit sent; up will finalize the report",
    );
    Ok(())
}

pub(crate) fn cmd_snap(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, run_roots) = single_id(args, "snap")?;
    print_title("qol emu snap");
    print_hint(verbose);
    let live::VerifiedRun { run, mut qmp } = find_run(&run_roots, &id)?;
    let snapshot = run.run_dir.join(format!(
        "overlay-snap-{}.qcow2",
        qol_dev_env::unix_millis()?
    ));
    qmp.disk_snapshot(&snapshot)?;
    step_label("snap", StepKind::Success, &snapshot.display().to_string());
    step_label(
        "frozen",
        StepKind::Info,
        "previous overlay is now read-only and safe for host inspection",
    );
    Ok(())
}

fn runs_root() -> Result<PathBuf> {
    Ok(repo_root()?.join("target/qol-emu"))
}

fn find_run(run_roots: &[PathBuf], selector: &str) -> Result<live::VerifiedRun> {
    if run_roots.is_empty() {
        return live::find(&runs_root()?, selector);
    }
    live::find_in_roots(run_roots.iter().map(PathBuf::as_path), selector)
}

fn single_id(args: &[OsString], command: &str) -> Result<(String, Vec<PathBuf>)> {
    let (positional, run_roots) = routed_args(args)?;
    let [id] = positional.as_slice() else {
        bail!("usage: qol emu {command} [--run-root <path>] <run-id|environment>");
    };
    Ok((utf8(id)?, run_roots))
}

fn id_and_rest(
    args: &[OsString],
    command: &str,
    rest_usage: &str,
) -> Result<(String, Vec<String>, Vec<PathBuf>)> {
    let (positional, run_roots) = routed_args(args)?;
    let Some((id, rest)) = positional.split_first() else {
        bail!("usage: qol emu {command} [--run-root <path>] <run-id|environment> {rest_usage}");
    };
    if rest.is_empty() {
        bail!("usage: qol emu {command} [--run-root <path>] <run-id|environment> {rest_usage}");
    }
    let rest = rest.iter().map(utf8).collect::<Result<Vec<_>>>()?;
    Ok((utf8(id)?, rest, run_roots))
}

fn routed_args(args: &[OsString]) -> Result<(Vec<OsString>, Vec<PathBuf>)> {
    let mut positional = Vec::new();
    let mut run_roots = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            positional.extend_from_slice(&args[index + 1..]);
            break;
        }
        if arg == "--run-root" {
            let Some(path) = args.get(index + 1) else {
                bail!("--run-root requires a path");
            };
            if Path::new(path).as_os_str().is_empty() {
                bail!("--run-root requires a non-empty path");
            }
            run_roots.push(PathBuf::from(path));
            index += 2;
            continue;
        }
        positional.extend_from_slice(&args[index..]);
        break;
    }
    Ok((positional, run_roots))
}

fn utf8(value: &OsString) -> Result<String> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("argument is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn point_parsing_extracts_pixel_pairs() {
        let cases = [("300,300", Some((300, 300))), ("0,799", Some((0, 799)))];
        for (input, expected) in cases {
            assert_eq!(parse_point(input).ok(), expected, "input: {input}");
        }
        for input in ["300", "a,b", "300,", ",300", "-1,5"] {
            assert!(parse_point(input).is_err(), "input: {input}");
        }
    }

    #[test]
    fn click_arg_parsing_defaults_and_validates_buttons() {
        let (point, button) = parse_click_args(&["300,300".to_string()]).unwrap();
        assert_eq!((point, button), ((300, 300), "left"));

        let (point, button) = parse_click_args(&[
            "10,20".to_string(),
            "--button".to_string(),
            "middle".to_string(),
        ])
        .unwrap();
        assert_eq!((point, button), ((10, 20), "middle"));

        let error = parse_click_args(&[
            "10,20".to_string(),
            "--button".to_string(),
            "wheel".to_string(),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("usage: qol emu click"),
            "error: {error}"
        );
    }

    #[test]
    fn drag_waypoints_interpolate_monotonically_to_target() {
        let cases = [
            ((300, 300), (700, 600), 12),
            ((700, 600), (300, 300), 12),
            ((5, 5), (5, 5), 4),
        ];
        for (from, to, steps) in cases {
            let waypoints = drag_waypoints(from, to, steps);
            assert_eq!(waypoints.len(), steps as usize, "from: {from:?} to: {to:?}");
            assert_eq!(
                waypoints.last(),
                Some(&to),
                "final waypoint must land on target, from: {from:?} to: {to:?}"
            );
        }
    }

    #[test]
    fn ppm_size_parsing_reads_header_and_rejects_junk() {
        let cases = [
            (&b"P6\n1280 800\n255\nrest"[..], Some((1280, 800))),
            (&b"P6 640 480 255 "[..], Some((640, 480))),
            (&b"P5\n1280 800\n255\n"[..], None),
            (&b"P6\n0 800\n255\n"[..], None),
            (&b"not a ppm"[..], None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_ppm_size(input),
                expected,
                "input: {:?}",
                String::from_utf8_lossy(&input[..input.len().min(16)])
            );
        }
    }

    #[test]
    fn routed_args_accept_roots_before_selector_and_repeated() {
        let cases = [
            (
                vec!["--run-root", "/a", "run-a"],
                vec!["run-a"],
                vec![PathBuf::from("/a")],
            ),
            (
                vec!["--run-root", "/a", "--run-root", "relative/cases", "run-a"],
                vec!["run-a"],
                vec![PathBuf::from("/a"), PathBuf::from("relative/cases")],
            ),
            (
                vec!["--run-root", "/a", "--", "--run-root"],
                vec!["--run-root"],
                vec![PathBuf::from("/a")],
            ),
        ];
        for (input, expected_positional, expected_roots) in cases {
            let (positional, roots) = routed_args(&os_args(&input)).unwrap();
            assert_eq!(
                positional,
                os_args(&expected_positional),
                "input: {input:?}"
            );
            assert_eq!(roots, expected_roots, "input: {input:?}");
        }
    }

    #[test]
    fn id_and_rest_forwards_selector_adjacent_delimiter_as_guest_argument() {
        let (id, words, roots) = id_and_rest(
            &os_args(&[
                "--run-root",
                "/a",
                "run-a",
                "--",
                "echo",
                "--run-root",
                "literal",
            ]),
            "sh",
            "<command>...",
        )
        .unwrap();
        assert_eq!(id, "run-a");
        assert_eq!(words, ["--", "echo", "--run-root", "literal"]);
        assert_eq!(roots, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn global_delimiter_preserves_guest_global_flag_literals() {
        let parsed = crate::cli::parse_cli(os_args(&[
            "emu",
            "sh",
            "--",
            "run-a",
            "echo",
            "-v",
            "--no-plugins",
        ]));
        let (id, words, roots) = id_and_rest(&parsed.values[2..], "sh", "<command>...").unwrap();

        assert!(!parsed.verbose);
        assert!(!parsed.skip_plugins);
        assert_eq!(id, "run-a");
        assert_eq!(words, ["echo", "-v", "--no-plugins"]);
        assert!(roots.is_empty());
    }

    #[test]
    fn guest_shell_command_preserves_every_argument_boundary() {
        let words =
            ["printf", "%s\\n", "two words", "$(not-run)", "", "quote'"].map(str::to_string);
        let command = guest_shell_command(&words);

        assert_eq!(shell_words::split(&command).unwrap(), words);
    }

    #[test]
    fn routed_args_do_not_intercept_run_root_after_selector() {
        let (positional, roots) = routed_args(&os_args(&[
            "--run-root",
            "/a",
            "run-a",
            "echo",
            "--run-root",
            "literal",
        ]))
        .unwrap();
        assert_eq!(
            positional,
            os_args(&["run-a", "echo", "--run-root", "literal"])
        );
        assert_eq!(roots, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn routed_args_reject_missing_and_empty_root_paths() {
        for input in [vec!["--run-root"], vec!["--run-root", ""]] {
            let error = routed_args(&os_args(&input)).unwrap_err().to_string();
            assert!(error.contains("--run-root requires"), "input: {input:?}");
        }
    }
}
