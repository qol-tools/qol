# Clipboard history: a headless plugin that extends the launcher

Status: accepted 2026-10-16, in build. Design rounds: canvas https://claude.ai/artifact/4xmpaAq8V2Gf2D1kZRyNZQ, rounds 01 to 05; the locked look is below.

## Goal

Every copy on the computer lands in a searchable history.
The history opens inside the launcher, looks like the launcher, and Enter copies the chosen clip again.
The clipboard plugin ships with no change to the launcher's code, the same way qol memory does today, once the launcher stops knowing memory's vocabulary (see "What leaks today").

## How a plugin extends the launcher today

qol memory is the only plugin that does this. The path has four hops and no plugin-to-plugin channel.

1. **Manifest.** The plugin declares a flow in `plugin.toml`:

   ```toml
   [launcher]
   kind = "flow"
   title = "qol memory"
   prompt = "Ask memory"
   query = "rows"
   ```

   The schema is `LauncherSpec` in `libs/plugin-api/src/manifest/schema.rs` (`kind`, `title`, `prompt`, `query`, `row_actions`).

2. **Host.** qol-tray reads every installed manifest, keeps the valid flows (`collect_flow_entries` in `apps/tray/src/features/launcher_apps/mod.rs`) and writes them to `launcher-flows.json` in the data dir (`write_flow_entries`).
   The file's contract is `FlowEntry` in `libs/plugin-api/src/launcher_flows/mod.rs`: `plugin_id`, `title`, `prompt`, `query`, `row_actions`, `icon`.

3. **Launcher.** The launcher reads that file and lists each flow as a card in Apps (`ResultItem::Flow`).
   Enter on the card calls `state.enter_flow` (`plugins/launcher/src/ui/state.rs`). From then on every keystroke is a query.

4. **Query.** `fetch_rows` (`plugins/launcher/src/flow/mod.rs`) posts `{query: <typed text>}` to the tray route `/api/plugins/<plugin_id>/queries/<query>` (`qol_conventions::api_routes::plugin_query`) with a 12 s timeout.
   The tray hands it to the plugin daemon; memory answers it in `plugins/memory/src/app/request.rs` with `{rows: [{title, subtitle, copy, ...}]}`. The launcher keeps at most 8 rows (`MAX_ROWS`).

So the coupling points are a manifest table, a host-written file, and a tray route. The launcher never links the plugin crate, and the plugin never links the launcher. That is the part worth keeping.

### Keys inside a flow today

`apply_flow_key` in `plugins/launcher/src/ui/input.rs`:

- Enter on a row opens its detail (`FlowDetail`).
- Enter again (`FlowActivate`) runs `activate_flow_row` in `plugins/launcher/src/ui/controller.rs`: the first `row_actions` entry if there is one, otherwise it copies `row.copy` (or the title), then hides the launcher.

So copying takes two Enters. A clipboard history needs one.

## What leaks today

The transport is generic; the rendering is not. `plugins/launcher/src/flow/mod.rs` and `plugins/launcher/src/ui/view.rs` know memory's answer shape:

| Launcher code | Memory concept it hard-codes |
|---|---|
| `parse_verdict`, `FlowVerdict::{Answered, Checking, Vague, NoMemory}` | memory's verification outcome (supported, ambiguous, unsupported, pending) |
| `row_sort_key` | rows of kind `"answer"` first, then by `trail[0].at` |
| `trail_of`, `lead_of`, `host_of`, `sources_of`, `nearby_of`, `detail_of` | memory's evidence trail, source count, nearby memories |
| `send_feedback` | memory's `"feedback"` query with a vote of -1 |
| strings in `view.rs` | "true now", "N sources agree", "NO CONFIDENT ANSWER · RELATED MEMORIES", "CHECKING ANSWER", "no memory covers this" |
| `FLOW_WIDTH` 580 | one width for every flow, sized for memory's answer |

A second flow plugin would either look like memory or need launcher edits. Both break the extension promise.

## Proposal: a generic flow row contract

Move the row vocabulary into a neutral contract in `libs/plugin-api/src/launcher_flows/`, next to `FlowEntry`, and have the launcher render only that. Plugins send data; the launcher owns every pixel, so every flow keeps the launcher's look (grounds, card sizes, tile, side panel, focus ring) without copying it.

### Manifest additions

