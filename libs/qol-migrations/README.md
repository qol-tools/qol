# qol-migrations

On-disk and cloud-stored data migrations between qol-tray releases.

## Why this crate exists

When a qol-tray release changes the on-disk config layout (file paths, JSON schemas, directory structure) or the cloud backend that stores user data (gist vs repo, schema version), users upgrading from the previous release have data in the old shape. The daemon must either:

1. Refuse to start until something migrates the data, or
2. Read both old and new shapes forever, interweaving migration code into every feature module.

Option 2 is what most projects drift into. It rots the codebase: every feature gradually accumulates "if old format do this, if new format do that" branches, and removing the legacy branch becomes scary because callers everywhere depend on it.

This crate enforces option 1, isolated from `qol-tray/src/`. Migration logic lives here and only here. The daemon calls two functions at boot; if there's nothing to do, the calls are free. Pruning a migration is a single `rm -rf` of one folder.

## Two-phase boot model

Migrations split into two phases because the runtime conditions they need are different:

- **PreFlight** - synchronous, file-only, runs before any feature module reads config. Used for on-disk format and layout changes. No network, no auth, no daemon. Must complete before housekeeping creates the new layout's empty directories.
- **PostAuth** - asynchronous, network-aware, runs after the GitHub auth token loads. Used for backend swaps (gist to repo) and recovery of cloud-stored data. Requires an authenticated HTTP client.

Call sites in qol-tray:

```rust
// pre-flight (sync, in main.rs before housekeeping)
qol_migrations::run_pre_flight(&config_dir)?;

// post-auth (async, after github_auth loads)
qol_migrations::run_post_auth(&MigrationContext {
    config_dir,
    github_token,
    http,
}).await?;
```

Both phases share the same journal, lock, and registry machinery. A migration declares its phase via the `Phase` enum on its trait impl; the runner only invokes migrations whose phase matches the current call.

## Sliding-window release migrations

- Each release that breaks a contract ships exactly one migration (file or cloud).
- The supported upgrade window is the previous **N** releases (3 by default). Anything older: the daemon refuses to start with a clear "upgrade to vX first" message and points the user at an intermediate release.
- Aged-out migrations are deleted in the same commit that introduces a new one. The crate never amasses migration code forever.

## Pitfall guards

Each guard exists because some other project paid for the lesson.

**Strict in-order chain.** Migrations apply in registry order, period. No "newer migration first" or "skip the ones that look done". Flyway and goose both taught the industry that out-of-order application "sometimes works", which is the worst possible failure mode: it works on the dev's laptop and corrupts production.

**`OLDEST_SUPPORTED` const refuses old installs.** When a user's data predates the window, the runner returns a refuse-to-start error naming the intermediate release they must upgrade through first. We do not fake-stamp the journal to pretend the missing migrations ran. Alembic and Django's squash features made this trap famous: silently marking old migrations as applied leaves data in an indeterminate state that no later migration can detect.

**Per-step `.done` journal via rename-into-place.** Each completed migration writes `config_dir/migrations/applied/<name>.done` atomically (write temp, fsync, rename). Crash recovery consults the journal, not the filesystem shape. Filesystems have no transactions; a half-migrated layout can look "almost right" to a naive applies() check, and re-running the migration on partial state is how data gets duplicated or lost.

**fs4 exclusive lock on `config_dir/.migration-lock` for both phases.** A tray app that the user double-launches during an update, or a stale daemon that didn't exit cleanly, must not race a fresh daemon's migration run. Flyway's dual-instance race is the canonical example: two runners both think they're alone, both apply the same step, one wins the rename and the other corrupts the journal.

**Install-id sentinel on remotes.** Before reading or writing any cloud-stored data, the cloud migrations write and verify a `MarkerFile { install_id, profile_id, schema_version }` on the remote. If the marker disagrees with this install, the migration aborts rather than stomp on someone else's bucket. Mastodon's S3 cross-account-stomp class of incident is the lesson here.

**Backend abstraction (`trait GistStore`).** Cloud migrations talk to a `GistStore` trait with `MemoryGistStore` for tests and `GitHubGistStore` for production. Tests never mock at the HTTP layer; they swap the backend. HTTP-layer mocking lets bugs in JSON shape, pagination, and error mapping leak past tests.

**Cross-OS portability helpers.** Sync stability between Linux, macOS and Windows hinges on a few normalisations: profile names go through NFC unicode normalisation before being compared or written, repositories get a `.gitattributes` that enforces LF endings, and path helpers normalise separators. Without these, the same profile name encoded two ways on two OSes produces two profiles, and a CRLF auto-conversion on Windows produces a sync loop.

## Folder layout

```
src/
  lib.rs                         trait + Phase + Registries + runners + OLDEST_SUPPORTED
  fs_util.rs                     archive helpers
  journal.rs                     .done markers
  lock.rs                        fs4 wrapper
  sentinel.rs                    install-id MarkerFile
  cloud/gist_store/              {mod,memory,github}.rs - GistStore trait + impls
  transforms/gist_v1_to_layout.rs  pure gist JSON -> {path -> bytes} map
  portability/                   unicode.rs, paths.rs, gitattributes.rs
  v3_15_to_v3_16/                file migration (PreFlight)
  v3_15_to_v3_16_gist_to_repo/   cloud migration (PostAuth) - added by parallel assembly agent
fixtures/<future migration>/before/, after/  (recommended for big migrations)
```

## Adding a new migration

1. Pick the trait. `FileMigration` for on-disk changes (sync, PreFlight). `CloudMigration` for anything that touches a remote (async, PostAuth, takes the `MigrationContext`). If a release needs both, ship two migrations and let them run in their respective phases.
2. Folder-per-migration default. `mkdir src/vN_to_vNplus1_<short_name>/` and create `mod.rs`. Folder lets the migration grow aux files (transforms, schema converters, split tests) without polluting siblings, and pruning is a single `rm -rf`.
3. Register it. Append the migration to the appropriate registry (`file_registry()` or `cloud_registry()`) in `src/lib.rs`. Registry order is application order; never reorder.
4. Tests. Cover both `applies()` paths (true AND false), and a full `migrate()` round trip asserting archive contents and the resulting config-dir or remote state. Cloud migrations use `MemoryGistStore`. Inline tests in `mod.rs` until they grow past ~150 lines, then split into `tests.rs`.
5. Prune. If this release pushes a migration outside the supported window, `rm -rf src/vN_oldest/ fixtures/vN_oldest/` in the same commit. Drop the entry from the registry. Bump the `qol-migrations` minor version.

## Why a separate crate, not an HTTP service

PreFlight has no daemon to talk to: it runs before the daemon boots. The new daemon cannot start until the layout matches, and the old daemon has already exited. An HTTP-based microservice has nothing to bind to and no client to call it.

PostAuth could technically be a service (the daemon is up by then), but the lifecycle penalty of starting one - extra process, extra socket, extra failure surface - outweighs the cost of an in-process async call. Both phases are consumed two ways:

- As a library from the qol-tray daemon (one call per phase at boot).
- As a standalone CLI via the `qol-tray-migrate` binary in the qol-tray repo, for manual `--dry-run` debugging or running on a config dir other than the default.

Both paths share the same registries and the same migration implementations.
