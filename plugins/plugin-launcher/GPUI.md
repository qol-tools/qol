# GPUI Knowledge Base

Bespoke documentation for gpui, built through hands-on exploration.

## Resources

- [gpui.rs](https://www.gpui.rs/) - Official site
- [docs.rs/gpui](https://docs.rs/gpui) - API docs
- [Zed gpui crate](https://github.com/zed-industries/zed/tree/main/crates/gpui) - Source of truth
- [gpui-component](https://github.com/longbridge/gpui-component) - 60+ ready-made components (recommended)
- [WindowOptions docs](https://docs.rs/gpui/latest/gpui/struct.WindowOptions.html)

## Project Setup

```toml
[dependencies]
gpui = "0.2"
```

Requires: Rust stable, macOS or Linux.

### Linux Dependencies (Ubuntu/Debian)

```bash
sudo apt install gcc g++ libasound2-dev libfontconfig-dev libwayland-dev \
    libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev libvulkan1 \
    libgit2-dev make cmake clang mold libstdc++-14-dev
```

Adjust `libstdc++-14-dev` based on Ubuntu version:
- Ubuntu 24.04+: `libstdc++-14-dev`
- Ubuntu 22.04: `libstdc++-12-dev`
- Ubuntu 20.04: `libstdc++-10-dev`

### macOS Dependencies

Xcode with Metal support.

## Minimal Window (verified working on Linux)

```rust
use gpui::*;

actions!(launcher, [Quit]);

struct MyView;

impl Render for MyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .child("Hello gpui")
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(600.), px(42.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(200.), px(20.))),  // No webkit2gtk min size!
            titlebar: None,
            focus: true,
            ..Default::default()
        };

        cx.open_window(options, |_, cx| cx.new(|_| MyView)).unwrap();
        cx.activate(true);
    });
}
```

## Core Concepts

### App (cx)
Top-level application context in `run()` callback. Used to:
- Create views: `cx.new(|_| MyView)`
- Open windows: `cx.open_window(options, |window, cx| { ... })`
- Bind keys: `cx.bind_keys([...])`
- Register actions: `cx.on_action(|action, cx| { ... })`

### Context<T>
View-specific context passed to `render()`. Used for state updates and notifications.

### Window
Passed to `render()`. Used for window operations like `window.resize()`.

### Render trait
Views implement this to draw UI:
```rust
impl Render for MyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().child("content")
    }
}
```

### FocusHandle
Tracks keyboard focus. Essential for text input.

## Styling (Tailwind-like)

Chain methods on elements:

```rust
div()
    .flex()              // display: flex
    .flex_col()          // flex-direction: column
    .gap_2()             // gap: 0.5rem (2 * 0.25rem)
    .p_4()               // padding: 1rem
    .px_2()              // padding-x: 0.5rem
    .bg(rgb(0x1e1e2e))   // background color
    .text_color(white()) // text color
    .rounded_md()        // border-radius
    .shadow_lg()         // box-shadow
    .w(px(600.))         // width: 600px
    .h(px(42.))          // height: 42px
    .size_full()         // width: 100%, height: 100%
```

## Window Sizing

### Initial size (no minimum constraint!)
```rust
let bounds = Bounds::centered(None, size(px(600.), px(42.)), cx);
let options = WindowOptions {
    window_bounds: Some(WindowBounds::Windowed(bounds)),
    window_min_size: Some(size(px(100.), px(20.))),  // Can go tiny!
    ..Default::default()
};
```

### Runtime resize
```rust
// Inside a window context
window.resize(size(px(600.), px(new_height)));
```

Methods available:
- `window.resize(Size<Pixels>)` - Set content size
- `window.bounds()` - Get current position/size
- `window.viewport_size()` - Get drawable area

## Keyboard Input

### Define actions
```rust
actions!(launcher, [Submit, Cancel, SelectNext, SelectPrev]);
```

### Bind keys globally
```rust
cx.bind_keys([
    KeyBinding::new("enter", Submit, None),
    KeyBinding::new("escape", Cancel, None),
    KeyBinding::new("down", SelectNext, None),
    KeyBinding::new("up", SelectPrev, None),
    KeyBinding::new("ctrl-n", SelectNext, None),
    KeyBinding::new("ctrl-p", SelectPrev, None),
]);
```

### Handle actions
```rust
impl MyView {
    fn submit(&mut self, _: &Submit, cx: &mut ViewContext<Self>) {
        // Handle enter
    }
}
```

### Observe keystrokes
```rust
cx.observe_keystrokes(move |event, _, cx| {
    let keystroke = event.keystroke.clone();
    // Process keystroke
});
```

## Text Input

### The hard way (raw gpui)
700+ lines of code. Implements `EntityInputHandler`. Not recommended.

### The easy way (gpui-component)
Use the [gpui-component](https://github.com/longbridge/gpui-component) library:

```rust
use ui::{Input, InputState};

// Create state
let input_state = cx.new(|cx| InputState::new(cx));

// Render
Input::new(&input_state)
    .appearance(false)  // No border
    .placeholder("Search...")
    .cleanable()        // Show clear button
```

The Input component handles:
- Text editing (backspace, delete, cut, paste, undo, redo)
- Navigation (arrows, home, end)
- Selection (select all, shift+arrows)
- Focus management

## Focus Management

```rust
struct MyView {
    focus_handle: FocusHandle,
}

impl MyView {
    fn new(cx: &mut ViewContext<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

// Focus the element
self.focus_handle.focus(cx);

// Check if focused
self.focus_handle.is_focused(cx);
```

## Launcher-Specific Patterns

### Dynamic height based on results
```rust
fn update_height(&self, result_count: usize, window: &mut Window, cx: &mut Context) {
    let input_height = 42.;
    let result_height = 36.;
    let max_results = 8;
    let visible = result_count.min(max_results);
    let total = input_height + (visible as f64 * result_height);
    window.resize(size(px(600.), px(total as f32)));
}
```

### Borderless popup window
```rust
WindowOptions {
    titlebar: None,
    window_decorations: Some(WindowDecorations::Client),
    is_resizable: false,
    is_movable: true,
    focus: true,
    ..Default::default()
}
```

## Gotchas

### Documentation is sparse
Best source is Zed's source code. Search in `zed/crates/` for patterns.

### Text input is complex
Don't build from scratch. Use gpui-component's Input.

### Actions need context
Actions are dispatched based on focus. Make sure elements have focus handles.

### Breaking changes
Pre-1.0 library. Pin your version in Cargo.toml.

## TODO

- [x] Test minimal window on Linux (42px works!)
- [ ] Test dynamic window resize
- [ ] Test keyboard input (text field)
- [ ] Test list rendering
- [ ] Test result selection with keyboard
- [ ] Test gpui-component Input integration
