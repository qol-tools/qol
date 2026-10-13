use super::ax::Element;

const MAX_DEPTH: usize = 8;
const BANNER: &str = "AXNotificationCenterBanner";
const HISTORY_LIST: &str = "AXNotificationListItems";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Node {
    pub(super) subrole: String,
    pub(super) identifier: String,
    pub(super) description: String,
    pub(super) value: String,
    pub(super) children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Banner {
    pub(super) id: String,
    pub(super) app: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) body: String,
}

pub(super) fn snapshot(element: &Element) -> Node {
    snapshot_at(element, 0)
}

fn snapshot_at(element: &Element, depth: usize) -> Node {
    let text = |name| element.text(name).unwrap_or_default();
    Node {
        subrole: text("AXSubrole"),
        identifier: text("AXIdentifier"),
        description: text("AXDescription"),
        value: text("AXValue"),
        children: if depth < MAX_DEPTH {
            element
                .children()
                .iter()
                .map(|child| snapshot_at(child, depth + 1))
                .collect()
        } else {
            Vec::new()
        },
    }
}

pub(super) fn banners(root: &Node) -> Vec<Banner> {
    let mut found = Vec::new();
    if !contains_history(root) {
        collect(root, &mut found);
    }
    found
}

fn contains_history(node: &Node) -> bool {
    node.identifier == HISTORY_LIST || node.children.iter().any(contains_history)
}

fn collect(node: &Node, found: &mut Vec<Banner>) {
    if node.subrole == BANNER {
        found.push(banner(node));
        return;
    }
    for child in &node.children {
        collect(child, found);
    }
}

fn banner(node: &Node) -> Banner {
    let field = |id: &str| {
        node.children
            .iter()
            .find(|child| child.identifier == id)
            .map(|child| child.value.clone())
            .unwrap_or_default()
    };
    let title = field("title");
    let subtitle = field("subtitle");
    let body = field("body");
    let app = app_name(&node.description, [&title, &subtitle, &body]);
    let id = if node.identifier.is_empty() {
        format!("{app}\n{title}\n{subtitle}\n{body}")
    } else {
        node.identifier.clone()
    };
    Banner {
        id,
        app,
        title,
        subtitle,
        body,
    }
}

fn app_name(description: &str, parts: [&String; 3]) -> String {
    let content = parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if !content.is_empty() {
        if let Some(app) = description.strip_suffix(content.as_str()) {
            if let Some(app) = app.strip_suffix(", ") {
                return app.to_string();
            }
        }
    }
    description
        .split_once(", ")
        .map_or(description, |(app, _)| app)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(id: &str, value: &str) -> Node {
        Node {
            identifier: id.to_string(),
            value: value.to_string(),
            ..Node::default()
        }
    }

    fn group(subrole: &str, id: &str, description: &str, children: Vec<Node>) -> Node {
        Node {
            subrole: subrole.to_string(),
            identifier: id.to_string(),
            description: description.to_string(),
            children,
            ..Node::default()
        }
    }

    fn window(children: Vec<Node>) -> Node {
        group("AXSystemDialog", "", "", vec![group("", "", "", children)])
    }

    #[test]
    fn reads_a_live_banner_with_commas_in_its_text() {
        let root = window(vec![group(
            BANNER,
            "AF6B",
            "Microsoft Teams, Ada, Lovelace, #general, lunch, today?",
            vec![
                text("title", "Ada, Lovelace"),
                text("subtitle", "#general"),
                text("body", "lunch, today?"),
            ],
        )]);
        assert_eq!(
            banners(&root),
            vec![Banner {
                id: "AF6B".to_string(),
                app: "Microsoft Teams".to_string(),
                title: "Ada, Lovelace".to_string(),
                subtitle: "#general".to_string(),
                body: "lunch, today?".to_string(),
            }]
        );
    }

    #[test]
    fn ignores_the_open_notification_center_panel() {
        let root = window(vec![group(
            "",
            HISTORY_LIST,
            "",
            vec![
                group(BANNER, "1", "Tips, hi", vec![text("title", "hi")]),
                group(
                    "AXNotificationCenterBannerStack",
                    "2",
                    "Teams, x, stacked",
                    vec![],
                ),
            ],
        )]);
        assert!(banners(&root).is_empty());
    }

    #[test]
    fn leaves_alerts_and_stacks_alone() {
        let root = window(vec![
            group("AXNotificationCenterAlert", "1", "Reminders, due", vec![]),
            group("AXNotificationCenterBannerStack", "2", "Teams, x", vec![]),
        ]);
        assert!(banners(&root).is_empty());
    }

    #[test]
    fn app_name_cases() {
        let empty = String::new();
        let hi = "hi".to_string();
        let cases = [
            ("Slack, hi", [&hi, &empty, &empty], "Slack"),
            ("Slack, a, b", [&hi, &empty, &empty], "Slack"),
            ("Slack", [&empty, &empty, &empty], "Slack"),
            ("", [&hi, &empty, &empty], ""),
        ];
        for (description, parts, want) in cases {
            assert_eq!(app_name(description, parts), want, "{description:?}");
        }
    }
}
