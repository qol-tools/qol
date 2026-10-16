use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct FlowDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<DetailText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_fact: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct DetailText {
    #[serde(default)]
    pub head: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tail: Vec<String>,
    #[serde(default)]
    pub lines: usize,
    #[serde(default)]
    pub chars: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub numbered: bool,
}

impl DetailText {
    pub fn hidden(&self) -> usize {
        self.lines.saturating_sub(self.head.len() + self.tail.len())
    }

    pub fn tail_start(&self) -> usize {
        self.lines.saturating_sub(self.tail.len()) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_long_excerpt_round_trips_and_numbers_its_tail_from_the_end() {
        let detail = FlowDetail {
            text: Some(DetailText {
                head: vec!["l1".to_string(), "l2".to_string()],
                tail: vec!["l1283".to_string(), "l1284".to_string()],
                lines: 1_284,
                chars: 9_000,
                numbered: true,
            }),
            shown_fact: Some("Size".to_string()),
        };

        let wire = serde_json::to_value(&detail).unwrap();

        assert_eq!(wire["text"]["lines"], 1_284);
        assert_eq!(wire["shown_fact"], "Size");
        assert_eq!(serde_json::from_value::<FlowDetail>(wire).unwrap(), detail);
        let text = detail.text.unwrap();
        assert_eq!(text.hidden(), 1_280);
        assert_eq!(text.tail_start(), 1_283);
    }

    #[test]
    fn an_empty_answer_is_a_detail_without_text() {
        let detail: FlowDetail = serde_json::from_value(json!({})).unwrap();

        assert_eq!(detail, FlowDetail::default());
        let raw = json!({ "head": ["a", "b"], "lines": 2, "chars": 3 });
        let short: DetailText = serde_json::from_value(raw).unwrap();
        assert_eq!(short.hidden(), 0);
        assert!(!short.numbered);
    }
}
