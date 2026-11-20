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
- Skip and Never write Mint's own ignore list, so Mint's Update Manager and its nightly run skip the update too. Skipped lists them, and Update again takes one off. A user who is not an administrator cannot take off an entry that `/etc/mintupdate.blacklist` holds, and is told so.
- The waiting page tags every update with its updater when they differ: Update Manager, Flatpak or firmware. A list longer than 40 shows 40 at a time.
- Restart tonight waits for Mint's own nightly run; Restart now asks first.
- Hours and Installs on its own change Mint's own settings, and they stay when qol stops.
- Old kernels switches the system's own removal of old kernels, and it stays when qol stops. apt always keeps the running kernel and the two newest.
  - On Linux Mint it is Mint's cleanup, which runs weekly and 3 hours after each startup. It runs `apt-get autoremove --purge`, so it also removes other packages nothing needs.
  - Include kernels installed by hand adds `qol-kernels-by-hand.conf` to that cleanup, so kernels installed from Mint's Update Manager window are removed too. It is off until you switch it on, so a kernel kept on purpose stays.
  - On Ubuntu and Debian it is the unattended-upgrades setting that removes unused kernels, written to `/etc/apt/apt.conf.d/90qol-update-manager-kernels`. It runs only while automatic updates are on.
  - Anywhere else the row says nothing removes old kernels on its own.
- `status` ends with an `Old kernels:` line. It says unknown when Mint's removal timer and its flag file disagree.
- `cleanup` lists what updates left behind on Linux: old kernels, kernels installed by hand, packages nothing needs, downloaded packages, unused Flatpak runtimes and old snap versions. Each line gives the kind's id and its size, which is exact, dpkg's estimate, or unknown with the reason. The total counts a package two kinds list once. The daemon measures every 6 hours and after each change, never while it reads updates or installs, and measures again 10 minutes after a failed measure.
- `cleanup remove <kind>` removes one kind, and `cleanup remove all` removes every kind except old snap versions and kernels installed by hand. Those two need `--confirm`, since one is the last way back and the other was kept on purpose.
- `doctor` has one line per kind.
- Wait while in use and Hold all are qol's rules, and they end when qol stops.
- History lists each run with who started it; a run's page lists what it installed.

## License

PolyForm Noncommercial 1.0.0
