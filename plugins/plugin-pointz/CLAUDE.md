# Coding Guidelines

## IMPORTANT: Do NOT Build or Test

Never run build or test commands (`cargo build`, `cargo test`, `flutter build`, `make`, etc.) unless explicitly asked. The user will run these manually.

## Code Style

- Do not add comments to code. The code should be self-explanatory.
- Only use emojis if the user explicitly requests it.

## Git Commits

- **CRITICAL: Do NOT commit unless explicitly asked by the user or after confirming a fix works**
- During debugging/iteration, make changes and let the user test them first
- Only commit when the user confirms the changes are beneficial
- If making multiple experimental changes, test them all before committing anything
- NEVER add Co-Author lines or any attribution/generation text
- Always commit systematically in logical order
- Each commit must represent a working state with files that are logically tied together
- Use conventional commit style (feat:, fix:, refactor:, etc.)
- When you need to squash commits, use `git reset --hard <commit>` and `git push --force`

## Cross-Platform Support

Platform-specific code should be isolated in dedicated modules:
- Use `platform/` subdirectories for OS-specific implementations
- Keep main modules free of `#[cfg(target_os)]` conditionals when possible
- All platform differences should be handled at the platform abstraction layer
- Test on all target platforms (Linux, macOS, Windows)

### Platform-Specific Patterns

**Linux:**
- GTK event loops typically run in separate threads
- Use X11 bindings for low-level system interactions

**macOS:**
- UI frameworks (NSApplication, tray icons) MUST be created on the main thread
- `NSApplication.run()` blocks the main thread until quit
- Run async runtimes (Tokio) on background threads
- Use `objc2` crate for Cocoa bindings
- Use CoreGraphics APIs directly for performance-critical operations

**Windows:**
- Use Win32 APIs for system interactions
- Blocking patterns often use Condvar or WaitForSingleObject

## Lessons Learned

### Test-Driven Bug Discovery
Adding comprehensive edge case tests often reveals bugs in the implementation:
- Adding `("V1.2.3", vec![1, 2, 3])` test case revealed version parser only handled lowercase 'v'
- Adding `("--help", false)` test case revealed action ID validation didn't check leading dashes
- Adding `("<body data-x='a>b'>", Some(19))` test case revealed HTML parser didn't handle `>` inside quotes

**Pattern:** When adding tests, think about what the implementation *actually does* vs what it *should do*. Write the test for expected behavior first, then fix the implementation if it fails.

### Consolidate Validation Functions
Path/ID validation functions tend to get duplicated. Keep them in one place:
- Create shared validation utilities for common patterns (path components, IDs, etc.)
- Validate for security: no `/`, `\`, `..`, `.`, null bytes in user-provided paths
- Reuse validation across all entry points

### Graceful Process Shutdown
When stopping child processes:
1. Send SIGTERM first (Unix) to allow graceful cleanup
2. Wait with timeout (2s is reasonable)
3. Only SIGKILL if process doesn't respond
4. Use `libc::kill()` directly - no Rust wrapper needed

### Error Handling Patterns
- `.expect()` is acceptable for compile-time invariants (embedded assets)
- `.expect()` is NOT acceptable for runtime operations (file paths, config dirs)
- Return `Option` or `Result` and let callers decide how to handle
- Log errors at the point of failure, not just at the top level

### Parsing Edge Cases
Simple string matching for structured data (HTML, TOML, etc.) needs to handle:
- Case insensitivity where applicable
- Quotes and escaped characters
- Comments and whitespace
- Partial/incomplete data during development

A proper parser library is better than regex, but if rolling your own, handle the common edge cases correctly.

### macOS Event Loop Requirements
On macOS, many system frameworks require specific threading:
1. UI components must be created on the main thread
2. Event loops (NSApplication.run()) block the main thread until quit
3. Async runtimes must run on background threads

The pattern is: main thread runs system event loop, background thread runs async runtime.

### Broken Symlink Detection
On Unix-like systems, `std::path::Path::exists()` returns `false` for broken symlinks. To detect if a symlink exists regardless of its target, use `std::fs::symlink_metadata(path).is_ok()`. This is critical when managing links where targets might be moved or deleted.

### Robust Configuration Parsing
When scanning for configuration files, use fallback parsing patterns:
- Implement minimal versions of data structures for partial configs
- Allow optional sections to be missing during development
- Provide sensible defaults for missing fields

### UI Component Consistency
Reuse UI components across views for consistent look and feel:
- Define component classes once and reuse (buttons, badges, spinners)
- Use consistent naming patterns (`.btn-primary`, `.badge-success`, etc.)
- When adding new states, extend existing patterns rather than creating new ones

### Stable UI Layouts
To prevent layout jumping when state changes:
- Use fixed `min-height` on rows that may have variable content
- Always render placeholder elements (empty spans) to reserve space
- Use overlay positioning for transient indicators instead of inserting elements
- Clamp selection indices after list updates to prevent out-of-bounds states

### Smooth Animations During Async Operations
When showing animations during async operations, guard all render calls:
- Set a state flag before the operation
- Guard all async callbacks with `if (state.pending) return;`
- Only clear the flag and re-render once in the `finally` block
- This prevents intermediate re-renders that would restart CSS animations

### Security Best Practices
- Validate all user input at boundaries (path components, IDs, file names)
- Reject path traversal attempts (`..`, absolute paths in user input)
- Check file sizes before reading to prevent memory exhaustion
- Use timeouts for network operations to prevent hangs
- Remove internal error details from user-facing messages
- Sanitize shell inputs to prevent injection attacks

### Performance Patterns
- Use appropriate data structures (HashMap for lookups, Vec for iteration)
- Avoid cloning large data structures unnecessarily
- Profile before optimizing - measure actual bottlenecks
- Consider platform-specific optimizations (direct APIs vs libraries)
- Batch operations when possible (e.g., 16ms intervals for 60fps)

### Cross-Platform File Paths
- Use `std::path::PathBuf` and `Path` for all file operations
- Use `std::env::temp_dir()` instead of hardcoded `/tmp`
- Use platform-appropriate path separators automatically
- Test file operations on all platforms (case sensitivity, path limits, etc.)
