# Linking nearby computers

Status: selected. The [enrollment](2026-09-27-peer-enrollment-v1.md),
[enrollment host](2026-09-27-peer-enrollment-host-v1.md) and
[network](2026-09-27-peer-network-v1.md) contracts still apply; this document
records only what changes.

## Why

Linking two computers took a name, a typed `persist`, an invitation copied
between machines by hand, a prepare step, a send step, an approval and three
separate grants. Nobody does that twice. Two computers on the same network can
find each other, so the only thing the user should have to do is say which
computer to link and check that both screens show the same code, the way
Bluetooth pairing works.

## Turning linking on

`Request::Enable {}` turns linking on with defaults. It changes nothing when an
authority is already active. Otherwise it creates the authority the user would
have chosen: a persistent store on a Resident host and a session authority on a
Portable host, named after the computer. The settings page sends it when it
opens, so opening Linked computers is the local request the enrollment contract
requires. Residency itself is never changed.

## Discovery

The advertisement adds two claims: `name`, the authority name cut to 200 bytes
on a character boundary, and `link`, the enrollment listener port. The
enrollment listener therefore binds before discovery starts. Both are claims,
not trust: a name shown before the codes match is only what the other computer
says about itself.

The supervisor keeps at most 16 nearby computers: discovered peers that are not
linked, not revoked and not this computer, with at most eight endpoints each.
Service removal and discovery failure drop them. The network snapshot exposes
them as `nearby`. A rename is advertised the next time networking starts.

## The exchange

The computer where the user clicks Link is the joiner. It connects to the
other computer's enrollment listener with ALPN `qol-nearby/1`, pinning the
discovered peer ID; the listener proves its key and learns the joiner's key
from the same TLS handshake.

1. Joiner sends `offer`: a SHA-256 commitment to a random 32-byte nonce, its
   name and its storage lifetime.
2. Inviter creates an ordinary invitation bound to the joiner's key and name,
   with the connection's local address as its only endpoint, and answers with
   the invitation document and its own random nonce, or a rejection.
3. Joiner reveals its nonce. The inviter checks it against the commitment.
4. Inviter records the code on the invitation and answers `bound`.

The code is the first four bytes of SHA-256 over a domain label, the inviter's
key, the joiner's key and both nonces, as a big-endian integer modulo one
million, shown as `123 456`. The commitment stops a machine in the middle from
choosing its nonce after seeing the other one, so it cannot grind two
connections into showing the same code.

## Confirming

Both computers show the code and a Link button; the user checks that the codes
match and confirms on both, in either order.

- The joiner's confirmation prepares an ordinary outbound join from the
  received invitation and redeems it over `qol-link/1`. A stale pending join
  for the same computer is abandoned first, because clicking Link again is an
  explicit local decision.
- The inviter's confirmation approves at once if the redemption has arrived.
  Otherwise it is remembered on the invitation and the redemption is approved
  the moment it arrives.
- A nearby invitation accepts a redemption only from the key and name it was
  bound to, and only after the code exists.
- Declining cancels the invitation on the inviter, or drops the offer on the
  joiner. The invitation's 120-second expiry bounds the whole exchange.

Each confirmation carries the grants the user left ticked: the operations this
computer exposes to peers, offered per plugin. The inviter stores them with the
new link in the same commit. The joiner applies them to the new link once its
own commit lands. Nothing is granted that the user did not see ticked, and the
grant checks against the installed catalog are unchanged.

## Bounds

Nearby invitations share the eight invitation slots. A new offer from the same
key replaces that key's unconfirmed nearby invitation. The joiner keeps at most
eight offers. Offers, codes, nonces and pending grants live in memory only; the
durable records are the existing outbound joins, receipts and links.

## Evidence

Authority tests cover binding, early and late approval, grants in the commit,
wrong key and name, decline and expiry. An exchange test runs both computers
over real TLS on loopback and checks that both derive the same code and end up
linked with the ticked grants. The real two-computer run on the laptop and the
Linux desktop is the acceptance test.
