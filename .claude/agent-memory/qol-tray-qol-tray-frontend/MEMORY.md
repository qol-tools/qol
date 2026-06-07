
## 2026-06-07
- Pre-existing failing tests can mask new regressions; before treating a fail as your fault, git stash and re-run on clean main to confirm whether it pre-existed.
- `grid-template-columns: minmax(min, max)` with a fixed max leaves dead space at wide widths; use `1fr` as the max to expand cards to fill the row.
- After `git stash pop`, untracked files (screenshots, .playwright-mcp/) reappear in status; clean up debug artifacts before reporting done so the diff stays minimal.

## 2026-06-07
- When `cargo fmt` runs across a workspace it can silently modify files outside the directory you're cd'd into; re-check `git status` after fmt before staging.
- `Number.isFinite(Infinity)` is false in JS (unlike Rust's f32 where Infinity is finite), so `isFinite`-guarded clamps fall to default on ±Infinity — write tests to that contract.
- User-supplied premises about backend state ("reads via OnceLock and caches") can be stale; verify against the working tree with grep before designing around the claim.
