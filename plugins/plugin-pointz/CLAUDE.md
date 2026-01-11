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
- Keep main modules free of platform conditionals when possible
- All platform differences should be handled at the platform abstraction layer
- Test on all target platforms

## Lessons Learned

### Test-Driven Bug Discovery
Adding comprehensive edge case tests often reveals bugs in the implementation.

**Pattern:** When adding tests, think about what the implementation *actually does* vs what it *should do*. Write the test for expected behavior first, then fix the implementation if it fails.

### Consolidate Validation Functions
Validation functions tend to get duplicated. Keep them in one place:
- Create shared validation utilities for common patterns
- Validate for security: no path traversal, null bytes in user-provided paths
- Reuse validation across all entry points

### Error Handling Patterns
- `.expect()` is acceptable for compile-time invariants (embedded assets)
- `.expect()` is NOT acceptable for runtime operations (file paths, config dirs)
- Return `Option` or `Result` and let callers decide how to handle
- Log errors at the point of failure, not just at the top level

### Parsing Edge Cases
Simple string matching for structured data needs to handle:
- Case insensitivity where applicable
- Quotes and escaped characters
- Comments and whitespace
- Partial/incomplete data during development

A proper parser library is better than regex, but if rolling your own, handle the common edge cases correctly.

### UI Component Consistency
Reuse UI components across views for consistent look and feel:
- Define component classes once and reuse
- Use consistent naming patterns
- When adding new states, extend existing patterns rather than creating new ones

### Stable UI Layouts
To prevent layout jumping when state changes:
- Use fixed dimensions on elements that may have variable content
- Always render placeholder elements to reserve space
- Use overlay positioning for transient indicators instead of inserting elements
- Clamp selection indices after list updates to prevent out-of-bounds states

### Smooth Animations During Async Operations
When showing animations during async operations, guard all render calls:
- Set a state flag before the operation
- Guard all async callbacks with `if (state.pending) return;`
- Only clear the flag and re-render once in the `finally` block
- This prevents intermediate re-renders that would restart animations

### Security Best Practices
- Validate all user input at boundaries (path components, IDs, file names)
- Reject path traversal attempts (`..`, absolute paths in user input)
- Check file sizes before reading to prevent memory exhaustion
- Use timeouts for network operations to prevent hangs
- Remove internal error details from user-facing messages
- Sanitize shell inputs to prevent injection attacks

### Performance Patterns
- Use appropriate data structures for the task at hand
- Avoid cloning large data structures unnecessarily
- Profile before optimizing - measure actual bottlenecks
- Consider platform-specific optimizations when justified
- Batch operations when possible

### Cross-Platform File Paths
- Use appropriate path abstractions for your language/framework
- Avoid hardcoded paths - use environment-specific path helpers
- Use platform-appropriate path separators automatically
- Test file operations on all platforms (case sensitivity, path limits, etc.)
