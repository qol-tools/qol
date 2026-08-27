use std::cmp::max;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use super::{build_index, DocRef, Index};
use crate::text::{tokens, utf16_len};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    Fresh,
    Stale,
    Missing,
}

#[derive(Serialize)]
struct CacheFileOut<'a> {
    #[serde(rename = "N")]
    n: usize,
    avgdl: f64,
    #[serde(rename = "totalLength")]
    total_length: usize,
    terms: Vec<String>,
    #[serde(rename = "idfArr")]
    idf_arr: Vec<f64>,
    #[serde(rename = "dfArr")]
    df_arr: Vec<u32>,
    rows: Vec<RowOut<'a>>,
}

#[derive(Serialize)]
struct RowOut<'a> {
    k: &'a str,
    #[serde(rename = "L")]
    l: usize,
    tf: Vec<u32>,
}

#[derive(Deserialize)]
struct CacheFileIn {
    #[serde(rename = "N")]
    n: usize,
    avgdl: f64,
    #[serde(rename = "totalLength")]
    total_length: Option<usize>,
    terms: Vec<String>,
    #[serde(rename = "idfArr")]
    idf_arr: Vec<f64>,
    #[serde(rename = "dfArr")]
    df_arr: Option<Vec<u32>>,
    rows: Vec<RowIn>,
}

#[derive(Deserialize)]
struct RowIn {
    k: String,
    #[serde(rename = "L")]
    l: usize,
    tf: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct MetaIn {
    fp: Option<String>,
    size: Option<u64>,
    count: Option<usize>,
    #[serde(rename = "firstKey")]
    first_key: Option<String>,
    #[serde(rename = "lastKey")]
    last_key: Option<String>,
    fingerprint: Option<String>,
}

#[derive(Serialize)]
struct ProofMetaOut<'a> {
    fp: &'a str,
    size: u64,
    count: usize,
    #[serde(rename = "firstKey")]
    first_key: &'a str,
    #[serde(rename = "lastKey")]
    last_key: &'a str,
    fingerprint: &'a str,
}

#[derive(Serialize)]
struct FingerprintMetaOut<'a> {
    fingerprint: &'a str,
}

struct PrefixProof {
    size: u64,
    count: usize,
    first_key: String,
    last_key: String,
    fp: String,
}

fn hex16(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out.chars().take(16).collect()
}

pub fn persisted_index_path(root: &Path, layer: &str) -> PathBuf {
    root.join(format!("idx-{layer}.json"))
}

