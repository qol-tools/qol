# CLI Sessions state contract

`Status::definition()` in `plugins/cli-sessions/src/session/status.rs` owns the exhaustive mapping from session state to label, sort priority, human-attention policy, idle policy, and shared palette roles. Row dots, collapsed-panel priority, counters, notifications, and registry summaries consume this definition. `tests/state_mapping.rs` verifies every state against both native theme palettes.

Harness identity and session state are separate signals. Harness tabs consume `CliTool.label` and `CliTool.accent` from `qol-terminal-sessions`: pastel orange for Claude, pastel blue for Codex, and pastel pink for Pi; status dots consume semantic colors from `qol-theme`. Render both through the shared GPUI kit. Never substitute a harness accent for a status color or copy RGB values into a plugin.

Human input or approval takes precedence over bridge state. An open driver loop is coordinating; a completed delegated lane awaits agent review. Neither requests human attention. Preserve the underlying runtime status separately so closing a loop and acknowledging a session do not corrupt activity tracking.

The shared information role identifies orchestration and services, success identifies active work, warning identifies a completed turn awaiting the user, danger identifies an explicit request for input, and muted text identifies idle or acknowledged sessions. Theme selection colors remain independent of runtime state.

Display priority, highest first: needs you → your turn → awaiting agent review → coordinating agents → working → service/live → idle/unknown → acknowledged. Bridge membership does not override the displayed state’s priority. The open panel follows registry ordering on every update; selection follows session identity. Within a state, harness sessions precede generic terminals, with stable session-ID ordering.

Session rows use the shared full-width kit row and selection treatment. Their status lamps use the shared lamp size. Working, coordinating, and service lamps keep a steady center with an expanding, fading ring; all waiting, idle, and acknowledged lamps remain steady. `Status::is_active()` owns this motion policy. The collapsed indicator follows the same mapping and motion. The surface supplies one shared, bounded activity clock; individual rings never schedule display-refresh loops.

Rows separate session identity from state: the name leads, the pastel harness tab sits at the left edge with bottom-to-top text, and status and elapsed time share the second line. Shared separators distinguish resting rows; the opt-in shared tinted selection remains full width and preserves the session state hue.

The footer exposes a clickable `A ack` action for the selected YourTurn session. Acknowledging anchors selection to that session as it moves to the acknowledged tier. A held shortcut acknowledges only once per press. The action does not apply to working sessions or dismiss an explicit input request. `CLI_SESSIONS_ACK` records successful acknowledgements without session contents.

Rows reserve a fixed-width harness tab, trailing time column, and separate agent-control slot. Harness changes, elapsed-time digit changes, and bridge membership do not move the name or state columns. The harness tab stays present for generic terminals, and timestamps use the shared mono font. Flat tabs extend across the reserved row border, independent of content padding, and meet the top and bottom edges of a faint state-colored row wash and use the shared 16px spacing and 10.5px identity text tokens. Labels are measured and fitted with internal padding before rotation. `Kit::row_selected_tinted` opts into the shared tinted row palette for resting and selected rows; standard row selection keeps its default appearance. Rotated labels are cached by text in the shared GPUI component, with GPUI handling image rendering. Disabled acknowledgement retains its footer position.

Codex’s visible empty composer is live ready evidence, including below `/status`. The informational status banner must not freeze an earlier input request as historical content. Explicit interrupt and numbered-choice controls take precedence; ready evidence clears a dismissed input request after the normal settling grace.

State row washes stay faint in every state. Selection adds a contrasting outline derived from the state hue and the current theme’s text ink, rather than relying on a much stronger background tint.

Row backgrounds, hover washes, and selection outlines take their tone from `Status::definition().colors`, matching the status dot. The vertical label alone retains the harness accent.

Sessions inset the opt-in selection outline after the harness tab with `Kit::row_selected_tinted_after`, leaving the full-height harness color free of the state-colored border. The content and hit area keep the same geometry.
