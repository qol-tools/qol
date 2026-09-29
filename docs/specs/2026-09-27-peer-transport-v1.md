# Core peer transport v1

Status: selected for implementation; runtime and security verification remain
outstanding. The [linked-devices design](2026-09-27-linked-devices-design.md)
owns product scope, service ownership, migration, and acceptance. This document
owns the cryptographic transport choice. It does not establish a second trust
registry or authorize deployment.

## Implementation boundary

`qol-peers` owns identity, enrollment, authenticated transport, and the peer
authority. The tray supervises one authority. CLI, settings, and plugins attach
through the existing local runtime API. The authority supplies current trust to
transport verifiers; transport constructors never load an independent registry.

Keep contract types usable without enabling the `service` feature. Cryptography,
networking, and storage implementations belong behind that feature. The crate
must not depend on `qol-runtime` or `qol-plugin-api`: those consumers already have
a dependency relationship, and adding a reverse edge would create a cycle.
Canonical plugin and operation reference types will move to the existing lower
contract owner with facade re-exports before peer grants consume them. Do not
create duplicate operation identity schemas in the interim.

The first transport round implements identity and TLS configuration only. It
does not open production listeners, create a store, expose operations, or claim
the library is a delivered capability. Host integration and PointZ migration
remain required before a usable release.

## Identity

Generate an independent P-256 signing key through the cryptographic library's
operating-system randomness. The peer identifier is the full SHA-256 digest of
the canonical DER SubjectPublicKeyInfo, encoded as unpadded base64url. Store and
compare the exact SPKI pin as well. Names, IP addresses, and certificate serials
are attributes rather than identity.

Issue one self-signed X.509 certificate with P-256/SHA-256, digital-signature key
usage, server and client authentication extended key usages, and the fixed SAN
`qol-peer.invalid`. Certificates live for one year; renew within thirty days of
expiry using the same key. Clock or certificate failures have explicit outcomes.
Replacing a key requires new linking. Imported PointZ master seeds never become
core signing keys.

Certificate input is bounded to 4 KiB. Reject trailing data, chains with extra
certificates, unsupported algorithms or curves, invalid key lengths, CA
certificates, duplicate/malformed extensions, unexpected critical extensions,
invalid usage or validity, and invalid self-signatures. Identity restoration
also proves the private key matches the certificate. Private material has no
ordinary Debug or JSON representation; persistence uses an explicit secret
export/import boundary.

## TLS

Use TLS 1.3 over TCP with rustls and its ring provider, wrapped by tokio-rustls.
Use rcgen for certificate generation and x509-parser with signature verification
enabled for certificate inspection. Current implementation inputs are rustls
0.23.40 and tokio-rustls 0.26.4 already in Cargo.lock, plus rcgen 0.14.10 and
x509-parser 0.18.1. The manifests and regenerated lockfile own resolved versions.
Revisit API and security behavior when those versions change.

Normal connections require mutual current SPKI pinning and ALPN `qol-peers/1`.
Certificate policy and TLS CertificateVerify are separate checks; both must
succeed. Delegate handshake-signature verification to rustls's supported
cryptographic implementation. Use no public-CA, hostname-only, accept-all, or
discovery-based trust fallback.

Disable early data and session resumption in both directions: client resumption
is disabled; server ticket issuance and session storage are disabled. Each new
connection proves possession and consults current trust. Revocation additionally
closes existing sessions and blocks request dispatch; a handshake check alone
does not revoke an already established connection.

Enrollment has its own temporary endpoint and ALPN `qol-link/1`. The joiner pins
the inviter before sending invitation material. The enrollment server requires
a valid client certificate and CertificateVerify but does not treat that key as
trusted until enrollment completes. Separate constructors keep this policy from
being selected accidentally for a normal session. Enrollment messages never
reach the ordinary operation dispatcher.

## Invitations

Use a random 256-bit one-use invitation secret and a random 128-bit invitation
identifier, with at most a 120-second lifetime. A bounded `qol-link:` document
carries the inviter pin, endpoint hints, version, and requested storage lifetime.
The user transfers it by direct scan or an authenticated copy/paste channel.
Tokens never enter discovery, logs, shell arguments/history, or automatic
clipboard synchronization. The old PointZ six-digit exchange is not reused.

Pending invitations stay in memory and expire by a monotonic clock. Reserve
redemption to the authenticated joiner key and transaction identifier. Local
approval confirms the peer and storage lifetime; initial grants are empty.
Commit the link and transaction receipt before acknowledging success. Later
grant changes use the receiving host's explicit local administration API.
Recovery after a lost reply checks the same authenticated
key and committed receipt; it cannot reopen a consumed invitation or revive a
revoked peer.

The [enrollment contract](2026-09-27-peer-enrollment-v1.md) defines bilateral
commit, recovery, message bounds, and the local administration integration.

## Discovery and authority

Use mDNS/DNS-SD `_qol-peer._tcp.local.` as untrusted endpoint discovery, using
mdns-sd when the discovery adapter is implemented. An authenticated session
handshake establishes online state. Advertisements never establish identity,
trust, permission, or device ownership. Invitation endpoint hints provide a
direct-address path when multicast is unavailable.

Persistent identity, pins, grants, tombstones, request admission state, and the
PointZ migration commit point belong in one versioned private atomic snapshot.
Hold a fixed-inode writer lock for the canonical authority root. Publish a new
in-memory state only after its durable commit. A failed or uncertain commit
fails closed. Portable credentials remain in memory; persistent linking requires
local selection and stays outside profile export and synchronization.

Request and observation state machines must retain the design's replay,
freshness, and reconciliation guarantees. Their concrete wire fixtures are
frozen with the service round before general remote dispatch is enabled.

## Required evidence

Exercise real client/server TLS over isolated in-memory or guest transports.
Cover wrong/unknown pins, revoked trust on a later handshake, absent client
certificates, mismatched private keys, invalid signatures and certificate
fields, expiry, ALPN/version mismatch, and disabled resumption. Tests must prove
failure at the connection boundary as well as certificate parser behavior.

No host trust files, production network listeners, real Bluetooth connections,
or PointZ pairing records are test fixtures. The full design acceptance gate
still applies after these transport checks pass.

## Primary references

- [TLS 1.3, RFC 8446](https://www.rfc-editor.org/rfc/rfc8446.html)
- [rustls verification contract](https://docs.rs/rustls/latest/rustls/client/danger/trait.ServerCertVerifier.html)
- [tokio-rustls](https://docs.rs/tokio-rustls/latest/tokio_rustls/)
- [rcgen](https://docs.rs/rcgen/0.14.10/rcgen/)
- [x509-parser](https://docs.rs/x509-parser/0.18.1/x509_parser/)
- [mdns-sd](https://docs.rs/mdns-sd/latest/mdns_sd/)
