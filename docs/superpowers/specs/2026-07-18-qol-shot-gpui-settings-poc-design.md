# qol-shot gpui settings POC

## Goal

A qol Shortcut (exported to the launcher as an installed app) opens a native gpui settings surface for qol-shot instead of the browser settings page.
This is the POC for "plugin config in gpui": one plugin, no new schema, no new store.

## Single source of truth (nothing new)

- Field model: `qol-config.toml`, parsed by the existing `qol_config` contract into `ResolvedConfig` (`resolve_config`), the same model the webview auto-config renders.
- Values: `config.json` in the existing plugin config paths; the panel reads merged values and writes the same file the tray writes.
- Window layer: `qol_gpui::surface` from the toast POC, extended with a focus-taking `Panel` kind that reuses the proven `open_window_with_focus` path.
- Launch path: the existing `settings` action and Shortcut launcher export; no new channel.

## Behavior

- `qol-shot settings` with the daemon live opens the gpui panel in the daemon process (single-binary daemons route every action through the socket).
- Headless `qol-shot settings` (no daemon) keeps today's behavior: open the qol-tray settings URL.
- The panel renders qol-shot's sections and fields from `ResolvedConfig`, implementing only the kinds qol-shot uses: `boolean`, `select`, `number`, `string`, `string_array`.
- Keyboard-first: arrows move between fields, space toggles booleans, left/right cycle selects and step numbers, enter edits text, Escape closes the panel.
- Edits write `config.json` immediately (read-modify-write of the existing file), matching the webview's save-on-change semantics.
- The webview settings page stays untouched and continues to work.

## Non-goals

- Other plugins, other field kinds (`action`, `status`, `list`, `qr_code`, `color`, `object_array`, `object_map`).
- Shipping a Shortcut; the user creates one for the `settings` action with launcher export (existing feature).
- Replacing or changing the web UI.

## Error handling

- If the panel cannot open, fall back to the browser settings URL and log the failure.
- A malformed `config.json` falls back to contract defaults for display; the first edit rewrites a valid file.

## Testing

- Unit tests: contract-to-panel row mapping for qol-shot's kinds, value merge (defaults + config.json), and edit-apply producing the expected JSON.
- Existing contract tests keep validating `qol-config.toml`.
- Visual and focus behavior verified in a guest lane (or host if explicitly requested).
