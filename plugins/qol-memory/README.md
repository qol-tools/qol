<div align="center">

# QoL Memory

Long-context memory: recall settled facts from your agent session history for [QoL Tray](../../apps/qol-tray).

</div>

## Quick start

Install from the [QoL Tray](../../apps/qol-tray) plugin store.

```text
qol-memory ask "<query>" [--brief] [--json] [--store PATH]
qol-memory status [--json]
qol-memory doctor [--json]
```

## About

Answers `qol-memory ask "<query>"` from the local memory store with a verdict, an outcome (supported, qualified, ambiguous, conflicting, unsupported) with its reason code, evidence, and coverage signals that match the research scripts result for result. The Node tooling in `docs/research/qol-memory` stays the write path while the plugin owns the read path, `status`, and `doctor`.

## License

PolyForm Noncommercial 1.0.0
