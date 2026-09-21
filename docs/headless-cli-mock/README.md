# headless CLI mock lab

Thin bash mocks that faithfully mimic each feature's real headless CLI surface
(exact `HeadlessApp` registrations, command names, daemon vs on-demand patterns,
dashed/flat verbs, subcommands, fallback layers). Used to simulate the
five-version design evolution in the companion spec:

`docs/specs/2026-08-03-headless-cli-common-interface.md`

## Usage

```bash
# any mock mirrors its real surface
./bins/qol-bluetooth list
./bins/qol-bluetooth --json list
./bins/qol-alt-tab --show
./bins/qol-voice session status --json
./bins/qol-pointz server

# help and doctor
./bins/qol-bluetooth help
./bins/qol-bluetooth help connect
./bins/qol-bluetooth connect help
./bins/qol-bluetooth --json doctor

# lifecycle (V2+)
./bins/qol-cli-sessions daemon    # canonical start
./bins/qol-cli-sessions status
./bins/qol-cli-sessions kill

# config (V4+)
./bins/qol-bluetooth config show
./bins/qol-bluetooth config get managed_devices
```

## Flavours captured

| Flavour | Mocks |
|---|---|
| UI-host dependent | qol-alt-tab (needs cinnamon/muffin) |
| data-heavy with --json | qol-bluetooth |
| daemon-signal, best-effort | qol-cli-sessions |
| status + privileged fix | qol-controllers |
| daemon + status, no kill | qol-ide-checkout |
| run/reload/kill + aliases | qol-keyremap |
| retained-GPUI, dashed verbs | qol-launcher |
| action-heavy flat | qol-lights, qol-window-actions |
| theme state machine | qol-os-themes |
| hierarchical subcommands | qol-voice |
| destructive flags + confirmation | qol-removeapp |
| legacy fallback forwarding | qol-pointz |
| side-effect default | qol-shot |
| installer + legacy fallback | qol-tray-install |
| migration + legacy fallback | qol-tray-migrate |
| scaffold | qol-template |
| tool (13 flat commands) | qol |
