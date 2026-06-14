# qol-cli

The `qol` CLI: dev console (`qol dev`), emu workflows (`qol emu`), doctor, trace.

## Dev console design rules

- **One shell, many sign-boxes.** `draw()` renders a single persistent
  `Shell { title }` (the `qol dev` frame) and hands its inner `Rect` to the
  view. Every view paints its content as a `SignBox` via
  `view_box(frame, shell, title, lines, accent)` - the dashboard menu is just
  the Dashboard view's box. Never hand-roll `Block::bordered()` or a bespoke
  title; `Shell`, `SignBox`, and the floating keys badge all compose the same
  `Sign`, so frames cannot drift apart. A new view is a `view_box` call.
- **A frame fills its rect.** `view_box` and `draw_stream` fill the whole
  `Rect` they are handed (full width + height) - the rect IS the size knob.
  A view that wants a shorter box passes a shorter rect (see `draw_emu_detail`
  splitting its area into an info rect and a log rect); never shrink-wrap to
  content width or row count.
- **Size scrolling from `SignBox::capacity(shell.height)`.** Anything that
  windows a list against its box (visible rows, highlight width) derives from
  the box, never `area.height - N` arithmetic - that keeps the `TITLE_CAP` row
  and `SignBox::CHROME_ROWS` chrome in one place.
- **One line per list item.** Detail lines below an item are reserved for
  failure states; healthy items earn exactly one line. Static facts that never
  change between renders (paths, versions) belong in `qol emu doctor` or the
  empty state, not on every frame.
