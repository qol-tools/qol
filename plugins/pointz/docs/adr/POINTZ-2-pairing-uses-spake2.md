# POINTZ-2 Pairing Uses SPAKE2 And Command Payloads Are Sealed

- **Status:** Accepted
- **Issue:** none (security review finding T5, F1 and F3)
- **Date:** 2026-11-07
- **Related:** supersedes [POINTZ-1](POINTZ-1-pairing-secret-is-broadcast-in-plaintext-during-discovery.md); pointz-client Flutter repo (lockstep companion change)

## Problem

POINTZ-1 moved PointZ to a per-device key handed over after an X25519 exchange, with the typed PIN folded into the HKDF info that derives `K_auth` and `K_wrap`.
That construction hides the key from a passive observer, but it does not stop an observer from testing PIN guesses offline.

`client_confirm` is `HMAC(K_auth, "pointz-confirm-client")`, and `K_auth` depends on the Diffie-Hellman secret, the public keys, the salt, the device id and the PIN.
An attacker who runs the exchange as a man in the middle, or who is simply the party that sent `PairHello`, chooses their own ephemeral secret.
They therefore know the Diffie-Hellman secret on their side of the exchange and can recompute `K_auth` for every PIN.
With one captured `PairConfirm` from a real phone that was talking to the attacker's own key, or with the attacker's own exchange against the desktop, the six-digit PIN (a million values) is checked offline against the confirm tag in a fraction of a second.
The three-attempt limit only bounds online guesses, so the PIN that is meant to authenticate the exchange is recoverable without ever touching the attempt counter.

The command channel has a second weakness: envelope v2 authenticates the JSON payload with a MAC but sends it in cleartext, so a LAN observer reads every keystroke and pointer move the phone sends.

| ID | State | Smell |
|----|-------|-------|
| POINTZ-2.1 | Broken | The PIN is brute-forceable offline against `client_confirm`, because the PIN enters a hash input that an attacker can evaluate with a secret they hold. |
| POINTZ-2.2 | Broken | Command payloads are authenticated but not encrypted, so typed text crosses the LAN in cleartext. |
| POINTZ-2.3 | Hazard | Any fix must keep the typed six-digit PIN and the three-attempt, 60-second window, and must not strand phones paired before the change. |

## Decision

Pairing uses SPAKE2 (RFC 9382) over edwards25519 with SHA-256, HKDF-SHA256 and HMAC-SHA256, and command payloads are sealed with ChaCha20-Poly1305 in envelope v3.

A password-authenticated key exchange is the primitive built for a low-entropy secret: an observer or an active attacker learns nothing that lets them test a PIN guess without a fresh online run against a live desktop, and each run costs one of the three attempts.
The desktop answers its own confirmation (`server_confirm`) only after the phone's confirmation verified, so a wrong PIN never yields a value that can be checked offline.

The exact byte-level protocol (point encodings, the password scalar `w`, the transcript, key schedule, messages and a shared test vector) is the PointZ pairing v2 and envelope v3 specification that both repositories implement.
The deliberate deviation from the RFC is that `w` is encoded little-endian in the transcript, so both sides use the same scalar encoding as the curve library.

```mermaid
sequenceDiagram
    participant C as Phone (party A)
    participant D as Desktop (party B, shows PIN)
    C->>D: PairHello { protocol 2, device_id, name, pa = w*M + x*G }
    D-->>C: PairOffer { protocol 2, server_id, pb = w*N + y*G, salt }
    Note over C,D: K = h*x*(pb - w*N) on the phone, h*y*(pa - w*M) on the desktop
    Note over C,D: TT, Ke, Ka, KcA, KcB, cA, cB, K_wrap from the spec
    C->>D: PairConfirm { protocol 2, device_id, client_confirm = cA }
    Note over D: constant-time compare with the expected cA; a mismatch costs an attempt
    D-->>C: PairResult { protocol 2, server_confirm = cB, wrap_nonce, sealed = AEAD(K_wrap, K_dev) }
    Note over C: verify cB, then open sealed and store K_dev
```

Received points must decode and must not be of small order, which also rejects the identity.
Any `Pair*` message whose `protocol` is not 2 is answered with `PairError` reason `protocol` and `expected: 2`, and costs no attempt.

### Envelope v3

`{"version":3,"device_id","sent_at_ms","nonce","payload"}` with a 12-byte random nonce and `payload = ChaCha20-Poly1305(K_dev, nonce, aad = [0x03] || sent_at_ms (8 bytes big-endian) || device_id, command JSON)`.
The device id and the timestamp are bound by the AAD, so a captured packet cannot be retargeted or retimed.
The desktop keeps the 30-second clock-skew window and the replay set keyed by `(device_id, nonce)`; the replay set accepts both the 16-byte v2 nonce and the 12-byte v3 nonce.

Discovery answers `authentication: "pair-spake2-v2"` and `command: "sealed-v3"`, so a phone or desktop on the other side of the change shows an "update the app" or "update the desktop" message instead of attempting to pair.

### Registry

Each stored device carries `sealed_only`.
It is true for every device paired through SPAKE2 and false for devices imported from the legacy registry and for every record stored before this change (the field defaults to false when absent).
The desktop refuses a v2 envelope from a `sealed_only` device, so a device that can speak v3 cannot be downgraded to the MAC-only format.

## Rollout

Desktop first, phone app in lockstep.
The desktop release speaks only pairing protocol 2.
The phone release speaks pairing protocol 2 and sends v3 envelopes to every desktop that advertises `command: "sealed-v3"`.

Phones paired before this change keep working without re-pairing.
The desktop accepts legacy v2 envelopes only for devices whose record is not `sealed_only`, which are exactly the devices imported or paired before the change.
A phone that updates starts sending v3 with its existing key, which the desktop accepts for legacy devices as well.
Pairing a phone again through SPAKE2 replaces its record with a `sealed_only` one, which closes the v2 path for that device.

A phone on the old app sees `authentication: "pair-spake2-v2"` and cannot start a new pairing until it updates, which is the intended failure.

## Consequences

The offline PIN search against `client_confirm` is gone, and a PIN guess costs an online attempt against a live desktop within a 60-second window.
Typed text and pointer movement are encrypted on the LAN for every device that has paired through the new protocol or updated its app.
Legacy devices keep the v2 MAC-only path until they pair again, and that path is the only remaining v2 surface.
The pairing code adds `curve25519-dalek` as a direct dependency of `qol-peers`, and drops `x25519-dalek`.
Nothing in the tray UI changes: the PIN is generated and shown as before.
