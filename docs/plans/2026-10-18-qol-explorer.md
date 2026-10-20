# qol explorer

qol explorer is a file manager plugin. It works the same on every OS, is driven from the keyboard first, and uses the launcher's search as you type.

## Design sources

- The frozen design is design A, one window: https://claude.ai/artifact/BiECCxJvxS6aDEb2udnS6F (version 3, "Frozen").
- The move-problems page, still waiting for an A or B pick: https://claude.ai/artifact/7KuTqwrMQGm9uGGgpboRRN. Its step 1 (type to jump) is picked.
- The research synthesis, with sourced reports, is at `~/.local/share/design-evolution/qol-monorepo/research/explorer/synthesis.md`.

## Window

The window opens at 1040 by 640 and can be resized. Surfaces have square corners, and every value comes from `qol_theme` (the type ladder, the SPACE ladder, the heights and the grounds). Names use IBM Plex Sans. Mono appears only in keycaps.

- **Band, 52 high.** On the left is the crumb trail: `explorer / home / Git / ...`, set in the settings crumb style (Heading, 20 px). Parent crumbs are faint, separators are at Veil alpha, and the last crumb is ink. On the right is a note at 12.5 px. The band has no bottom key legend, ever.
- **Rail, 200 wide.** Places lists Home and each XDG user folder that exists (Desktop, Documents, Downloads, Music, Pictures, Videos). Devices lists This computer and every mounted drive. Rows are 36 high, with labels in the Label style.
- **List.** A 28-high header reads Name, Kind and Size. Rows are 36 high and are drawn by a uniform-height virtual list, so the cost of a row does not depend on folder size. A picked row uses the band fill and the cursor row uses the well fill. Files show their size in bytes. Folders show "N items", counted in the background.

## Keys in M1

| Key | What it does |
|---|---|
| up / down | move the cursor |
| enter | open the folder, or open the file in its default app (`qol_apps::open_with_default_app`) |
| backspace, alt+up | go up one folder |
| alt+left / alt+right | go back or forward |
| a to z | type to jump |
| ctrl+g, click the trail | type a path |
| space | pick or unpick the cursor row |
| shift+up / shift+down | pick a run |
| ctrl+click | pick one more |
| ctrl+a | pick everything |
| esc | pick nothing |
| ctrl+h | show or hide hidden files |

## Rules

- **Type to jump.** Typed characters build a run, and the run resets after 1 s without typing. The cursor jumps to the first name, in list order and ignoring case, that starts with the run. If the run is one letter repeated and no name starts with the whole run, the cursor moves to the next name after it that starts with that letter, wrapping around. No input box appears. In the cursor row only, the typed letters are drawn at TEXT_DISPLAY (20 px) weight 600 in the accent color, and the rest of the name stays ink.
- **Path field.** The trail turns into a text field holding the current path, with `~` standing for home. Below it, a menu suggests the subfolders of the typed parent folder whose names start with the last segment, showing names only. Tab completes the top suggestion, enter goes there, and esc puts the trail back.
- **Listing.** A folder is read on a background thread and its rows stream in. Navigating away cancels the read. While a read is still running the list shows a loading state, never "empty".
- **Sorting.** Folders come first. Names sort naturally (file2 before file10) and ignore case. Sort keys are computed once per listing, so sorting 100,000 names is a plain key compare.
- **Hidden files.** Which files count as hidden is a per-OS rule kept behind the platform adapter: dot names on Linux and macOS, plus `UF_HIDDEN` on macOS and the Hidden attribute on Windows.
- **Memory.** Each history entry remembers its cursor and scroll position, and both come back when you return to that folder. Picks are kept by path, so they survive navigation.
- **Failures.** A folder that cannot be read says why, for example "No permission to read ai/ollama/blobs". Nothing fails silently.
- **Drives.** The Devices list comes from a platform adapter: mountinfo on Linux (real disks under /media, /mnt and /run/media, plus /), /Volumes on macOS, and drive letters on Windows. Free space comes from `qol_platform` statvfs or its equivalent.

## Platforms

All the code compiles for Linux, macOS and Windows, and per-OS behavior sits behind `platform/mod.rs` adapters (qol-arch-code). The manifest declares only `linux` until a guest VM has verified each of the others.

## Milestones

1. **M1, browse and pick (this branch).** The window, band, rail, list, keys, type to jump, the path field, picks and the listing engine.
2. **M2, file operations.** This milestone waits for the pick on the move-problems page. It covers a job engine with a ctrl+j list, the name-clash sheet, and cross-drive moves done as copy, verify, then delete. It also covers the OS trash, with an explicit "delete permanently?" prompt when the trash refuses, an undo journal and no-clobber renames. The engine notes and the research rules apply.
3. **M3, search.** ctrl+f opens a full-window search that keeps only the folders holding matches. It reuses the `qol_search` scorer over an index of names plus parent ids.

## Layout

```text
plugins/explorer/src/
  main.rs cli.rs lib.rs
  app/        window launch and process setup
  listing/    background read, natural sort keys, streaming, cancel
  nav/        history, cursor, picks, type to jump, path parse and suggestions
  places/     rail model; places/platform/ for drives, free space and hidden files
  ui/         window, band, rail, list, path field
```

The crumb trail, both its read-only mode and its editable path mode, is a shared `qol-gpui` component. The settings panel moves onto that component with no visible change.
