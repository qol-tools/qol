use anyhow::{anyhow, bail, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;

use super::guest::{DebianNocloud, GuestOs};
use super::{find_on_path, live, machine, serial, unix_millis};

const SH_TIMEOUT: Duration = Duration::from_secs(30);
const SERIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn cmd_shot(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, run_roots) = single_id(args, "shot")?;
    print_title("qol emu shot");
    print_hint(verbose);
    let live::VerifiedRun { run, mut qmp } = find_run(&run_roots, &id)?;
    let path = run
        .run_dir
        .join(format!("screenshot-{}.ppm", unix_millis()?));
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
    let command = words.join(" ");
    let output = serial.run_command(&command, SH_TIMEOUT)?;
    print!("{output}");
    println!();
    step_label("sh", StepKind::Success, &command);
    Ok(())
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
    let snapshot = run
        .run_dir
        .join(format!("overlay-snap-{}.qcow2", unix_millis()?));
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
    let mut options = true;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if options && arg == "--" {
            options = false;
            index += 1;
            continue;
        }
        if options && arg == "--run-root" {
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
        positional.push(arg.clone());
        index += 1;
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
    fn routed_args_accept_roots_before_after_and_repeated() {
        let cases = [
            (
                vec!["--run-root", "/a", "run-a"],
                vec!["run-a"],
                vec![PathBuf::from("/a")],
            ),
            (
                vec!["run-a", "--run-root", "/a"],
                vec!["run-a"],
                vec![PathBuf::from("/a")],
            ),
            (
                vec!["--run-root", "/a", "run-a", "--run-root", "relative/cases"],
                vec!["run-a"],
                vec![PathBuf::from("/a"), PathBuf::from("relative/cases")],
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
    fn routed_args_preserve_literal_flags_after_separator() {
        let (positional, roots) = routed_args(&os_args(&[
            "--run-root",
            "/a",
            "run-a",
            "--",
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
        for input in [vec!["run-a", "--run-root"], vec!["--run-root", ""]] {
            let error = routed_args(&os_args(&input)).unwrap_err().to_string();
            assert!(error.contains("--run-root requires"), "input: {input:?}");
        }
    }
}
