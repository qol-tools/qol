use anyhow::{anyhow, bail, Result};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;

use super::guest::{DebianNocloud, GuestOs};
use super::{find_on_path, live, machine, qmp, serial, unix_millis};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const SH_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn cmd_shot(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "shot")?;
    print_title("qol emu shot");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    let path = live
        .run_dir
        .join(format!("screenshot-{}.ppm", unix_millis()?));
    client.screendump(&path)?;
    step_label("shot", StepKind::Success, &path.display().to_string());
    Ok(())
}

pub(crate) fn cmd_key(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, keys) = id_and_rest(args, "key", "<qcode>...")?;
    print_title("qol emu key");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.send_keys(&keys)?;
    step_label("key", StepKind::Success, &keys.join("+"));
    Ok(())
}

pub(crate) fn cmd_insert(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "insert")?;
    print_title("qol emu insert");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let qemu_img = find_on_path("qemu-img").ok_or_else(|| anyhow!("missing qemu-img on PATH"))?;
    let stick = machine::ensure_usb_stick(&live.run_dir, &qemu_img)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.attach_usb_stick(&stick)?;
    step_label("insert", StepKind::Success, &stick.display().to_string());
    Ok(())
}

pub(crate) fn cmd_sh(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, words) = id_and_rest(args, "sh", "<command>...")?;
    print_title("qol emu sh");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let port = live
        .serial_port
        .ok_or_else(|| anyhow!("run has no serial console; rerun `qol emu up {id}`"))?;
    let mut serial = serial::connect(port, CONTROL_TIMEOUT)?;
    DebianNocloud.ensure_root_shell(&mut serial)?;
    let command = words.join(" ");
    let output = serial.run_command(&command, SH_TIMEOUT)?;
    print!("{output}");
    println!();
    step_label("sh", StepKind::Success, &command);
    Ok(())
}

pub(crate) fn cmd_pull(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "pull")?;
    print_title("qol emu pull");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.detach_usb_stick()?;
    step_label("pull", StepKind::Success, "usb stick detached");
    Ok(())
}

pub(crate) fn cmd_down(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "down")?;
    print_title("qol emu down");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.fire("quit")?;
    step_label(
        "down",
        StepKind::Success,
        "quit sent; up will finalize the report",
    );
    Ok(())
}

pub(crate) fn cmd_snap(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "snap")?;
    print_title("qol emu snap");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    let snapshot = live
        .run_dir
        .join(format!("overlay-snap-{}.qcow2", unix_millis()?));
    client.disk_snapshot(&snapshot)?;
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

fn single_id(args: &[OsString], command: &str) -> Result<String> {
    let [id] = args else {
        bail!("usage: qol emu {command} <environment>");
    };
    utf8(id)
}

fn id_and_rest(
    args: &[OsString],
    command: &str,
    rest_usage: &str,
) -> Result<(String, Vec<String>)> {
    let Some((id, rest)) = args.split_first() else {
        bail!("usage: qol emu {command} <environment> {rest_usage}");
    };
    if rest.is_empty() {
        bail!("usage: qol emu {command} <environment> {rest_usage}");
    }
    let rest = rest.iter().map(utf8).collect::<Result<Vec<_>>>()?;
    Ok((utf8(id)?, rest))
}

fn utf8(value: &OsString) -> Result<String> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("argument is not valid UTF-8"))
}
