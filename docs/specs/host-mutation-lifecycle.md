# The host-mutation lifecycle

Architecture, 2026-08-19. Built on a five-lane read-only audit of the `bone` tree
(`scratchpad/hostaudit/{display,input,system,tray,mechanism}.md`, 1103 lines, 32 mutation
blocks, every claim carrying a file:line someone actually read).

## The contract being implemented

Stated by you, verbatim, and treated here as the spec:

> when qol tray boots on a PC it snapshots the state of all the plugins it has, before
> applying ANYTHING from the user profile. then when the user exits qol tray: IF RESIDENT:
> noop just exit. IF NOT RESIDENT: restore to initial snapshot baselines (do not let host
> keep users values from their qol profile)

Plus the rule you added when the monitor regression hit:

> MONITORSETTINGS. NO MATTER IF IT IS A RESIDENT OR WHATEVER, SETTING THAT SHOULD FUCKING
> CHANGE VALUES

Residency decides what happens at **exit**. It never decides what a setting does.

And the rule you gave when the four open questions came back, which turns out to govern all of
them:

> qol should not mutate state beyond what it can revert.

That is the whole design in one sentence. It is stronger than "restore at exit", because it
also decides what qol is allowed to touch in the first place. Two consequences, both new:

**Revertibility is hardline on a Portable host.** If qol cannot capture the prior value, it
does not make the change. No exceptions, no degraded write, no "surface it and carry on." A
borrowed machine is handed back exactly as found, and the only way to promise that is to
refuse the mutation qol cannot undo. The refusal is visible: the tray says which setting did
not apply and why.

On a Resident host there is no such gate. Residency is the user saying this box is theirs, so
qol applies the setting and records what it can. A capture that fails on a Resident host is
noted in the doctor's inventory and nothing more.

The one carve-out on both sides is the user-directed destructive action the audit already
classified as out of scope: uninstalling an app, deleting a file. Those are the user's own
act, not qol's policy.

**Recognition gates what gets pushed, never what works.** On a host qol has never seen, it
applies the profile settings that make sense for *this* machine and leaves the rest alone. A
generic setting like light-or-dark theme applies everywhere. A device-specific one does not:
an unidentified monitor gets no profile value pushed at it on startup, because the profile has
no entry describing it.

That is a rule about the startup push and nothing else. The monitor plugin still runs, still
lists that display, and still changes its brightness the moment the user asks - and that live
change is claimed and reverted like any other. An unrecognized device is quiet at boot, not
inert.

## What the audit found

32 host mutations. Nine already honour the contract, nine are user-directed actions that are
correctly out of scope (uninstalling an app, saving a screenshot), and **fourteen are gaps**.

The gaps are not the interesting finding. This is:

**There is no boot snapshot anywhere in the repo.** Tray startup (`app/mod.rs:44`) has no
step that captures pre-qol state. Every subsystem that does snapshot does it lazily, at its
own first write, in its own format. And tray shutdown (`app/mod.rs:145-151`) restores exactly
one thing: the hotkey takeover ledger. It is also the only residency-aware restore in the
host application.

So the contract is currently implemented five separate times, by five different mechanisms,
three of which are wrong:

| Mechanism | Owner | Snapshot | Crash recovery | Residency-aware |
|---|---|---|---|---|
| Policy journal | `qol-host-fixes/policy` | yes, checksummed, staged | yes, state machine | gates enable only |
| Session store | `plugins/monitor` | yes, Envelope + fnv1a | yes | yes, both gates |
| Session store | `plugins/os-themes` | yes, generic over `SessionSnapshot` | yes | yes, exit gate |
| Takeover ledger | `qol-tray/hotkeys/takeover` | yes, claim + previous | marker only | yes |
| Bluetooth hostfix | `plugins/bluetooth` | yes | orphan repair | yes |
| *nothing* | alt-tab, controllers, os-themes KDE, tray autostart | no | no | no |

Five correct-ish implementations of one idea, and every new plugin has to rediscover it. That
is the defect. The fourteen gaps are its symptom.

## The design

One primitive, one ledger, three lifecycle moments. Everything else is a backend.

### The primitive

`libs/qol-host-session`, a crate name you already picked in the earlier plan.

```rust
pub enum Lifetime {
    PortableSession,
    ResidentPolicy,
}

pub trait HostMutation {
    type Snapshot: Serialize + DeserializeOwned;

    fn id(&self) -> MutationId;
    fn lifetime(&self) -> Lifetime;
    fn capture(&self) -> Result<Self::Snapshot>;
    fn restore(&self, snapshot: Self::Snapshot) -> Result<()>;
}
```

