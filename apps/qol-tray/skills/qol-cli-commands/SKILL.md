---
name: qol-cli-commands
description: Use when running, modifying, explaining, or debugging the local `qol` CLI command surface, especially `qol setup`, `qol dev`, `qol emu`, dashboard rows, and the installed-vs-source restart gotcha.
---

# qol-cli-commands

Use this when the user asks about `qol` commands, `qol dev`, the dev dashboard, `qol emu`, or why a CLI/dashboard change is not visible yet.

## Source Of Truth

- CLI source: `tools/qol-cli`
- Command list: `tools/qol-cli/src/cli.rs`
- Dispatch: `tools/qol-cli/src/main.rs`
- Dev dashboard: `tools/qol-cli/src/dev_console.rs`
- Emu discovery/runtime: `tools/qol-cli/src/commands/emu.rs`

Always check source before claiming a command exists.

## Commands

```bash
qol setup
qol dev [worktree]
qol emu list
qol emu doctor
qol emu up <environment>
qol cat [--no-less] [--plain|--color=auto|always|never] <path|->
qol build [name]
qol clean [name]
qol install
qol trace [name]
qol doctor [step]
```

`qol sync` is intentionally not implemented yet; use `make sync` when needed.

Global flags:

```bash
-v, --verbose
-n, --no-plugins
```

`--no-plugins` only applies to `qol dev`.

## Dev Dashboard

`qol dev` starts the tray and opens a terminal dashboard. Current top-level rows:

```text
tray
web
plugins
emu
doctor
logs
trace
```

After changing `tools/qol-cli`, run `qol setup` and restart the current `qol dev` session. The running dashboard is the old process and cannot show newly compiled rows.

## Emu

`qol emu` is the QEMU-backed clean-environment MVP. Architecture (capability x
platform grid, Medium injector strategy, Machine substrate, milestones M1-M5):
`docs/superpowers/specs/2026-06-10-emu-test-harness-design.md`. M1 (launch) and
M2 (control verbs + arch-aware accel) are complete; next is M3 (first GuestOs
adapter + workflow).

Binary, accelerator, machine type, and firmware all derive from the guest
arch (`GuestArch` in `commands/emu/arch.rs`): `qemu-system-<arch>`, hvf/kvm/
whpx only when host arch == guest arch (else tcg), `q35` for x86_64 vs `virt`
+ edk2 pflash (located at `<bin>/../share/qemu/edk2-aarch64-code.fd`) +
`-cpu host|max` for aarch64. Verified on macOS arm64: an aarch64 guest boots
the EDK II UEFI shell under hvf; x86_64 guests fall back to tcg.

The QMP socket stays connectable after `up` reports `running`; the control
verbs below use that socket, resolving the newest run whose `report.json` says
`running`. QEMU emits `DEVICE_DELETED` before the `device_del` return, so the
QMP client buffers events read while awaiting a return; `wait_event` checks
that buffer first.

- `qol emu list`: list discovered/configured emus and resolver state.
- `qol emu doctor`: one row per guest arch (binary path + chosen accelerator), plus qemu-img, virsh, config path, and run directory.
- `qol emu up <id>`: create a disposable qcow2 overlay, boot it in QEMU (per-host accel: kvm/hvf/whpx), confirm control over a loopback QMP socket, and block until the VM exits; teardown removes every disk image in the run dir (`overlay*.qcow2`, `usb-stick.raw`) and keeps `report.json`, `qemu-command.txt`, and screenshots. Report statuses: `running` while up, then `pass` / `failed` / `skipped`.
- `qol emu shot <id>`: QMP screendump into the run dir (kept as evidence).
- `qol emu key <id> <qcode>...`: send one key chord (e.g. `ctrl alt delete`).
- `qol emu insert <id>` / `qol emu pull <id>`: hot-plug a scratch 16M USB stick (xhci + usb-storage); pull waits for `DEVICE_DELETED` then drops the blockdev.
- `qol emu snap <id>`: `blockdev-snapshot-sync` on the active overlay; the previous overlay freezes read-only for host inspection (the M3 `DiskSnapshot`).
- `qol emu down <id>`: send `quit` fire-and-forget; the blocking `up` finalizes the report and teardown.

All control verbs work against a guest with no OS (SeaBIOS screen), which is
how they are runtime-verified.

Emus shown in `qol dev` must be found, not hard-coded. Discovery sources:

- libvirt/QEMU domains from `virsh --connect qemu:///system list --all --name`
- libvirt/QEMU domains from `virsh --connect qemu:///session list --all --name`
- optional local image config

Local image config (string form defaults to x86_64; table form sets the arch):

```toml
[images]
my-windows = "/path/to/windows.qcow2"

[images.my-arm-linux]
path = "~/VMs/arm-linux.qcow2"
arch = "aarch64"
```

Config path is the platform config dir as reported by `qol emu doctor`
(macOS: `~/Library/Application Support/qol-tray/emu.toml`, Linux:
`~/.config/qol-tray/emu.toml`).

Run artifacts live under:

```text
target/qol-emu/
```

## Cat

`qol cat <path|->` is the deterministic local code viewer.

- Rust files are formatted for display through `rustfmt --emit stdout` via stdin.
- Other files and stdin are printed raw.
- Output always uses stable line numbers.
- Color defaults to `auto`: enabled on terminals, disabled when piped.
- `--plain` / `--color=never` disables ANSI color; `--color` / `--color=always` forces it.
- Paging defaults to `auto`: `qol cat <path>` opens `less -R -F -X` in a terminal, while pipes print directly.
- `--less` / `--pager` forces the pager; `--no-less` / `--no-pager` / `--stdout` prints directly.
- The built-in highlighter is dependency-free and intentionally small; it currently targets Rust syntax.
- It does not rewrite the source file.
