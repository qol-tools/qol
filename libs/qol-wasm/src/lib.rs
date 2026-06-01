use qol_search::fuzzy_match as api_fuzzy_match;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn fuzzy_match(query: &str, candidate: &str) -> JsValue {
    match api_fuzzy_match(query, candidate) {
        Some(m) => serde_wasm_bindgen::to_value(&m).unwrap(),
        None => JsValue::NULL,
    }
}
