# headless CLI mock lab

Thin bash mocks that faithfully mimic each feature's real headless CLI surface
(exact `HeadlessApp` registrations, command names, daemon vs on-demand patterns,
dashed/flat verbs, subcommands, fallback layers). Used to simulate the
five-version design evolution in the companion spec:

`docs/superpowers/specs/2026-08-03-headless-cli-common-interface.md`

## Usage

```bash
# any mock mirrors its real surface
./bins/bluetooth list
./bins/bluetooth --json list
./bins/alt-tab --show
./bins/qol-voice session status --json
./bins/pointz server

# help and doctor
./bins/bluetooth help
./bins/bluetooth help connect
./bins/bluetooth connect help
./bins/bluetooth --json doctor

# lifecycle (V2+)
./bins/cli-sessions daemon    # canonical start
./bins/cli-sessions status
./bins/cli-sessions kill

# config (V4+)
./bins/bluetooth config show
./bins/bluetooth config get managed_devices
```

## Flavours captured

| Flavour | Mocks |
|---|---|
| UI-host dependent | alt-tab (needs cinnamon/muffin) |
| data-heavy with --json | bluetooth |
| daemon-signal, best-effort | cli-sessions |
| status + privileged fix | controllers |
| daemon + status, no kill | ide-checkout |
| run/reload/kill + aliases | keyremap |
| retained-GPUI, dashed verbs | launcher |
| action-heavy flat | lights, window-actions |
| theme state machine | os-themes |
| hierarchical subcommands | qol-voice |
| destructive flags + confirmation | removeapp |
| legacy fallback forwarding | pointz |
| side-effect default | qol-shot |
| installer + legacy fallback | qol-tray-install |
| migration + legacy fallback | qol-tray-migrate |
| scaffold | template |
| tool (13 flat commands) | qol |