pub fn layer_fingerprint(items: &[DocRef<'_>]) -> String {
    let mut hasher = Sha1::new();
    for item in items {
        hasher.update(item.key.as_bytes());
        hasher.update(utf16_len(item.text).to_string().as_bytes());
    }
    hasher.update(items.len().to_string().as_bytes());
    hex16(&hasher.finalize())
}

fn prefix_proof(source_path: &Path, items: &[DocRef<'_>]) -> Option<PrefixProof> {
    let size = fs::metadata(source_path).ok()?.len();
    let count = items.len();
    let first_key = items.first().map(|i| i.key.to_string()).unwrap_or_default();
    let last_key = items.last().map(|i| i.key.to_string()).unwrap_or_default();
    let mut hasher = Sha1::new();
    hasher.update(size.to_string());
    hasher.update(":");
    hasher.update(count.to_string());
    hasher.update(":");
    hasher.update(first_key.as_bytes());
    hasher.update(":");
    hasher.update(last_key.as_bytes());
    Some(PrefixProof {
        size,
        count,
        first_key,
        last_key,
        fp: hex16(&hasher.finalize()),
    })
}

fn meta_path(root: &Path, layer: &str) -> PathBuf {
    let mut os = persisted_index_path(root, layer).into_os_string();
    os.push(".meta");
    PathBuf::from(os)
}

fn read_meta(root: &Path, layer: &str) -> Option<MetaIn> {
    let data = fs::read_to_string(meta_path(root, layer)).ok()?;
    serde_json::from_str(&data).ok()
}

fn load_cache_file(path: &Path) -> anyhow::Result<Index> {
    let file: CacheFileIn = serde_json::from_str(&fs::read_to_string(path)?)?;
    let mut docs: Vec<super::IndexDoc> = Vec::with_capacity(file.rows.len());
    for row in &file.rows {
        let mut tf: HashMap<String, u32> = HashMap::new();
        for pair in row.tf.chunks_exact(2) {
            if let Some(term) = file.terms.get(pair[0] as usize) {
                tf.insert(term.clone(), pair[1]);
            }
        }
        docs.push(super::IndexDoc {
            key: row.k.clone(),
            tf,
            len: row.l,
        });
    }
    let idf: HashMap<String, f64> = file
        .terms
        .iter()
        .zip(file.idf_arr.iter())
        .map(|(t, v)| (t.clone(), *v))
        .collect();
    let df: HashMap<String, u32> = if let Some(arr) = &file.df_arr {
        file.terms
            .iter()
            .zip(arr.iter())
            .map(|(t, c)| (t.clone(), *c))
            .collect()
    } else {
        recount_df(&docs)
    };
    let total_length = file
        .total_length
        .unwrap_or_else(|| docs.iter().map(|d| d.len).sum());
    Ok(Index {
        docs,
        idf,
        df,
        n: file.n,
        avgdl: file.avgdl,
        total_length,
    })
}

fn recount_df(docs: &[super::IndexDoc]) -> HashMap<String, u32> {
    let mut df: HashMap<String, u32> = HashMap::new();
    for d in docs {
        for t in d.tf.keys() {
            *df.entry(t.clone()).or_insert(0) += 1;
        }
    }
    df
}

fn can_merge(proof: &PrefixProof, meta: &MetaIn, items: &[DocRef<'_>]) -> bool {
    let (Some(meta_size), Some(meta_count)) = (meta.size, meta.count) else {
        return false;
    };
    if proof.size <= meta_size {
        return false;
    }
    if items.len() < meta_count {
        return false;
    }
    if meta_count == 0 {
        return true;
    }
    items[meta_count - 1].key == meta.last_key.as_deref().unwrap_or("")
        && items[0].key == meta.first_key.as_deref().unwrap_or("")
}

fn merge_tail(mut cached: Index, tail: &[DocRef<'_>]) -> Index {
    for item in tail {
        let mut tf: HashMap<String, u32> = HashMap::new();
        for t in tokens(item.text) {
            *tf.entry(t).or_insert(0) += 1;
        }
        for t in tf.keys() {
            *cached.df.entry(t.clone()).or_insert(0) += 1;
        }
        let len = utf16_len(item.text);
        cached.total_length += len;
        cached.docs.push(super::IndexDoc {
            key: item.key.to_string(),
            tf,
            len,
        });
    }
    let merged_n = cached.docs.len();
    let merged_total = cached.total_length;
    for (t, count) in &cached.df {
        cached
            .idf
            .insert(t.clone(), super::idf_value(merged_n, *count));
    }
    cached.n = merged_n;
    cached.avgdl = merged_total as f64 / max(1, merged_n) as f64;
    cached
}

fn save_index(
    root: &Path,
    layer: &str,
    idx: &Index,
    items: &[DocRef<'_>],
    source_path: Option<&Path>,
) -> anyhow::Result<()> {
    let p = persisted_index_path(root, layer);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut vocab: HashMap<String, usize> = HashMap::new();
    let mut terms_vec: Vec<String> = Vec::new();
    let mut df_counts: HashMap<String, u32> = HashMap::new();
    let mut rows: Vec<RowOut<'_>> = Vec::with_capacity(items.len());
    let mut total_length = 0usize;
    for (item, doc) in items.iter().zip(idx.docs.iter()) {
        let mut tf_pairs: Vec<u32> = Vec::with_capacity(doc.tf.len() * 2);
        for (t, f) in doc.tf.iter() {
            let next_id = vocab.len();
            let id = *vocab.entry(t.clone()).or_insert(next_id);
            if id == next_id {
                terms_vec.push(t.clone());
            }
            tf_pairs.push(id as u32);
            tf_pairs.push(*f);
            *df_counts.entry(t.clone()).or_insert(0) += 1;
        }
        total_length += doc.len;
        rows.push(RowOut {
            k: item.key,
            l: doc.len,
            tf: tf_pairs,
        });
    }
    let n = idx.n;
    let avgdl = total_length as f64 / max(1, n) as f64;
    let mut idf_arr = vec![0.0f64; terms_vec.len()];
    for (t, v) in idx.idf.iter() {
        if let Some(id) = vocab.get(t) {
            idf_arr[*id] = *v;
        }
    }
    let mut df_arr = vec![0u32; terms_vec.len()];
    for (t, c) in df_counts.iter() {
        if let Some(id) = vocab.get(t) {
            df_arr[*id] = *c;
        }
    }
    let payload = CacheFileOut {
        n,
        avgdl,
        total_length,
        terms: terms_vec,
        idf_arr,
        df_arr,
        rows,
    };
    qol_fs::atomic_write(&p, serde_json::to_string(&payload)?.as_bytes())?;
    let fingerprint = layer_fingerprint(items);
    let proof = source_path.and_then(|sp| prefix_proof(sp, items));
    match proof {
        Some(proof) => qol_fs::atomic_write(
            &meta_path(root, layer),
            serde_json::to_string(&ProofMetaOut {
                fp: &proof.fp,
                size: proof.size,
                count: proof.count,
                first_key: &proof.first_key,
                last_key: &proof.last_key,
                fingerprint: &fingerprint,
            })?
            .as_bytes(),
        )?,
        None => qol_fs::atomic_write(
            &meta_path(root, layer),
            serde_json::to_string(&FingerprintMetaOut {
                fingerprint: &fingerprint,
            })?
            .as_bytes(),
        )?,
    }
    prune_session_caches(root);
    Ok(())
}

fn is_session_cache_name(name: &str) -> bool {
    name.strip_prefix("idx-pool-x-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|mid| !mid.is_empty())
}

fn prune_session_caches(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut candidates: Vec<(String, i128)> = entries
        .flatten()
        .filter(|e| is_session_cache_name(&e.file_name().to_string_lossy()))
        .map(|e| {
            let mtime = e
                .metadata()
                .ok()
                .and_then(|md| md.modified().ok())
                .map(|t| {
                    t.duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i128)
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            (e.file_name().to_string_lossy().into_owned(), mtime)
        })
        .collect();
    candidates.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (name, _) in candidates.iter().skip(5) {
        let _ = fs::remove_file(root.join(name));
        let _ = fs::remove_file(root.join(format!("{name}.meta")));
    }
}

fn try_cached(
    root: &Path,
    layer: &str,
    meta: &MetaIn,
    proof: Option<&PrefixProof>,
    items: &[DocRef<'_>],
    source_path: Option<&Path>,
) -> anyhow::Result<Option<Index>> {
    let p = persisted_index_path(root, layer);
    if let Some(pr) = proof {
        if meta.fp.as_deref() == Some(pr.fp.as_str()) {
            return Ok(Some(load_cache_file(&p)?));
        }
        if can_merge(pr, meta, items) {
            let cached = load_cache_file(&p)?;
            let meta_count = meta.count.unwrap_or(0);
            if items.len() == meta_count {
                if meta.fingerprint.as_deref() == Some(layer_fingerprint(items).as_str()) {
                    qol_fs::atomic_write(
                        &meta_path(root, layer),
                        serde_json::to_string(&ProofMetaOut {
                            fp: &pr.fp,
                            size: pr.size,
                            count: meta_count,
                            first_key: meta.first_key.as_deref().unwrap_or(""),
                            last_key: meta.last_key.as_deref().unwrap_or(""),
                            fingerprint: meta.fingerprint.as_deref().unwrap_or(""),
                        })?
                        .as_bytes(),
                    )?;
                    return Ok(Some(cached));
                }
            } else {
                let merged = merge_tail(cached, &items[meta_count..]);
                save_index(root, layer, &merged, items, source_path)?;
                return Ok(Some(merged));
            }
        }
    }
    if meta.fingerprint.as_deref() == Some(layer_fingerprint(items).as_str()) {
        return Ok(Some(load_cache_file(&p)?));
    }
    Ok(None)
}

pub fn build_or_load(
    root: &Path,
    layer: &str,
    items: &[DocRef<'_>],
    source_path: Option<&Path>,
) -> Index {
    let meta = read_meta(root, layer);
    if let Some(meta) = meta {
        let proof = source_path.and_then(|sp| prefix_proof(sp, items));
        if let Ok(Some(idx)) = try_cached(root, layer, &meta, proof.as_ref(), items, source_path) {
            return idx;
        }
    }
    let idx = build_index(items);
    if let Err(err) = save_index(root, layer, &idx, items, source_path) {
        eprintln!("qol-memory: index save failed for {layer}: {err}");
    }
    idx
}

pub fn cache_state(
    root: &Path,
    layer: &str,
    items: &[DocRef<'_>],
    source_path: Option<&Path>,
) -> CacheState {
    let Some(meta) = read_meta(root, layer) else {
        return CacheState::Missing;
    };
    let proof_matches = source_path
        .and_then(|sp| prefix_proof(sp, items))
        .is_some_and(|pr| meta.fp.as_deref() == Some(pr.fp.as_str()));
    let fingerprint_matches =
        meta.fingerprint.as_deref() == Some(layer_fingerprint(items).as_str());
    if proof_matches || (source_path.is_none() && fingerprint_matches) {
        CacheState::Fresh
    } else {
        CacheState::Stale
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use super::*;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_store(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "qol-memory-cache-test-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn refs<'a>(pairs: &'a [(&'a str, &'a str)]) -> Vec<DocRef<'a>> {
        pairs
            .iter()
            .map(|(k, t)| DocRef { key: k, text: t })
            .collect()
    }

    #[test]
    fn node_reference_fingerprints() {
        let items = refs(&[("u1", "hello world"), ("u2", "second doc: café \u{1f30d}")]);
        assert_eq!(layer_fingerprint(&items), "bc59b35bce75cf8b");
        assert_eq!(layer_fingerprint(&refs(&[])), "b6589fc6ab0dc82c");
    }

    #[test]
    fn node_reference_prefix_proof() {
        let dir = temp_store("prefix-proof");
        let src = dir.join("units-source.txt");
        fs::write(&src, b"hello\n").unwrap();
        let items = refs(&[("u1", "hello world"), ("u2", "second doc text")]);
        let proof = prefix_proof(&src, &items).unwrap();
        assert_eq!(proof.size, 6);
        assert_eq!(proof.fp, "15dddb393c2e31b5");
        assert!(prefix_proof(&dir.join("absent.bin"), &items).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = temp_store("round-trip");
        let items = refs(&[
            ("u1", "hello world alpha alpha"),
            ("u2", "beta gamma tokens here"),
            ("u3", "delta epsilon zeta\u{e9} 42"),
        ]);
        let idx = build_index(&items);
        save_index(&dir, "user", &idx, &items, None).unwrap();
        let loaded = load_cache_file(&persisted_index_path(&dir, "user")).unwrap();
        assert_eq!(loaded.n, idx.n);
        assert_eq!(loaded.avgdl, idx.avgdl);
        assert_eq!(loaded.total_length, idx.total_length);
        for (term, value) in idx.idf.iter() {
            assert_eq!(loaded.idf[term], *value);
        }
        for (built, row) in idx.docs.iter().zip(loaded.docs.iter()) {
            assert_eq!(built.key, row.key);
            assert_eq!(built.len, row.len);
            for (t, f) in built.tf.iter() {
                assert_eq!(row.tf[t], *f);
            }
            assert_eq!(row.tf.len(), built.tf.len());
        }
        let meta_text = fs::read_to_string(meta_path(&dir, "user")).unwrap();
        let meta: MetaIn = serde_json::from_str(&meta_text).unwrap();
        assert_eq!(
            meta.fingerprint.as_deref(),
            Some(layer_fingerprint(&items).as_str())
        );
        assert_eq!(meta.size, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_only_meta_state() {
        let dir = temp_store("fp-only-state");
        let items = refs(&[("a", "one two two"), ("b", "three")]);
        assert_eq!(cache_state(&dir, "user", &items, None), CacheState::Missing);
        let idx = build_or_load(&dir, "user", &items, None);
        assert_eq!(idx.n, 2);
        assert_eq!(cache_state(&dir, "user", &items, None), CacheState::Fresh);
        let other = refs(&[("a", "different words entirely")]);
        assert_eq!(cache_state(&dir, "user", &other, None), CacheState::Stale);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_or_load_merges_grown_source_and_rewrites_meta() {
        let dir = temp_store("merge");
        let source = dir.join("units.jsonl");
        fs::write(&source, b"v1\n").unwrap();
        let first_two = refs(&[("u1", "hello world"), ("u2", "second entry text")]);
        assert_eq!(
            cache_state(&dir, "pool", &first_two, Some(&source)),
            CacheState::Missing
        );
        let idx = build_or_load(&dir, "pool", &first_two, Some(&source));
        assert_eq!(idx.n, 2);
        assert_eq!(
            cache_state(&dir, "pool", &first_two, Some(&source)),
            CacheState::Fresh
        );

        fs::write(&source, b"v1\nv2 plus more bytes\n").unwrap();
        assert_eq!(
            cache_state(&dir, "pool", &first_two, Some(&source)),
            CacheState::Stale
        );

        let three_items = refs(&[
            ("u1", "hello world"),
            ("u2", "second entry text"),
            ("u3", "trailing delta note"),
        ]);
        let merged = build_or_load(&dir, "pool", &three_items, Some(&source));
        assert_eq!(merged.n, 3);
        assert_eq!(
            merged.total_length,
            utf16_len("hello world")
                + utf16_len("second entry text")
                + utf16_len("trailing delta note")
        );
        for term in merged.df.keys() {
            let expected = crate::retrieval::idf_value(3, merged.df[term]);
            assert_eq!(merged.idf[term], expected);
        }
        let meta_text = fs::read_to_string(meta_path(&dir, "pool")).unwrap();
        let meta: MetaIn = serde_json::from_str(&meta_text).unwrap();
        assert_eq!(meta.count, Some(3));
        assert_eq!(meta.last_key.as_deref(), Some("u3"));
        assert_eq!(meta.first_key.as_deref(), Some("u1"));
        assert_eq!(
            meta.fingerprint.as_deref(),
            Some(layer_fingerprint(&three_items).as_str())
        );
        assert_eq!(
            cache_state(&dir, "pool", &three_items, Some(&source)),
            CacheState::Fresh
        );

        fs::write(&source, b"v1\nv2 plus more bytes\nextra byte\n").unwrap();
        let rewritten = build_or_load(&dir, "pool", &three_items, Some(&source));
        assert_eq!(rewritten.n, 3);
        let meta_after: MetaIn =
            serde_json::from_str(&fs::read_to_string(meta_path(&dir, "pool")).unwrap()).unwrap();
        assert_eq!(meta_after.count, Some(3));
        assert_ne!(meta_after.fp.as_deref(), meta.fp.as_deref());
        assert_eq!(
            meta_after.fingerprint.as_deref(),
            meta.fingerprint.as_deref()
        );
        assert_eq!(
            cache_state(&dir, "pool", &three_items, Some(&source)),
            CacheState::Fresh
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
