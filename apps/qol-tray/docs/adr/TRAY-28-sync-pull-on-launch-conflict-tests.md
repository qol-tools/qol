# TRAY-28 Sync Pull on Launch Conflict Tests

- **Status:** Proposed
- **Closes:** #28
- **Date:** 2026-05-09
- **Related:** {OTHER-PID}, {OTHER-PID-2}

## Problem

{1-2 sentence framing of the current state and why it hurts.}

```mermaid
stateDiagram-v2
    direction LR
    [*] --> Healthy
    Healthy --> Broken: example failure
    classDef bad fill:#f5c2c7,stroke:#842029,color:#000
    classDef warn fill:#ffeeba,stroke:#856404,color:#000
    class Broken bad
```

| ID | State | Smell |
|----|-------|-------|
| TRAY-28.1 | 🔴 Broken | one-sentence smell |
| TRAY-28.2 | 🟡 Leaky | one-sentence smell |

> Severity: 🔴 bad (broken / silent failure / data loss) · 🟡 warn (leaky / race / brittle) · 🟢 good (used in proposal diagrams to mark what is now safe)

## Proposals

### Proposal A — {name} `[cheap|medium|heavy]`

{Short description of the change.}

```mermaid
graph LR
    A --> B
    classDef good fill:#cfe8d6,stroke:#0f5132,color:#000
    class B good
```

| Pros | Cons |
|------|------|
| one sentence | one sentence |
| one sentence | one sentence |

**Closes:** TRAY-28.1, TRAY-28.2

---

### Proposal B — {name} `[cheap|medium|heavy]`

{Short description.}

```mermaid
graph LR
    A --> C
```

| Pros | Cons |
|------|------|
| one sentence | one sentence |

**Closes:** TRAY-28.2

---

**Recommended:** A (or A+B together).

## Notes

Free-form notes, links to related ADRs, references to industry patterns, or context the diagram cannot carry. Optional.
