use qol_tray::menu::router::{EventHandler, EventPattern, EventRoute, EventRouter, HandlerResult};
use std::sync::{Arc, Mutex};

#[test]
fn exact_pattern_matches_only_exact_string() {
    // Arrange
    let pattern = EventPattern::Exact("quit".to_string());

    // Act
    let matches_quit = pattern.matches("quit");
    let matches_quit_now = pattern.matches("quit_now");
    let matches_qui = pattern.matches("qui");
    let matches_reload = pattern.matches("reload");

    // Assert
    assert!(matches_quit);
    assert!(!matches_quit_now);
    assert!(!matches_qui);
    assert!(!matches_reload);
}

#[test]
fn prefix_pattern_matches_strings_with_prefix() {
    // Arrange
    let pattern = EventPattern::Prefix("plugin::".to_string());

    // Act
    let matches_action = pattern.matches("plugin::action");
    let matches_empty = pattern.matches("plugin::");
    let matches_nested = pattern.matches("plugin::sub::action");
    let matches_partial = pattern.matches("plugi");
    let matches_other = pattern.matches("other::action");

    // Assert
    assert!(matches_action);
    assert!(matches_empty);
    assert!(matches_nested);
    assert!(!matches_partial);
    assert!(!matches_other);
}

#[test]
fn router_routes_to_first_matching_handler() {
    // Arrange
    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    let routes = vec![
        EventRoute {
            pattern: EventPattern::Prefix("plugin::".to_string()),
            handler: EventHandler::Sync(Box::new(move |_| {
                *call_count_clone.lock().unwrap() += 1;
                Ok(HandlerResult::Continue)
            })),
        },
    ];
    let router = EventRouter::new(routes);

    // Act
    let _ = router.route("plugin::action");

    // Assert
    assert_eq!(*call_count.lock().unwrap(), 1);
}

#[test]
fn router_returns_quit_handler_result() {
    // Arrange
    let routes = vec![
        EventRoute {
            pattern: EventPattern::Exact("quit".to_string()),
            handler: EventHandler::Sync(Box::new(|_| Ok(HandlerResult::Quit))),
        },
    ];
    let router = EventRouter::new(routes);

    // Act
    let result = router.route("quit").unwrap();

    // Assert
    match result {
        HandlerResult::Quit => {},
        _ => panic!("Expected Quit result"),
    }
}

#[test]
fn router_returns_continue_for_unmatched_events() {
    // Arrange
    let routes = vec![
        EventRoute {
            pattern: EventPattern::Exact("quit".to_string()),
            handler: EventHandler::Sync(Box::new(|_| Ok(HandlerResult::Quit))),
        },
    ];
    let router = EventRouter::new(routes);

    // Act
    let result = router.route("unknown").unwrap();

    // Assert
    match result {
        HandlerResult::Continue => {},
        _ => panic!("Expected Continue result for unmatched event"),
    }
}

#[test]
fn router_passes_event_id_to_handler() {
    // Arrange
    let received_id = Arc::new(Mutex::new(String::new()));
    let received_id_clone = received_id.clone();

    let routes = vec![
        EventRoute {
            pattern: EventPattern::Prefix("plugin::".to_string()),
            handler: EventHandler::Sync(Box::new(move |event_id| {
                *received_id_clone.lock().unwrap() = event_id.to_string();
                Ok(HandlerResult::Continue)
            })),
        },
    ];
    let router = EventRouter::new(routes);

    // Act
    let _ = router.route("plugin::test_action");

    // Assert
    assert_eq!(*received_id.lock().unwrap(), "plugin::test_action");
}

#[test]
fn router_uses_first_matching_route_when_multiple_match() {
    // Arrange
    let first_called = Arc::new(Mutex::new(false));
    let second_called = Arc::new(Mutex::new(false));

    let first_clone = first_called.clone();
    let second_clone = second_called.clone();

    let routes = vec![
        EventRoute {
            pattern: EventPattern::Prefix("plugin::".to_string()),
            handler: EventHandler::Sync(Box::new(move |_| {
                *first_clone.lock().unwrap() = true;
                Ok(HandlerResult::Continue)
            })),
        },
        EventRoute {
            pattern: EventPattern::Prefix("plugin::".to_string()),
            handler: EventHandler::Sync(Box::new(move |_| {
                *second_clone.lock().unwrap() = true;
                Ok(HandlerResult::Continue)
            })),
        },
    ];
    let router = EventRouter::new(routes);

    // Act
    let _ = router.route("plugin::action");

    // Assert
    assert!(*first_called.lock().unwrap());
    assert!(!*second_called.lock().unwrap());
}