A ledger record holds the id, the owning crate, the lifetime, the session id, the captured
payload, a checksum, and a state. Both existing session stores already hold four of those six;
the policy journal holds all six. Nothing here is invented.

### The three moments

```rust
pub fn claim<M: HostMutation>(m: &M) -> Result<()>;      // before the first write
pub fn recover(owner: &str) -> RestoreReport;            // at process start
pub fn release_session(owner: &str) -> RestoreReport;    // at process exit
pub fn release_residency() -> RestoreReport;             // when residency goes off
```

- `claim` captures pre-qol state and persists it, once, idempotently. Second call is a no-op,
  which is what makes "snapshot before the profile is applied" true without a boot-time sweep
  that would have to know every plugin's business.
- `recover` restores records left by a session that died. Portable only.
- `release_session` restores `PortableSession` records when the host is Portable, and does
  nothing when it is Resident. `ResidentPolicy` records are never touched here.
- `release_residency` restores everything, both lifetimes. This is the disable path, and it
  is the one `090fd191` just half-built.

Residency is read at each of those moments, live, never cached. That is already how monitor
does it (`daemon.rs:327`).

### Why lazy claim rather than a literal boot snapshot

Your sentence says the tray snapshots everything at boot. A literal reading needs the tray to
enumerate every mutation every plugin might make, which couples the host to every plugin's
internals and breaks the moment someone writes a new one. `claim` gets the identical
guarantee - no host value is ever overwritten without its predecessor being on disk first -
and it composes. The observable contract is unchanged; only the moment of capture moves, and
it moves to the only point that can be correct for a plugin the host has never heard of.

### What does not move

Root-owned mutations stay in the policy journal at `/var/lib`. They cannot live in a
user-writable ledger, and the audit turned up the constraint that governs them: a restore
needing root cannot run unattended, because elevation goes through `pkexec` and prompts. So
root-owned restores either run from an already-elevated process or fail **visibly**, leaving
the journal intact for a later retry. Never silently skipped. The current code already does
the right thing here (`restore.rs` records a `Failed` entry, `apply_residency` bails with the
list) and that behaviour is load-bearing, not incidental.

## Phases

Each is one commit, each leaves the tree green.

**Phase 1. Extract the ledger.** `os-themes`'s `SessionStore<T: SessionSnapshot>` is already
generic; lift it into `libs/qol-host-session` with the `Lifetime` field added, and migrate
os-themes onto it. Pure move, no behaviour change, so the existing os-themes tests are the
proof.

**Phase 2. Migrate monitor.** Its store is the same shape with two extras: handoff generations
and a side-effecting restore that needs live display handles. Handoff stays a monitor-side
concept layered on top; the restore stays behind its trait impl. The audit lists the exact
divergences (checksum on envelope vs body, clean-marker placement) so this is mechanical.

**Phase 3. Wire the tray.** The three moments get called from `app/mod.rs`: `recover` beside
the existing startup restore (:136), `release_session` beside the existing exit restore (:150),
`release_residency` from `apply_residency`. Today the exit path restores hotkeys and nothing
else, and `resident_policy::restore_all` is never on it at all.

**Phase 4. Convert the gaps**, worst blast radius first, one commit each:

1. **alt-tab** installs a Cinnamon extension into the user's extensions dir and edits
   `enabled-extensions`. Never swept, never snapshotted. A Portable host keeps a qol extension
   in its desktop environment forever.
2. **controllers** writes a root-owned `xpadneo.conf` and a live sysfs param. Root-owned, so
   this one goes in the policy journal, not the ledger.
3. **os-themes KDE** writes `kdeglobals`, `kcminputrc`, and the GTK ini with no snapshot at
   all; the restore engine only replays gsettings keys, so a KDE host is never handed back.
4. **bluetooth** adapter power and the PipeWire default sink: live host state changed from
   config, never captured.
5. **autostart**: keep writing it on Portable hosts, and make it revertible. That needs
   `remove_target` on the `AutostartOps` trait and each platform impl (every platform's
   artifact is one owned file), plus a persisted intent flag consulted by
   `heal_drift_on_startup`, which today treats an absent artifact as drift and rewrites it at
   the next login. Without the flag a removal is silently undone.
6. **launcher bundles**: sweep them on a Portable exit. On top of that, a Portable run writes
   its bundles to a temp directory and points the launcher there, so the host's own indexer
   never sees them at all. Belt and braces: even a crash that skips the sweep leaves nothing
   in `~/Applications`.