```toml
[launcher]
kind = "flow"
title = "Clipboard"
prompt = "Search clipboard"
query = "rows"
enter = "copy"            # copy | paste | detail | action:<id>; default "detail" keeps memory unchanged
panel = "preview"         # preview | none
detail = "detail"         # optional: the query that answers one row's detail by id

[[launcher.row_actions]]
action = "paste"
key = "shift+enter"
label = "Paste into the last window"

[[launcher.row_actions]]
action = "pin"
key = "ctrl+p"
label = "Pin"

[[launcher.row_actions]]
action = "forget"
key = "delete"
label = "Forget"

[[launcher.row_actions]]
action = "copy"           # keyless: never a key hint; ↵ runs it for a row without copy text
label = "Copy"
input = { id = "{id}" }
```

`RowActionSpec` (`libs/config/src/contract/v1.rs`) already carries `action`, `input`, `label`, `description`, `key` and `when`; the launcher only has to honour `key` for flow rows.
`enter` is the one new verb. `copy` writes the row's `copy` text (or `copy_image`, see below) through gpui's clipboard, shows "Copied …" on the search line, and hides the launcher.
A row without `copy` text (a secret, or a clip too big to send inline) is copied by the plugin: ↵ runs the declared `copy` row action with the row's id and still shows COPIED. A row action without a `key` is never shown as a key hint and never matches a key.

### Row schema (version 1)

```json
{
  "rows": [
    {
      "id": "c_01J…",
      "kind": "link",
      "title": "github.com/qol-tools/qol-monorepo/pull/147",
      "subtitle": "Link · Firefox · 2 min",
      "copy": "https://github.com/qol-tools/qol-monorepo/pull/147",
      "mono": true,
      "group": "Now",
      "tags": [{"text": "Link", "tone": "kind"}, {"text": "Firefox"}],
      "preview": {"text": "fix(monitor): save display mode changes…", "mono": false},
      "facts": [["Copied from", "Firefox, 2 min"], ["Computer", "This computer"]],
      "thumb": "file:///…/thumbs/c_01J….webp",
      "swatch": "#d9822b",
      "masked": false,
      "badge": {"text": "clears in 22 s", "tone": "warning"}
    }
  ],
  "status": {"text": "Capture is paused", "tone": "warning"}
}
```

Every field except `title` is optional, so memory's current rows stay valid.
`kind` picks the tile mark from the launcher's mark set; an unknown kind falls back to a plain mark.
`tone` is one of a closed list (`kind`, `info`, `warning`, `error`) that the launcher maps to theme tokens, so no plugin sends colours, except `swatch`, which is the content itself.
`status` replaces `parse_verdict`: memory sends "CHECKING ANSWER" or "NO CONFIDENT ANSWER" as a status line instead of the launcher deriving it.
The order of `rows` is the plugin's order; `row_sort_key` goes away.

**Rows are small and bounded.** Every answer crosses the local socket under `qol_runtime::local_ipc::MAX_MESSAGE_BYTES` (64 KiB). A row never carries a clip's whole text: titles, app names and file names are capped, `copy` is inline only for a non-secret clip whose escaped text is at most 256 bytes, and the rows answer carries no `preview` for clipboard. `status` is sent only when there is something to say (capture paused), never as a count.

### Row detail (on demand)

A flow that declares `detail = "<query>"` gets each row's detail by id. The launcher posts `{"id": "<row id>"}` to that query for the selected row, off the UI thread, and caches the answer per id for the life of the flow. An answer for a row that is no longer selected, or from an earlier flow, is dropped. While the detail loads the frame keeps its size and stays empty; a failed detail falls back to the row's own fields.

```json
{
  "text": {
    "head": ["line 1", "line 2"],
    "tail": ["line 1273", "line 1284"],
    "lines": 1284,
    "chars": 48210,
    "numbered": true
  },
  "shown_fact": "Size"
}
```

`head` is the first lines and `tail` the last ones, so the sheet's "N lines" divider and the tail's line numbers are real. `lines` and `chars` count the whole clip. `shown_fact` names the fact the launcher extends with how much the frame shows. Both fields are optional; an empty answer means the row has no text to show. The type is `FlowDetail` in `libs/plugin-api/src/launcher_flows/detail.rs`. Clipboard sends at most 40 head lines and 12 tail lines, each capped, so a detail of a 1 MB clip stays far under the message limit; a test builds that worst case.

### Migration of memory

