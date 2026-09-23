# CLI Sessions state contract

`Status::definition()` in `plugins/cli-sessions/src/session/status.rs` owns the exhaustive mapping from session state to label, sort priority, human-attention policy, idle policy, and shared palette roles. Row dots, counters, notifications, and registry summaries consume this definition. `tests/state_mapping.rs` verifies every state against both native theme palettes.

Harness identity and session state are separate signals. Harness tabs consume `CliTool.label` and `CliTool.accent` from `qol-terminal-sessions`: pastel orange for Claude, pastel blue for Codex, and pastel pink for Pi; status dots consume semantic colors from `qol-theme`. Render both through the shared GPUI kit. Never substitute a harness accent for a status color or copy RGB values into a plugin.

Human input or approval takes precedence over bridge state. An open driver loop is coordinating; a completed delegated lane awaits agent review. Neither requests human attention. Preserve the underlying runtime status separately so closing a loop and acknowledging a session do not corrupt activity tracking.

The shared information role identifies orchestration and services, success identifies active work, warning identifies a completed turn awaiting the user, danger identifies an explicit request for input, and muted text identifies idle or acknowledged sessions. Theme selection colors remain independent of runtime state.

Display priority, highest first: needs you → your turn → awaiting agent review → coordinating agents → working → service/live → idle/unknown → acknowledged. Bridge membership does not override the displayed state’s priority. The open panel follows registry ordering on every update; selection follows session identity. Within a state, harness sessions precede generic terminals, with stable session-ID ordering.

Session rows use the shared full-width kit row and selection treatment. Their status lamps use the shared lamp size. Working, coordinating, and service lamps keep a steady center with an expanding, fading ring; all waiting, idle, and acknowledged lamps remain steady. `Status::is_active()` owns this motion policy. The collapsed header has no animated indicator and does not run the activity clock. The surface supplies one shared, bounded activity clock; individual rings never schedule display-refresh loops.

Rows separate session identity from state: the name leads, the pastel harness tab sits at the left edge with bottom-to-top text, and status and elapsed time share the second line. Shared separators distinguish resting rows; the opt-in shared tinted selection remains full width and preserves the session state hue.

Expanded and collapsed modes render the same header: Sessions, the shared live-count chip, and window controls. Live counts all non-idle states, including sessions awaiting input; it uses the same definition and presentation in both modes. Collapsing hides only the rows and footer. Header height, width, padding, colors, and square frame remain unchanged; no separate waiting summary is introduced. Only the title area starts a drag or accepts click-to-expand.

Close and the expand/collapse toggle stay at the top right in both expanded and collapsed modes, using the same control renderer and shared GPUI styling. The chevron points up to collapse and down to expand. Control clicks do not initiate strip dragging or trigger the strip’s click-to-expand handler. The footer contains shortcut hints, not action buttons. Shortcuts remain A, Alt+S, and platform+W (or Escape), with Enter focusing the selected session. A clickable `your turn ✓` pill replaces the row status only for YourTurn. Acknowledging targets that row, removes the pill, and anchors selection to the session as it moves into the inactive acknowledged tier at the bottom. A held shortcut acknowledges only once per press. The action does not dismiss an explicit input request. `CLI_SESSIONS_ACK` records successful acknowledgements without session contents.

Rows reserve a fixed-width harness tab, trailing time column, and separate agent-control slot. Harness changes, elapsed-time digit changes, and bridge membership do not move the name or state columns. The harness tab stays present for generic terminals, and timestamps use the shared mono font. Flat tabs extend across the reserved row border, independent of content padding, and meet the top and bottom edges of a faint state-colored row wash and use the shared 16px spacing and 10.5px identity text tokens. Labels are measured and fitted with internal padding before rotation. `Kit::row_selected_tinted` opts into the shared tinted row palette for resting and selected rows; standard row selection keeps its default appearance. Rotated labels are cached by text in the shared GPUI component, with GPUI handling image rendering. The status line reserves the same height whether it contains the acknowledgement pill or plain status text.

Codex’s visible empty composer is live ready evidence, including below `/status`. The informational status banner must not freeze an earlier input request as historical content. Explicit interrupt and numbered-choice controls take precedence; ready evidence clears a dismissed input request after the normal settling grace.

State row washes stay faint in every state. Selection adds a contrasting outline derived from the state hue and the current theme’s text ink, rather than relying on a much stronger background tint.

Row backgrounds, hover washes, and selection outlines take their tone from `Status::definition().colors`, matching the status dot. The vertical label alone retains the harness accent.

Sessions inset the opt-in selection outline after the harness tab with `Kit::row_selected_tinted_after`, leaving the full-height harness color free of the state-colored border. The content and hit area keep the same geometry.

Collapse intent and native resize completion are distinct. Render the body according to the current viewport height until the window manager delivers the new size, so an in-flight collapse never paints an empty full-height strip. The collapsed header stays top-aligned throughout the resize. Existing collapse and native bounds traces record the transition.

Both modes retain one panel root and the same control element identities. Sharing a header function alone is insufficient: changing its ancestor identity resets retained hover and focus state. Collapse changes body visibility and the toggle glyph without replacing the header subtree.

Header control focus styling uses GPUI `focus`, which matches only the focused button. `in_focus` also matches descendants of the focused panel and would incorrectly illuminate both controls when collapse restores panel focus. Pointer hover remains independent.

The footer is a compact shortcut legend using shared `Kit::hint_bar_compact` and `Kit::hint_label_first`. Action labels precede compact mono keycaps with a faint wash and no border in one centered group with consistent spacing. The compact variant uses the shared inline height; standard hint bars and boxed keycaps retain their defaults.
