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
gpui-component = "0.5.0"
```

Requires: Rust stable, macOS or Linux.

### Linux Dependencies (Ubuntu/Debian)

```bash
sudo apt install gcc g++ libasound2-dev libfontconfig-dev libwayland-dev \
    libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev libvulkan1 \
    libgit2-dev make cmake clang mold libstdc++-14-dev
```

## Minimal Window

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

## Text Input

Use `gpui-component` for a robust input field.

```rust
use gpui_component::input::{Input, InputState};

struct MyView {
    input: Entity<InputState>,
}

impl MyView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Create state (needs window access)
        let input = cx.new(|cx| 
            InputState::new(window, cx)
                .placeholder("Search...")
        );
        
        // Listen to changes
        cx.subscribe_in(&input, window, |_, _, event, _, _| {
            if let gpui_component::input::InputEvent::Change = event {
                println!("Input changed");
            }
        }).detach();

        Self { input }
    }
}

impl Render for MyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.input)
    }
}
```

## List Rendering

Use `gpui-component`'s `List` for virtualized lists with keyboard navigation.

```rust
use gpui_component::list::{List, ListDelegate, ListState, ListItem};

struct MyDelegate {
    items: Vec<String>,
    selected_index: Option<IndexPath>,
}

impl ListDelegate for MyDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.items.len()
    }

    fn render_item(&mut self, ix: IndexPath, _window: &mut Window, _cx: &mut Context<ListState<Self>>) -> Option<Self::Item> {
        Some(ListItem::new(("item", ix.row))
            .child(self.items[ix.row].clone()))
    }

    fn set_selected_index(&mut self, ix: Option<IndexPath>, _window: &mut Window, _cx: &mut Context<ListState<Self>>) {
        self.selected_index = ix;
    }
}

// In your view:
let list_state = cx.new(|cx| ListState::new(delegate, window, cx));

// Render:
List::new(&list_state).h_full()
```

## Keyboard Navigation

`gpui-component::List` handles Up/Down/Enter automatically if focused.
To confirm selection:

```rust
// In MyDelegate
fn confirm(&mut self, _secondary: bool, _window: &mut Window, _cx: &mut Context<ListState<Self>>) {
    if let Some(ix) = self.selected_index {
        println!("Confirmed item at index {:?}", ix);
    }
}
```

## Focus Management

GPUI uses `FocusHandle` to track focus. `gpui-component` manages this internally for Input and List.

To focus an element programmatically:

```rust
// Focus input
self.input_state.update(cx, |state, cx| state.focus(window, cx));

// Focus list
self.list_state.update(cx, |state, cx| state.focus(window, cx));
```

## Window Resize

Resize the window based on content (e.g., search results).

```rust
fn update_window_height(&self, item_count: usize, window: &mut Window) {
    let item_height = 24.0;
    let input_height = 40.0;
    let max_height = 400.0;
    
    let content_height = input_height + (item_count as f32 * item_height);
    let new_height = content_height.min(max_height);
    
    window.resize(size(px(300.0), px(new_height)));
}
```

## State Updates

Use `cx.notify()` to trigger a re-render of the current view.
When updating `Entity` state (like `ListState`), use `.update()`:

```rust
self.list_state.update(cx, |state, cx| {
    state.delegate_mut().items = new_items;
    cx.notify(); // Notify the ListState view
});
```

## Low-Level Patterns (verified)

Alternative to gpui-component for full control.

### Borderless Popup Window
```rust
WindowOptions {
    titlebar: None,
    window_decorations: Some(WindowDecorations::Client),
    kind: WindowKind::PopUp,
    focus: true,
    ..Default::default()
}
```

### Focusable Trait
Views that need focus must implement `Focusable`:
```rust
impl Focusable for MyView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
```

### Manual Focus
```rust
cx.open_window(options, |window, cx| {
    let view = cx.new(|cx| MyView::new(cx));
    window.focus(&view.focus_handle(cx));  // 1 arg only
    view
})
```

### Key Handling on Elements
```rust
div()
    .id("my-element")
    .track_focus(&self.focus_handle)
    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
        match event.keystroke.key.as_str() {
            "backspace" => { this.query.pop(); cx.notify(); }
            "a" => { this.query.push('a'); cx.notify(); }
            _ => {}
        }
    }))
```

## Complete Example

A fully functional launcher with Input, List, and dynamic resizing.

```rust
use gpui::*;
use gpui_component::{
    input::{Input, InputState},
    list::{List, ListDelegate, ListState, ListItem},
    IndexPath, Sizable,
};

struct MyDelegate {
    items: Vec<String>,
    matches: Vec<String>,
    selected_index: Option<IndexPath>,
}

impl MyDelegate {
    fn new(items: Vec<String>) -> Self {
        Self {
            matches: items.clone(),
            items,
            selected_index: None,
        }
    }

    fn filter(&mut self, query: &str) {
        if query.is_empty() {
            self.matches = self.items.clone();
        } else {
            self.matches = self.items
                .iter()
                .filter(|i| i.to_lowercase().contains(&query.to_lowercase()))
                .cloned()
                .collect();
        }
    }
}

impl ListDelegate for MyDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.matches.len()
    }

    fn render_item(&mut self, ix: IndexPath, _window: &mut Window, _cx: &mut Context<ListState<Self>>) -> Option<Self::Item> {
        Some(ListItem::new(("item", ix.row))
            .child(self.matches[ix.row].clone()))
    }

    fn set_selected_index(&mut self, ix: Option<IndexPath>, _window: &mut Window, _cx: &mut Context<ListState<Self>>) {
        self.selected_index = ix;
    }
}

struct AppView {
    input_state: Entity<InputState>,
    list_state: Entity<ListState<MyDelegate>>,
}

impl AppView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let items = vec!["Apple", "Banana", "Cherry"].into_iter().map(String::from).collect();
        let delegate = MyDelegate::new(items);
        let list_state = cx.new(|cx| ListState::new(delegate, window, cx));
        let input_state = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));
        
        Self { input_state, list_state }
    }

    fn on_input_change(&mut self, _: &gpui_component::input::InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.input_state.read(cx).value();
        self.list_state.update(cx, |state, cx| {
             state.delegate_mut().filter(&query);
             cx.notify();
        });
        
        let count = self.list_state.read(cx).delegate().matches.len();
        let height = 40.0 + (count as f32 * 24.0).min(300.0);
        window.resize(size(px(300.0), px(height)));
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Input::new(&self.input_state))
            .child(List::new(&self.list_state).h_full())
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| {
                let mut view = AppView::new(window, cx);
                let input = view.input_state.clone();
                cx.subscribe_in(&input, window, |this: &mut AppView, _, event, window, cx| {
                    this.on_input_change(event, window, cx);
                }).detach();
                view
            })
        }).unwrap();
    });
}
```
