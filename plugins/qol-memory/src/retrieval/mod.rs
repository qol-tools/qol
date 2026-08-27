use std::cmp::Ordering;
use std::collections::HashMap;

use crate::text::{collapse_ws, tokens, utf16_index_of, utf16_len, utf16_slice};

pub mod cache;

pub struct DocRef<'a> {
    pub key: &'a str,
    pub text: &'a str,
}

pub struct IndexDoc {
    pub key: String,
    pub tf: HashMap<String, u32>,
    pub len: usize,
}

pub struct Index {
    pub docs: Vec<IndexDoc>,
    pub idf: HashMap<String, f64>,
    pub df: HashMap<String, u32>,
    pub n: usize,
    pub avgdl: f64,
    pub total_length: usize,
}

pub struct Ranked {
    pub key: String,
    pub score: f64,
}

fn idf_value(n_docs: usize, df: u32) -> f64 {
    (1.0f64 + (n_docs as f64 - f64::from(df) + 0.5) / (f64::from(df) + 0.5)).ln()
}

pub fn build_index(items: &[DocRef<'_>]) -> Index {
    let mut df: HashMap<String, u32> = HashMap::new();
    let mut docs: Vec<IndexDoc> = Vec::with_capacity(items.len());
    let mut total_length = 0usize;
    for item in items {
        let mut tf: HashMap<String, u32> = HashMap::new();
        for t in tokens(item.text) {
            *tf.entry(t).or_insert(0) += 1;
        }
        for t in tf.keys() {
            *df.entry(t.clone()).or_insert(0) += 1;
        }
        let len = utf16_len(item.text);
        total_length += len;
        docs.push(IndexDoc {
            key: item.key.to_string(),
            tf,
            len,
        });
    }
    let n = docs.len();
    let avgdl = total_length as f64 / std::cmp::max(1, n) as f64;
    let idf: HashMap<String, f64> = df
        .iter()
        .map(|(t, count)| (t.clone(), idf_value(n, *count)))
        .collect();
    Index {
        docs,
        idf,
        df,
        n,
        avgdl,
        total_length,
    }
}

pub fn bm25_ranks(query: &str, idx: &Index, k: usize) -> Vec<Ranked> {
    let qt = tokens(query);
    if qt.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(String, f64)> = Vec::with_capacity(idx.docs.len());
    for doc in &idx.docs {
        let mut s = 0.0f64;
        for t in &qt {
            let Some(f) = doc.tf.get(t) else { continue };
            if *f == 0 {
                continue;
            }
            let w = idx.idf.get(t).copied().unwrap_or(0.0);
            let ff = f64::from(*f);
            s += (w * ff * 1.2) / (ff + 1.2 * (1.0 - 0.75 + 0.75 * (doc.len as f64 / idx.avgdl)));
        }
        scored.push((doc.key.clone(), s));
    }
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
    });
    let all: Vec<Ranked> = scored
        .into_iter()
        .map(|(key, score)| Ranked { key, score })
        .collect();
    if k == 0 {
        all
    } else {
        all.into_iter().take(k).collect()
    }
}

pub fn snippet(text: &str, match_words: &[String], window: usize) -> String {
    let lower = text.to_lowercase();
    let mut found: Option<usize> = None;
    for word in match_words {
        if let Some(i) = utf16_index_of(&lower, word) {
            found = Some(match found {
                None => i,
                Some(current) => current.min(i),
            });
        }
    }
    let Some(idx) = found else {
        return utf16_slice(text, 0, window);
    };
    let start = idx.saturating_sub(window / 3);
    let mut s = collapse_ws(&utf16_slice(text, start, start + window));
    if start > 0 {
        s.insert_str(0, "...");
    }
    if start + window < utf16_len(text) {
        s.push_str("...");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS: [(&str, &str); 3] = [
        ("a", "alpha beta alpha gamma"),
        ("b", "beta delta"),
        ("c", "epsilon zeta"),
    ];

    fn refs<'a>(corpus: &'a [(&'a str, &'a str)]) -> Vec<DocRef<'a>> {
        corpus
            .iter()
            .map(|(k, t)| DocRef { key: k, text: t })
            .collect()
    }

    #[test]
    fn build_index_stats() {
        let idx = build_index(&refs(&CORPUS));
        assert_eq!(idx.n, 3);
        assert_eq!(idx.total_length, 44);
        assert!((idx.avgdl - 44.0 / 3.0).abs() < 1e-15);
        assert_eq!(idx.df["beta"], 2);
        assert_eq!(idx.df["alpha"], 1);
        assert!((idx.idf["beta"] - (1.0_f64 + (3.0 - 2.0 + 0.5) / (2.0 + 0.5)).ln()).abs() < 1e-15);
        assert!(build_index(&[]).avgdl == 0.0 && build_index(&[]).n == 0);
    }

    #[test]
    fn bm25_node_reference_scores() {
        let idx = build_index(&refs(&CORPUS));
        let ranked = bm25_ranks("alpha beta", &idx, 0);
        assert_eq!(ranked[0].key, "a");
        assert!((ranked[0].score - 0.857760656009398).abs() < 1e-9);
        assert_eq!(ranked[1].key, "b");
        assert!((ranked[1].score - 0.294729116676661).abs() < 1e-9);
        assert_eq!(ranked[2].key, "c");
        assert_eq!(ranked[2].score, 0.0);

        let dup = bm25_ranks("beta beta alpha", &idx, 2);
        assert_eq!(dup.len(), 2);
        assert_eq!(dup[0].key, "a");
        assert!((dup[0].score - 1.0705924881206745).abs() < 1e-9);
        assert_eq!(dup[1].key, "b");
        assert!((dup[1].score - 0.5894582333533216).abs() < 1e-9);

        assert!(bm25_ranks("!!!", &idx, 5).is_empty());
        assert!(bm25_ranks("alpha beta", &build_index(&[]), 5).is_empty());
    }

    #[test]
    fn bm25_tie_break_is_key_ascending_byte_order() {
        let tie_corpus = [
            ("b2", "gamma delta"),
            ("a1", "gamma delta"),
            ("a0", "other words here"),
        ];
        let idx = build_index(&refs(&tie_corpus));
        let ranked = bm25_ranks("gamma", &idx, 0);
        let keys: Vec<&str> = ranked.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["a1", "b2", "a0"]);
        assert!((ranked[0].score - ranked[1].score).abs() < 1e-15);
        assert_eq!(ranked[2].score, 0.0);
        assert_eq!(bm25_ranks("gamma", &idx, 99).len(), 3);
    }

    #[test]
    fn snippet_utf16_and_ellipsis_rules() {
        let text =
            "prefix alpha \u{1f30d} world suffix content that keeps going past the window edge here";
        assert_eq!(
            snippet(text, &["world".to_string()], 10),
            "...\u{1f30d} world s..."
        );
        assert_eq!(snippet(text, &["zzz".to_string()], 8), "prefix a");
        assert_eq!(snippet(text, &["suffix".to_string()], 240), text);
        let long_prefix = format!("{}TARGET tail info", "x".repeat(50));
        assert_eq!(
            snippet(&long_prefix, &["target".to_string()], 10),
            "...xxxTARGET..."
        );
        assert_eq!(snippet("", &["x".to_string()], 4), "");
    }
}
