# qol-cli

The `qol` CLI: dev console (`qol dev`), emu workflows (`qol emu`), doctor, trace.

## Dev console design rules

- **Every page renders through `panel()` in `dev_console.rs`.** Never build a
  page block with raw `Block::bordered()` - the shared helper owns the border,
  title badge, and padding, so pages cannot drift apart visually. (The floating
  keys badge is the one exception; it is an overlay, not a page.)
- **Page padding is `PANEL_PADDING`, applied only inside `panel()`.** Any code
  that windows or wraps content against the panel area must derive sizes from
  `panel_inner_height()` / `panel_inner_width()`, never from
  `area.height - 2` arithmetic.
- **One line per list item.** Detail lines below an item are reserved for
  failure states; healthy items earn exactly one line. Static facts that never
  change between renders (paths, versions) belong in `qol emu doctor` or the
  empty state, not on every frame.
