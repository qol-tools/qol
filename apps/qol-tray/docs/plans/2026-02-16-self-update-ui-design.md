# Self-Update UI

## Location

Sidebar footer, inline next to version label. Always visible.

## States

| State | Display |
|-------|---------|
| Checking | `v1.4.4` · spinner |
| Up to date | `v1.4.4` · `[↻] Check for updates` |
| Update available | `v1.4.4` · `[⬇] Update (1.4.5)` (blue) |
| Downloading | `v1.4.4` · spinner (morphed from ⬇) |
| Error | `v1.4.4` · `[↻] Check for updates` (retry) |

## Morph Animation

The `⬇` download button and spinner share the same `.refresh-btn` circle shape. Adding `.spinning` transitions `color → transparent` and animates `border-top-color → accent`. No shape change — smooth morph.

## Backend

- `GET /check-update` — runs `check_for_updates()`, returns `{ available: bool, latest: string | null }`
- `POST /self-update` — runs `download_and_install()`
- Startup auto-check already exists; endpoint exposes cached result from `OnceLock`

## Frontend

- `sidebar.js` renders version + button based on update state
- `main.js` calls `/check-update` on init, stores result
- Button click on `⬇` calls `POST /self-update`, morphs to spinner
- Button click on `↻` calls `GET /check-update`, shows spinner during fetch

## Existing Infrastructure

- `updates::check_for_updates()` — checks GitHub releases API
- `updates::latest_version()` — returns cached latest version
- `updates::download_and_install()` — downloads .deb, `pkexec dpkg -i`, restarts (Linux)
- `GET /version` — returns `CARGO_PKG_VERSION`
- Sidebar already renders version label at bottom
