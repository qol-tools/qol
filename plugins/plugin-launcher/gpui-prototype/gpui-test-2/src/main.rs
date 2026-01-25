use gpui::*;
use gpui_component::{
    input::{Input, InputState},
    list::{List, ListDelegate, ListState, ListItem},
    IndexPath, Sizable,
};

#[derive(Clone)]
struct MyDelegate {
    items: Vec<String>, // Just strings for simplicity
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
        let items = vec![
            "Apple".to_string(),
            "Banana".to_string(),
            "Cherry".to_string(),
            "Date".to_string(),
            "Elderberry".to_string(),
            "Fig".to_string(),
            "Grape".to_string(),
            "Honeydew".to_string(),
        ];

        let delegate = MyDelegate::new(items);
        let list_state = cx.new(|cx| ListState::new(delegate, window, cx));
        
        // Input state needs to communicate with list
        let input_state = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));
        
        Self {
            input_state,
            list_state,
        }
    }

    fn on_input_change(&mut self, _: &gpui_component::input::InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.input_state.read(cx).value();
        self.list_state.update(cx, |state, cx| {
             state.delegate_mut().filter(&query);
             cx.notify();
        });
        
        // Window resize logic
        let count = self.list_state.read(cx).delegate().matches.len();
        let height = 40.0 + (count as f32 * 24.0).min(300.0); // Simple calculation
        window.resize(size(px(300.0), px(height)));
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // We need to listen to input changes.
        // The simplest way with gpui-component might be to subscribe in `new`.
        // But for this example, I'll just re-render.
        // Wait, input state changes don't trigger AppView render unless we observe them.
        
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .flex_col()
            .child(
                Input::new(&self.input_state)
            )
            .child(
                List::new(&self.list_state)
                    .with_size(gpui_component::Size::Large)
                    .h_full()
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(300.), px(400.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let view = AppView::new(window, cx);
                    
                    // Subscribe to input changes to filter list
                    let input = view.input_state.clone();
                    cx.subscribe_in(&input, window, |this: &mut AppView, _, event: &gpui_component::input::InputEvent, window, cx| {
                        this.on_input_change(event, window, cx);
                    }).detach();
                    
                    view
                })
            },
        )
        .unwrap();
    });
}
