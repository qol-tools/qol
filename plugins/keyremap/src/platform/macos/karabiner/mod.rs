pub(crate) mod compile;
mod key_codes;

use serde_json::{json, Value};

pub(crate) fn report(compiled: &compile::Compiled) -> Value {
    json!({
        "complex_modifications_rule": compiled.rule,
        "simple_modifications": compiled.simple_modifications,
        "not_compiled": compiled
            .gaps
            .iter()
            .map(|gap| json!({ "rule": gap.rule, "reason": gap.reason }))
            .collect::<Vec<_>>(),
    })
}
