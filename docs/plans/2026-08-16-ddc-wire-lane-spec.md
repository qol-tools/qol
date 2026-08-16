# Spec: correct the DDC/CI wire protocol in plugin-monitor

Problem: `plugins/monitor/src/monitor/backends/i2c_ddc.rs` frames DDC/CI packets wrongly, so no real monitor will ever answer.
The unit tests pass because the fake bus implements the same wrong framing.
Verified against ddcutil `src/base/ddc_packets.c` (the reference implementation).

Three concrete defects:

1. Requests put the monitor address `0x6e` where the length byte belongs and have no length byte at all.
2. The checksum is an additive two's-complement sum; DDC/CI uses XOR, with the unwritten destination address `0x6e` folded into request checksums and the virtual host address `0x50` folded into reply checksums.
3. The reply parse expects 10 bytes `[0x6e, 0x51, 0x02, feature, result, cur_hi, cur_lo, max_hi, max_lo, chk]`; the real reply is 11 bytes with a type byte and max BEFORE current.

All work happens in this worktree on branch `ddc-wire`.
Scope is exactly one file: `plugins/monitor/src/monitor/backends/i2c_ddc.rs` (its impl and its `mod tests`), plus the fake-bus mirror inside `plugins/monitor/src/monitor/policy.rs` `mod tests` (`FakeI2cBus::write`).
Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## The correct wire protocol

Userspace writes to `/dev/i2c-N` after `ioctl(I2C_SLAVE, 0x37)`; the kernel emits the destination address `0x6e`, so it is never part of the written payload but IS part of the request checksum.

Get VCP request payload (5 bytes):

```
[0x51, 0x82, OP_GET_VCP, feature, chk]
chk = 0x6e ^ 0x51 ^ 0x82 ^ OP_GET_VCP ^ feature
```

Canonical brightness example (feature 0x10): `[0x51, 0x82, 0x01, 0x10, 0xac]`.

Set VCP request payload (7 bytes):

```
[0x51, 0x84, OP_SET_VCP, feature, value_hi, value_lo, chk]
chk = 0x6e ^ 0x51 ^ 0x84 ^ OP_SET_VCP ^ feature ^ value_hi ^ value_lo
```

Canonical example (feature 0x10, raw value 0x0032): `[0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]`.

Get VCP reply as read from the bus (11 bytes; `REPLY_LEN` becomes 11):

```
index:  0     1     2     3       4        5     6   7   8   9   10
value:  0x6e  0x88  0x02  result  feature  type  mh  ml  sh  sl  chk
```

- `result`: 0x00 success; 0x01 means the display does not support the feature; anything else is a protocol error (keep the existing three-way match, just at the new index).
- `max = (mh << 8) | ml`, `current = (sh << 8) | sl`. Max comes BEFORE current.
- Reply checksum is altmode XOR: `0x50 ^ frame[1] ^ frame[2] ^ ... ^ frame[9] == frame[10]`.
- Validate, in this order: checksum, `frame[0] == 0x6e`, `frame[1] == 0x88`, `frame[2] == OP_GET_VCP_REPLY`, `frame[4] == feature`, then the result code. Error messages keep their current wording where it still applies.

Timing: before the first read attempt after writing a get request, wait 40ms (the DDC/CI mandated response delay). Add a `response_delay: Duration` field beside `settle`/`poll_delay`, default `Duration::from_millis(40)`, injectable through `with_timing` (append it as a fifth parameter and update the call sites in this file, policy.rs tests, and nowhere else - `new()` supplies the default). Tests pass `Duration::ZERO`.

## Implementation notes

- Replace the `checksum` helper with a `xor_checksum(seed: u8, bytes: &[u8]) -> u8` style helper used by both request builders (seed `0x6e`) and the reply validator (seed `0x50`). Delete the additive helper.
- `get_vcp_request` returns 5 bytes as today (content per above). `set_vcp_request` returns 7 bytes.
- `HOST_ADDRESS`/`MONITOR_ADDRESS` constants stay; add `LENGTH_GET: u8 = 0x82`, `LENGTH_SET: u8 = 0x84`, `LENGTH_REPLY: u8 = 0x88`, `REPLY_VIRTUAL_HOST: u8 = 0x50` (or equally clear names).
- `parse_get_vcp_reply` adjusts to the 11-byte layout above and returns `(current, max)` in the SAME tuple order as today so `read_current_max` callers do not change; only the byte extraction moves.
- `read_reply` keeps the partial-fill retry loop, buffer size 11.
- The `FakeBus`/`FakeMonitor` in this file's tests and the `FakeI2cBus` in policy.rs tests must emit and expect the REAL framing (length bytes, XOR checksums, 11-byte reply with type byte and max-before-current). The fake validates request checksums with the XOR rule and rejects a request whose second byte is not `0x82`/`0x84`.
- Add two pinned canonical-bytes tests that assert the exact byte sequences above (`[0x51, 0x82, 0x01, 0x10, 0xac]` and `[0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]`), so the builders can never again drift in lockstep with the fake.
- Add a reply-parse test with a hand-built 11-byte frame for current=500/max=1000 whose checksum is computed with the altmode rule, plus corruption cases: bad checksum, wrong length byte, wrong feature echo, result 0x01.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p plugin-monitor
cargo fmt --check -p plugin-monitor   # run cargo fmt -p plugin-monitor first if needed
cargo clippy -p plugin-monitor --all-targets -- -D warnings
```

Commit on this branch with a conventional message like `fix(monitor): speak real DDC/CI framing on the i2c bus`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution to the commit message.
