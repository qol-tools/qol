use super::{GistMetadata, GistStore};
use anyhow::{anyhow, Result};
use std::collections::HashMap;

pub struct MemoryGistStore {
    gists: HashMap<String, (GistMetadata, HashMap<String, String>)>,
}

impl MemoryGistStore {
    pub fn new() -> Self {
        Self {
            gists: HashMap::new(),
        }
    }

    pub fn add_gist(&mut self, metadata: GistMetadata, files: HashMap<String, String>) {
        self.gists.insert(metadata.id.clone(), (metadata, files));
    }
}

impl Default for MemoryGistStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl GistStore for MemoryGistStore {
    async fn list(&self, _token: &str) -> Result<Vec<GistMetadata>> {
        Ok(self.gists.values().map(|(meta, _)| meta.clone()).collect())
    }

    async fn fetch_file(&self, _token: &str, gist_id: &str, file_name: &str) -> Result<String> {
        let (_, files) = self
            .gists
            .get(gist_id)
            .ok_or_else(|| anyhow!("gist not found: {gist_id}"))?;
        files
            .get(file_name)
            .cloned()
            .ok_or_else(|| anyhow!("file {file_name} not found in gist {gist_id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, description: &str, files: &[&str]) -> GistMetadata {
        GistMetadata {
            id: id.to_string(),
            description: description.to_string(),
            files: files.iter().map(|s| s.to_string()).collect(),
            updated_at: "2026-05-23T00:00:00Z".to_string(),
            public: false,
        }
    }

    fn file_map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn list_empty_store_returns_no_gists() {
        let store = MemoryGistStore::new();
        let listed = store.list("any-token").await.unwrap();
        assert!(listed.is_empty(), "empty store should list no gists");
    }

    #[tokio::test]
    async fn add_gist_then_list_returns_it() {
        let mut store = MemoryGistStore::new();
        let m = meta("g1", "first", &["a.txt"]);
        store.add_gist(m.clone(), file_map(&[("a.txt", "hello")]));

        let listed = store.list("any-token").await.unwrap();
        assert_eq!(listed.len(), 1, "expected exactly one gist after add");
        assert_eq!(listed[0].id, "g1", "id should round-trip through the store");
        assert_eq!(listed[0].description, "first");
        assert_eq!(listed[0].files, vec!["a.txt".to_string()]);
    }

    #[tokio::test]
    async fn fetch_file_returns_content_for_known_file() {
        let mut store = MemoryGistStore::new();
        store.add_gist(
            meta("g1", "first", &["a.txt", "b.txt"]),
            file_map(&[("a.txt", "alpha"), ("b.txt", "beta")]),
        );

        let cases = [("a.txt", "alpha"), ("b.txt", "beta")];
        for (file_name, expected) in cases {
            let got = store.fetch_file("tok", "g1", file_name).await.unwrap();
            assert_eq!(got, expected, "file: {file_name}");
        }
    }

    #[tokio::test]
    async fn fetch_file_unknown_gist_errors_with_descriptive_message() {
        let store = MemoryGistStore::new();
        let err = store
            .fetch_file("tok", "missing-id", "a.txt")
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("missing-id"),
            "error should mention the gist id, got: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("not found"),
            "error should say not found, got: {msg}"
        );
    }

    #[tokio::test]
    async fn fetch_file_unknown_file_in_known_gist_errors_with_descriptive_message() {
        let mut store = MemoryGistStore::new();
        store.add_gist(
            meta("g1", "first", &["a.txt"]),
            file_map(&[("a.txt", "alpha")]),
        );
        let err = store
            .fetch_file("tok", "g1", "missing.txt")
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("missing.txt") && msg.contains("g1"),
            "error should mention both file name and gist id, got: {msg}"
        );
    }
}