7. **window-actions minimize-state file**: the geometry itself is out of scope, but the state
   record it leaves on disk is not. Either write it inside qol's own state dir or claim it.
8. **lights**: capture bulb state before the first change and revert on a Portable exit, same
   as everything else.

**Phase 5. Make it inspectable.** The mission requires owned mutations to stay visible. A
doctor section and a settings row listing what qol currently owns on this host, with a
"hand this machine back" action that runs `release_residency` on demand.

## What the phase research turned up

Three lanes turned each phase into an implementation brief. Three findings change the plan
rather than just filling it in.

**Restore is per-process, not central.** Each plugin daemon restores its own mutations on its
own SIGTERM; the tray cannot reach into another process to undo its work. So `release_session`
at the tray is not the mechanism that hands the host back - it is the crash net over daemons
that already died. Exit correctness rests on the tray actually SIGTERMing every daemon and
each one restoring itself, which makes the daemon lifecycle part of this contract rather than
an implementation detail beside it.

**Monitor's snapshots on disk are flat JSON, not envelopes.** A user upgrading has pending
baselines written in the old shape right now. A loader that only understands the new envelope
would treat every one of them as unreadable and abandon the values it was supposed to restore,
which is precisely the failure this design exists to prevent. The migration reads both shapes
and rewraps on the next write.

**The lights backend cannot read bulb state at all.** `send_cluster_command` is the only path;
there is no ZCL read-attribute call anywhere in the crate, and the daemon caches only what it
predicted from its own writes. Under the hardline Portable rule that is decisive: until a read
path exists, lights must not change bulb state on a Portable host, because qol cannot put it
back. Adding the read is the prerequisite, not the polish.

## Verification

- Ledger and per-phase unit tests, plus one conformance test per migrated backend asserting
  the four-moment behaviour table.
- A guest VM lane per Linux-only phase (`qol env up <env> --dev-worktree`), never the host
  session: set a value, kill -9, restart, assert recovery; then flip Portable and assert the
  baseline returns on a real quit.
- macOS host check for the monitor DDC path, since that is the machine that reported the
  original regression.

## Decisions taken

1. **Autostart** is written on Portable hosts too, because it is revertible. Phase 4.5 makes
   it actually revert.
2. **Launcher bundles** are swept on exit, and on a Portable host they are written to a temp
   directory in the first place so the host launcher never indexes them.
3. **Window geometry** is visual state and does not count. Moving or minimizing a window is
   not a mutation. The state *file* window-actions leaves behind is, and phase 4.7 handles it.
4. **Lights** are in scope: capture and revert like anything else.

## Device identity

The recognition gate needs a stable per-display id. EDID is the answer on every platform: it
carries the manufacturer id, product code, and serial number, and every OS exposes it, just
through a different door.

- **Linux**: read the raw EDID block from the DRM subsystem at `/sys/class/drm/card*-*/edid`,
  the same source `edid-decode` and `xrandr --props` use. The connector name in that path
  (`card0-HDMI-A-1`) is a free disambiguator.
- **macOS**: `CGDisplayVendorNumber`, `CGDisplayModelNumber`, and `CGDisplaySerialNumber` give
  the three EDID fields without parsing anything, and `CGDisplayCreateUUIDFromDisplayID`
  (ColorSync) gives a per-display UUID that survives reconnect. The raw block is reachable
  through IOKit's display info dictionary if the parsed fields prove insufficient.
- **Windows**: `WmiMonitorID` over WMI, or `DISPLAYCONFIG_TARGET_DEVICE_NAME` via
  `DisplayConfigGetDeviceInfo`; both resolve to the same EDID data.

One caveat worth designing around rather than discovering later: **some vendors ship a whole
batch with the same serial number**, so two identical monitors on one desk can be
indistinguishable from EDID alone. The id is therefore manufacturer + product + serial, with a
connector or display-index disambiguator when two attached displays hash the same. When even
that cannot separate them, the honest move is to treat both as unrecognized rather than push
the wrong profile value at the wrong screen.

Sources: [EDID structure](https://en.wikipedia.org/wiki/Extended_Display_Identification_Data),
[reading EDID on Linux](https://www.adamsdesk.com/posts/read-edid-e-edid-displayid-metadata-linux/),
[decoding monitor EDID on macOS](https://notes.alinpanaitiu.com/Decoding-monitor-EDID-on-macOS),
[DISPLAYCONFIG_TARGET_DEVICE_NAME](https://learn.microsoft.com/en-us/windows/desktop/api/wingdi/ns-wingdi-displayconfig_target_device_name).
