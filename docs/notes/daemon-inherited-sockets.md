# Inherited daemon socket validation

Inherited descriptors must be validated before adoption. Stream descriptors
must already be listening; datagram port descriptors remain valid without
listening. Rejected descriptors stay open, and validation must not consume
pending connections or change socket state. Adopted Unix listeners restore
close-on-exec and leave socket-path ownership with the host.

Linux queries `SO_ACCEPTCONN`. macOS does not implement that socket-option
query, so its adapter reads the current process's `socket_fdinfo` through
`proc_pidfdinfo` and tests `soi_options`. A small C bridge compiles against the
SDK's declarations, keeping Apple's structure layout out of handwritten Rust
bindings. It requires a complete response and rejects failed queries.

The Unix test suite exercises inherited Unix/TCP/UDP descriptors, close-on-exec,
path ownership, invalid descriptors, connected and unbound streams, and a
pending connection that must survive validation. Both supported operating
systems run these tests in CI.

References: Apple's [socket-option implementation](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/uipc_socket.c),
[descriptor query](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c),
and [socket-info declarations](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h).