1. Memory's `rows` handler adds `group`, `tags`, `facts`, `preview` and `status` built from what `trail_of`, `sources_of` and `parse_verdict` read today.
2. The launcher renders the generic fields and keeps reading the old ones for one release.
3. The launcher drops `trail_of`, `lead_of`, `host_of`, `sources_of`, `nearby_of`, `detail_of`, `parse_verdict` and the memory strings. `send_feedback` becomes a memory `row_actions` entry with a key.

Memory's own design is out of date since the new launcher; this migration gives it the launcher's look for free.

## The clipboard plugin (`plugins/clipboard`, id `qol-clipboard`)

Headless first, per `docs/plugin-layout.md` and the qol-arch-code skill. The launcher is only one front end; the CLI works on its own.

```text
plugins/clipboard/
  plugin.toml
  src/
    main.rs
    lib.rs
    cli.rs                  # help, doctor, list, get, copy, pin, forget, clear, --json
    app/                    # daemon: owns the watcher and answers "rows", "copy", "pin", "forget"
    config/                 # typed config via qol_config::load_plugin_config_from_env_with_contract
    capture/
      mod.rs                # Watcher facade: next change -> Clip
      platform/
        mod.rs
        linux/              # X11 XFixes selection events; Wayland data-control when present
        macos/              # NSPasteboard changeCount poll
        windows/            # AddClipboardFormatListener
        fallback.rs         # typed "capture unsupported" error
    history/
      mod.rs                # append, search, pin, forget, expiry
      store.rs              # JSON lines + blobs under the plugin data dir
    secrets/
      mod.rs                # sensitivity policy: masked, short expiry, never synced
    sync/
      mod.rs                # peers sync of pinned and recent clips (opt-in)
```

Facts the design leans on:

- **State is files.** History lives under the plugin data dir (`history.jsonl` plus `blobs/` for images), so a daemon restart, a crash or the CLI all see the same history. The daemon keeps no load-bearing state in memory alone.
- **Capture is per platform** behind one facade, with a fallback that returns a typed error; `doctor` reports "capture unsupported here" as a warning, never as ok.
- **Secrets.** A copy marked sensitive by its source (`x-kde-passwordManagerHint`, `ExcludeClipboardContentFromMonitorProcessing`, `org.nspasteboard.ConcealedType`) is stored masked, expires after a short time (default 30 s), is never written to a thumbnail and never syncs. Config can drop secrets entirely.
- **No host mutation.** Reading the clipboard is not a host change. Writing it back is the user's own action, so it carries no lifetime (qol-mission, "a host setting the user changes through qol").
- **Bundled.** No clipboard daemon on the host is assumed; the watcher is in-process.
- **Visible failure.** A capture backend that stops answering surfaces as a toast and a `doctor` failure, and the flow's `status` says "Capture is paused" instead of showing a stale list.
- **Other computers.** Optional peers sync shares pinned clips and recent non-secret clips through the profile's peer channel. Each clip carries its `device`; the row shows "from Laptop" as a tag.

### Its launcher flow

`rows` answers the typed text with the newest matching clips, grouped as the user picks on the canvas (none, time, pinned).
Enter copies (`enter = "copy"`); shift+Enter pastes into the window that had focus before the launcher; ctrl+P pins; delete forgets.
Opening: a Clipboard card in Apps always works. A direct hotkey (for example super+V) runs the plugin's `open` action, which posts `{"plugin": "qol-clipboard"}` to the launcher's generic `flow` action through the tray (`POST /api/plugins/qol-launcher/actions/flow`). `qol-launcher --flow <plugin_id>` opens the same flow from a terminal.

## The locked look

Picked on the canvas in five rounds. Every value comes from the launcher's existing ladder in `libs/theme`; nothing below is a new token except where it says so.

**Frame of the flow.** The flow uses the launcher's full 840 width: the card list on the left (488 wide), the side panel on the right (320 wide), both 534 high under the search field.

**Cards.** Every card is 64 high, whatever the clip. The name line has a fixed 20 px line height, in mono for code-like clips (`mono: true`) and in the launcher's sans otherwise, so mono and sans cards are the same height. The tile shows the kind's mark, the image thumbnail, or the colour swatch. A pinned clip shows a "Pinned" tag; a secret shows its countdown ("clears in 22 s") in the warning ink.

**Nothing moves.** Switching from one clip to the next changes only the content inside fixed slots. The side panel is:

