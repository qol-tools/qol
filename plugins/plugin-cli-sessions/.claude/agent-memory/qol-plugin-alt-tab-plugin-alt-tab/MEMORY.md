
## 2026-06-23
- In rustfmt, short tuple rows in a `&[Case]` array auto-collapse to one line; expect `cargo fmt` to rewrite the table and re-run `--check` after applying.
- `&'static [u32]` slice fields in test case tuples force inner slices to be `'static`; use `const` (not `let`) for shared values so slice literals get static promotion.
- clippy `needless_range_loop` (under `-D warnings`) fires on `for i in 0..n { ... used[i] ... }` even when the body looks index-driven; rewrite as `used.iter().enumerate()`.
