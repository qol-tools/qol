//! Pin the encoded-cwd format Claude Code uses for its project directories.
//!
//! Claude stores per-project transcripts under `~/.claude/projects/<encoded>/`.
//! The encoding is deterministic - on every observed Claude install the cwd
//! path has every `/` replaced with `-`, so `/home/user` becomes
//! `-home-user`. This test locks that invariant; if Claude ever changes
//! the encoding, the test fails and the resolver is updated in lockstep.
//!
//! The template registry on plugin-kitty also enforces a regex
//! `^-[A-Za-z0-9._-]+$` on the result; every encoded form here must
//! satisfy that regex.
//!
//! Closes: CSESS-1.2 (encoded-cwd resolution invariant).

use std::path::Path;

use plugin_claude_sessions::encode_cwd;

#[test]
fn encode_cwd_replaces_slashes_with_dashes() {
    let cwd = Path::new("/home/user/repos/private/qol-tools/workspace");
    assert_eq!(
        encode_cwd(cwd),
        "-home-user-repos-private-qol-tools-workspace"
    );
}

#[test]
fn encode_cwd_preserves_dots_and_underscores() {
    let cwd = Path::new("/home/user/repos/foo.bar/baz_qux");
    assert_eq!(encode_cwd(cwd), "-home-user-repos-foo.bar-baz_qux");
}

#[test]
fn encode_cwd_root_directory() {
    // `/` alone encodes to `-` (a single leading dash, no path body).
    // plugin-kitty's regex `^-[A-Za-z0-9._-]+$` requires at least one
    // body character, so this case is structurally valid but exists
    // only as the degenerate edge.
    assert_eq!(encode_cwd(Path::new("/")), "-");
}

#[test]
fn encode_cwd_output_satisfies_template_regex_charset() {
    // Pin the implicit promise made to plugin-kitty's template regex
    // (`^-[A-Za-z0-9._-]+$`): every byte we emit for a sensible cwd
    // must be ASCII alphanumeric, dot, underscore, or dash. Anything
    // else means the cwd held a character the encoding does not handle
    // and we would fail validation at the resolving end.
    let cwd = Path::new("/home/user/.claude/projects-test");
    let encoded = encode_cwd(cwd);
    for (i, b) in encoded.bytes().enumerate() {
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
        assert!(
            ok,
            "encoded-cwd byte {i} (`{}`) violates template regex charset; \
             got byte 0x{b:02x} in `{encoded}`",
            b as char,
        );
    }
    assert!(
        encoded.starts_with('-'),
        "encoded-cwd must start with `-` (regex anchor): got `{encoded}`"
    );
}

#[test]
fn encode_cwd_is_deterministic() {
    // Two calls with the same input must produce identical output;
    // there is no hidden state (e.g. random salt, time-based suffix).
    let cwd = Path::new("/home/user/repos/private/qol-tools/workspace");
    assert_eq!(encode_cwd(cwd), encode_cwd(cwd));
}