| Slot | Position | Holds |
|---|---|---|
| Head band | top 0, 60 high | 44 tile, kind name, "app · age" (the row subtitle is "Kind · app · age") |
| Clip frame | top 68, down to 10 above the facts | the clip, see below |
| Facts | 4 rows of 22 above the key line | Kind, From, Size, Copied |
| Key line | 22 high, 14 above the bottom | ↵ Copy, space Preview |

The frame keeps its size for every kind: text fills it as a code block, an image covers it from the top left, a colour fills it with its value in a small chip, a link shows the URL and page title, a secret shows dots. The scroll track's space is always reserved and the line gutter is always 48 wide, so neither appears or disappears. Tags never wrap; a long value ellipsizes.

**Everything in a code block.** Every text clip, prose included, shows in mono inside the frame. Numbered lines (logs, code, JSON) get the 48 gutter. Text that does not fit fades out over the last 36 px of the frame.

**Code ground.** The frame sits on the launcher window's own ground (the pane colour, the darkest ground the launcher has) with a hairline under it, so it reads darker than the panel around it.

**The count lives in the Size fact.** There is no separate count line or count chip. Size reads "1,284 lines · 48 KB · 11 shown", "612 characters · more below", or "612 characters, all shown"; for an image it is "856 × 626 · PNG · 13 KB". The plugin sends the counts; the launcher appends the "shown" part to the fact the detail names, from what the frame actually fits.

**Preview.** Space opens a sheet over the whole launcher (8 in from the window's sides, under the search field): the same head band with "↵ copy all" and "esc close" on its right, then the clip's detail in the same frame. A very long clip shows its head and its tail, with their real line numbers, and a "N lines" divider between them. The sheet overlays; nothing under it moves. Esc or space closes it.

**After ↵.** The mode chip in the search field turns from CLIPBOARD to COPIED; the field and list stay as they are, and the launcher hides 350 ms later so the change is seen.

## Launcher work this needs (all generic)

1. Render the row schema above: tile by `kind`, `thumb`, `swatch`, `tags`, `preview`, `facts`, `group` heads, `badge`, `status`.
2. Honour `enter` and per-row-action `key` in `apply_flow_key`.
3. `qol-launcher --flow <plugin_id>` and the daemon's `flow` action (`{"plugin": "<id>"}` via the tray) to open straight into a flow.
4. Let the flow use the launcher's full 840 width and side panel instead of `FLOW_WIDTH` 580.
5. Image copy: `copy_image` (a blob path) written as an image to the clipboard.
6. The `detail` query: fetch the selected row's `FlowDetail` by id, cache it per id, ignore stale answers.
7. ↵ on a row without `copy` text runs the keyless `copy` row action.

Nothing in this list names clipboard or memory.

## Mission checks

| Check | Answer |
|---|---|
| User configures the host? | No. Capture runs in-process on every OS. |
| Host mutation lifetime | None. Reading is passive; writing back is the user's directed change. |
| Restored on exit | Nothing to restore. History is qol's own data in the profile's data dir. |
| Self-contained | No host clipboard manager or service needed. |
| Failures visible | doctor, toast and the flow's status line. |
| Portable vs Resident | History is per profile; capture stops when qol leaves, so a borrowed host keeps nothing. |

## Settled

- Clipboard stays a flow, opened from its card in Apps or straight through the launcher's `flow` action, bound to the plugin's `open` action. No third tab: a tab would make the launcher name one plugin.
- Unpinned clips are kept until there are 200 of them or they are 7 days old, whichever comes first; pinned clips are kept until unpinned. Both are config.
- A secret flagged by its source appears masked and clears after 30 s; `keep_secrets = false` drops them at capture. Secrets never get a thumbnail and never sync.

## Build order

1. `libs/plugin-api`: row schema v1 and the `enter` and `panel` manifest fields, with serde tests that memory's current rows still parse.
2. Launcher: render the schema in the locked look, `enter = "copy"`, per-row-action keys, the space sheet, `--flow <plugin_id>`, full width when `panel = "preview"`. Memory keeps its current look until its migration.
3. `plugins/clipboard`: store, CLI and daemon with Linux X11 capture first; macOS, Windows and Wayland data-control behind the same facade, each starting as a typed-unsupported stub that `doctor` reports as a warning.
4. Memory migration to the generic fields, then removal of the memory-specific launcher code.
