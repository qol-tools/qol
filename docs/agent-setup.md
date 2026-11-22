# Agent setup

The agent setup is which agent accounts a person has, which keyring entry each one reads, and the defaults every launch of an agent gets.
It lives in one JSON file inside the synced qol profile, so every PC that syncs the profile gets the same setup with no hand edit.
Secret values never go in it: the file names keyring entries, and each PC holds their values in its own keyring.

This document is the contract for that file.
qol (`libs/agent-launch`) reads it, and the account-profiles plugin, which builds the bash and zsh launchers, reads and writes it.

## Where the file is

The file is `<profile root>/<active profile>/core/agent-setup.json`.

- The profile root is `profile` in the qol config folder:
  `~/.config/qol-tray/profile` on Linux, `~/Library/Application Support/qol-tray/profile` on macOS and `%APPDATA%\qol-tray\profile` on Windows.
- The active profile is the name in the `active` file of the profile root, `default` when that file is missing or does not hold a safe name (letters, digits, `-` and `_`).
  Switching the active profile switches the accounts too, so the `work` profile can hold its own.
- `core/` syncs to every system, and its JSON files merge field by field (see "How sync settles a change" below).
  `device/` never syncs, so the file never goes there.
  `os/<linux|macos|windows>/agent-setup.json` is left free for a later overlay of values that differ per system; nothing reads it today.

A program that is not qol finds the file with `qol agents setup-path` and never copies these rules:

```
$ qol agents setup-path
/home/me/.config/qol-tray/profile/default/core/agent-setup.json
$ qol agents setup-path --json
{"exists":true,"path":"/home/me/.config/qol-tray/profile/default/core/agent-setup.json","profile":"default"}
```

- It prints the path whether or not the file exists yet, and exits 0.
- It exits non-zero when it is given an argument or the qol config folder cannot be found.
- A `qol` too old to know the verb exits 1 with an error on stderr that starts with ``unknown qol agents subcommand `setup-path` ``, or exits 64 for `--json`.
- A program that finds no `qol`, or a `qol` too old to know the verb, keeps whatever it did before this file existed.

## Schema, version 1

```json
{
  "version": 1,
  "accounts": {
    "clauded": {
      "harness": "claude",
      "home": "~/.claude",
      "purpose": "personal account",
      "marketplaces": ["claude-plugins-official", "kmrh47-skills", "qol-skills"]
    },
    "claudedd": {
      "harness": "claude",
      "home": "~/.claude-deepmind",
      "purpose": "DeepMind Teams account",
      "key": "claude-code-deepmind",
      "marketplaces": ["claude-plugins-official", "kmrh47-skills", "qol-skills"]
    }
  },
  "launch": {
    "claude": {
      "env": { "MAX_THINKING_TOKENS": "63999" },
      "args": ["--append-system-prompt", "..."]
    }
  }
}
```

`accounts` is a map keyed by the account name, never a list.
Sync merges a map key by key but treats a list as one value, so two PCs that each add a different account merge cleanly.
Sync rewrites a merged file with its keys sorted, so no reader may depend on the order of accounts.

| Field | Read by | Meaning |
|---|---|---|
| `version` | qol, account-profiles | `1`. Adding a field keeps `1`. qol does not use a file whose version is missing or higher, and logs why; account-profiles refuses to rewrite one. |
| account name (the map key) | account-profiles | The launcher command, such as `claudedd`. |
| `harness` | qol, account-profiles | Required: `claude` today, later `codex`, `kimi` or `pi`. A reader skips an account whose harness it does not handle. |
| `home` | qol, account-profiles | The account's home folder (`CLAUDE_CONFIG_DIR` for claude). Write `~/` and the rest with forward slashes; an absolute path fits only one system, so writers refuse one. |
| `key` | qol, account-profiles | Optional. The name of the keyring entry that holds the account's token, never the token. Without it the account uses its own `/login`. |
| `token_env` | qol, account-profiles | Optional. The variable the key's value is exported as; `CLAUDE_CODE_OAUTH_TOKEN` when absent, `ANTHROPIC_API_KEY` for an API key account. |
| `purpose` | account-profiles | Optional. A line describing the account; account-profiles asks for it when a person adds an account, and every reader accepts an account without one. |
| `marketplaces` | account-profiles | The marketplace names the account uses; today the kitty marketplace refresh helper reads them. |
| `launch.<harness>.env`, `.args` | account-profiles | Variables and argument words every launch of that harness gets. Sessions that qol starts do not get them yet. |

Two accounts of one harness must not share a home.
qol refuses to launch a home that two accounts name, and names both.

