<div align="center">

# Update Manager

See what updates wait, install them when you choose, and restart when it suits you.

</div>

## Quick start

```sh
qol-update-manager status
qol-update-manager settings
qol-update-manager doctor
```

## Settings

- The first page lists the first 6 waiting updates, what finishes at the next restart, and the last run.
- Install all starts at once; it opens a page of warnings first when an update needs a restart, carries a caution, loses an administrator's hold or drops the sound.
- Each update's page shows its warnings, then Install, Skip this version and Never update.
- Skip and Never write Mint's own ignore list, so Mint's Update Manager and its nightly run skip the update too. Skipped lists them, and Update again takes one off.
- The waiting page tags every update with its updater when they differ: Update Manager, Flatpak or firmware. A list longer than 40 shows 40 at a time.
- Restart tonight waits for Mint's own nightly run; Restart now asks first.
- Hours and Installs on its own change Mint's own settings, and they stay when qol stops.
- Wait while in use and Hold all are qol's rules, and they end when qol stops.
- History lists each run with who started it; a run's page lists what it installed.

## License

PolyForm Noncommercial 1.0.0
