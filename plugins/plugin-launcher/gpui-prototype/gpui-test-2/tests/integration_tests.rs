use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, WindowOptions, div, px, size, SharedString,
};
use gpui_component::{
    input::{Input, InputState},
    list::{List, ListDelegate, ListItem, ListState},
    IndexPath,
};
use proptest::prelude::*;

// --- Re-implementation of App Logic for Testing ---

#[derive(Clone)]
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
        if ix.row >= self.matches.len() {
            return None;
        }
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
    fn new(items: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            .child(div().size_full().child(List::new(&self.list_state)))
    }
}

// --- Tests ---

#[gpui::test]
fn test_filter_logic_basic(cx: &mut TestAppContext) {
    let window_handle = cx.update(|cx| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
             cx.new(|cx| AppView::new(vec!["Apple".into(), "Banana".into()], window, cx))
        }).unwrap()
    });

    let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
    let view = window_handle.root(&mut cx).unwrap();

    // Initial state
    cx.update(|_window, cx| {
        let list = view.read(cx).list_state.read(cx);
        assert_eq!(list.delegate().matches.len(), 2);
    });

    // Simulate input change
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.input_state.update(cx, |input, cx| {
                input.set_value("Ap", window, cx);
            });
            view.on_input_change(&gpui_component::input::InputEvent::Change, window, cx);
        });
    });

    // Verify filter
    cx.update(|_window, cx| {
        let list = view.read(cx).list_state.read(cx);
        assert_eq!(list.delegate().matches.len(), 1);
        assert_eq!(list.delegate().matches[0], "Apple");
    });
}

#[gpui::test]
fn test_window_resizing(cx: &mut TestAppContext) {
    let window_handle = cx.update(|cx| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
             cx.new(|cx| AppView::new(vec!["A".into(), "B".into(), "C".into()], window, cx))
        }).unwrap()
    });

    let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
    let view = window_handle.root(&mut cx).unwrap();

    // Trigger resize logic
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
             view.on_input_change(&gpui_component::input::InputEvent::Change, window, cx);
        });
    });
    
    // Verify window size: 40 + 3*24 = 112
    cx.update(|window, _cx| {
        let bounds = window.bounds();
        let height = bounds.size.height;
        assert_eq!(height, px(112.0));
    });

    // Filter to 0 items
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
             view.input_state.update(cx, |input, cx| {
                 input.set_value("Z", window, cx);
             });
             view.on_input_change(&gpui_component::input::InputEvent::Change, window, cx);
        });
    });

    // Verify window size: 40 + 0 = 40
    cx.update(|window, _cx| {
        let bounds = window.bounds();
        let height = bounds.size.height;
        assert_eq!(height, px(40.0));
    });
}

#[gpui::test]
fn test_dynamic_growth_and_scroll_limits(cx: &mut TestAppContext) {
    let window_handle = cx.update(|cx| {
        gpui_component::init(cx);
        cx.open_window(WindowOptions::default(), |window, cx| {
             cx.new(|cx| AppView::new(vec![], window, cx))
        }).unwrap()
    });

    let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
    let view = window_handle.root(&mut cx).unwrap();

    // 0. Trigger initial resize to set "empty" state
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
             view.on_input_change(&gpui_component::input::InputEvent::Change, window, cx);
        });
    });

    // 1. Start empty (Height 40)
    cx.update(|window, _| {
        assert_eq!(window.bounds().size.height, px(40.0));
    });

    // 2. Add 5 items (should fit: 40 + 5*24 = 160)
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let mut items = vec![];
            for i in 0..5 { items.push(format!("Item {}", i)); }
            
            // Directly update delegate items (source of truth)
            view.list_state.update(cx, |state, cx| {
                state.delegate_mut().items = items;
                cx.notify();
            });
            
            // Trigger resize using APP LOGIC
            view.on_input_change(&gpui_component::input::InputEvent::Change, window, cx);
        });
    });

    cx.update(|window, _| {
        assert_eq!(window.bounds().size.height, px(160.0));
    });

    // 3. Add 50 items (should cap at 300px content height)
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let mut items = vec![];
            for i in 0..50 { items.push(format!("Item {}", i)); }
            
            // Directly update delegate items (source of truth)
            view.list_state.update(cx, |state, cx| {
                state.delegate_mut().items = items;
                cx.notify();
            });
            
            // Trigger resize using APP LOGIC
            view.on_input_change(&gpui_component::input::InputEvent::Change, window, cx);
        });
    });

    cx.update(|window, _| {
        // App logic: 40 + (count * 24).min(300)
        // 40 + 300 = 340
        assert_eq!(window.bounds().size.height, px(340.0));
    });
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_test_delegate_filter(
        query in "[a-zA-Z0-9]*",
        items in prop::collection::vec("[a-zA-Z0-9]+", 0..50)
    ) {
        let mut delegate = MyDelegate::new(items.clone());
        delegate.filter(&query);
        
        let query_lower = query.to_lowercase();
        for item in &delegate.matches {
            assert!(item.to_lowercase().contains(&query_lower));
        }
        
        if query.is_empty() {
             assert_eq!(delegate.matches.len(), items.len());
        } else {
             assert!(delegate.matches.len() <= items.len());
        }
    }
}