### Reserved names

These names are reserved for later sections, so they can arrive without a version bump:

- per account: `plugins` (a map of `plugin@marketplace` to on or off), `settings` (the `settings.json` keys the file owns), `rules` (the account's global `CLAUDE.md` text), `launch` (env and args that override `launch.<harness>` for this account) and `secrets` (a map of variable name to keyring entry, for harnesses such as pi that read several keys; `key` with `token_env` stays as the one-key form);
- at the top: `marketplaces` (a map of marketplace name to its source; an account's `marketplaces` list names them).

Codex, kimi and pi accounts are accounts with their own `harness`, and their launch defaults go in `launch.codex`, `launch.kimi` and `launch.pi`.

### Rules for every reader and writer

- A reader ignores every field it does not know; it never rejects the file for one.
- A writer keeps every field it does not know when it rewrites the file, at the top and inside each account.
  This is what lets the reserved sections arrive later without a version bump: an older writer passes them through untouched.
- A writer writes the whole file to a temporary file in the same folder and renames it over the old one, because the tray commits whatever is in the profile folder every 10 seconds.
- Nobody writes a secret value into the file.

## How qol reads accounts

For each launch of claude with a config dir, qol takes the first of these that names that home:

1. `[claude_accounts."<home>"]` in `sessions.toml`, this PC's own override.
   It wins over the synced file, so a PC can differ from the others.
2. The synced `agent-setup.json`, when it exists and is version 1.
   An account with a `key` makes the launch read that key; when this PC's keyring does not give it, the launch is refused and the error names the key to store.
   An account without a `key` maps nothing, and the launch uses the caller's own login.
   While this file is usable, `profiles.json` is not read at all.
3. `~/.config/account-profiles/profiles.json`, only while the synced file is missing or unusable.
   A profile whose key cannot be read is skipped, as before this file existed.
4. Nothing: the launch runs under the login of the session that started it.

`agents.toml` (see `docs/agent-homes.md`) stays a separate, machine-level file: it says which homes qol-memory keeps apart, and an account in this file does not become a memory home.

During the move, `profiles.json` becomes output that account-profiles generates from this file, like its `init.sh`, for qol builds that do not read this file yet.
Its first install creates this file from `profiles.json` when the file is missing, so every home that had a key keeps it.

## Homes and keys per system

- `~/` expands against the user's home folder (`/home/<name>`, `/Users/<name>`, `C:\Users\<name>`), and the rest joins with that system's separator.
  qol compares the expanded path with the launch's config dir component by component.
- A key name means the same keyring entry everywhere: the Secret Service entry with attribute `service` set to the name on Linux (`secret-tool`), the login Keychain generic password with that service on macOS (`security -s`), and the generic credential with that target name on Windows (Credential Manager).
- The bash and zsh launchers exist only on Linux and macOS; on Windows only sessions that qol launches read the file.

## How sync settles a change

There are no timestamps.
When two PCs both changed the profile, sync compares each JSON field with the last commit both PCs share (the git merge base):

- a field changed on one side only takes that side's value;
- a field changed on both sides to the same value keeps it;
- a field changed on both sides to different values is a conflict.

A list or a text value is one field, so two edits to the same list clash even when they touch different entries.

A conflict stops the whole profile's sync, not only this file: nothing from the other PC is applied and nothing is pushed, this PC's files stay as they are, and the tray says how many settings need review.
The person then picks this PC's value or the other's for each conflicting field in the tray; only then is a backup of both sides written to `sync/backups`, and the merged profile is applied and pushed.

The tray pulls when it starts (a setting) and every time it pushes, and it pushes within 10 seconds of a change.
A PC that changes nothing sees the other PCs' changes when its tray next starts.

## Later sections, both ways

The later step writes the setup into each account folder and reads changes back, and the sync above decides which change stands; there is no separate rule that the file wins.

- Apply: after a pull brought changes from another PC, the tray writes each section into the account's home.
- Capture: when an account's home changes, the tray writes the changed values into this file, and the next push carries them.

| Section | Account folder side |
|---|---|
| `plugins` | `enabledPlugins` in `<home>/settings.json`; both are maps, so they merge plugin by plugin. |
| `marketplaces` | `extraKnownMarketplaces` in `settings.json` and `plugins/known_marketplaces.json`; only the name and source are captured, never the install path on this PC. |
| `settings` | Only the keys already listed in the section are captured, because Claude Code writes paths of this PC into `settings.json`; adding a key is a deliberate act. |
| `rules` | `<home>/CLAUDE.md`; the whole text is one field. |
