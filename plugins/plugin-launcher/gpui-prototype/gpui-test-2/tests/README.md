# GPUI Integration Testing

This document outlines the testing strategy for the GPUI-based launcher, covering functional logic, UI dynamics, and edge cases.

## Test Suite Structure

Tests are located in `tests/integration_tests.rs`.

### 1. Functional Logic (`test_filter_logic_basic`)
*   **Goal:** Verify that input changes correctly filter the list.
*   **Method:**
    1.  Initialize app with known items ("Apple", "Banana").
    2.  Simulate user typing "Ap".
    3.  Assert that `ListDelegate` matches count drops to 1 and contains "Apple".

### 2. Window Dynamics (`test_window_resizing`)
*   **Goal:** Verify window height adapts to content size.
*   **Method:**
    1.  Start with 3 items. Verify height is `112px` (Header + 3 * Item).
    2.  Filter to 0 items. Verify height shrinks to `40px` (Header only).
    3.  **Crucial:** This ensures the "pop-up" feels responsive and doesn't leave empty space.

### 3. Fuzzing / Property Testing (`prop_test_delegate_filter`)
*   **Goal:** Ensure stability against arbitrary input.
*   **Method:**
    *   Uses `proptest` to generate 100 random combinations of search queries and item lists.
    *   **Invariants Checked:**
        *   Filtered count <= Total count.
        *   Every matched item *must* contain the query string.
        *   No panics on empty strings or special characters.

## Key Findings & Fixes

*   **Render Safety:** Discovered a potential panic where the list view might request an index that was just filtered out.
    *   *Fix:* Added bounds check in `ListDelegate::render_item`: `if ix.row >= self.matches.len() { return None; }`.
*   **Async State:** Tests require careful use of `cx.update(...)` to ensure state is synchronized between the test context and the view.

## Running Tests

```bash
cargo test --test integration_tests
```
