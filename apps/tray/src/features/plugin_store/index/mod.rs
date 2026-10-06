mod catalog;
mod document;
mod fetch;
#[cfg(test)]
mod install_tests;
mod registry;
mod stage;

pub(crate) use catalog::list_plugins;
pub(crate) use document::IndexedRelease;
pub(crate) use registry::download_asset;
pub(crate) use stage::{latest_version, load_config_contract, stage_release};

const CORE_INDEX_URL: &str = "https://qol-tools.github.io/qol/plugins/index.json";
const CORE_INDEX_PUBLIC_KEY_FILE: &str = include_str!("plugin-index.pub");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexLocation {
    pub(crate) url: String,
    pub(crate) public_key: String,
}

pub(crate) fn core_index() -> IndexLocation {
    IndexLocation {
        url: CORE_INDEX_URL.to_string(),
        public_key: qol_plugin_index::public_key_line(CORE_INDEX_PUBLIC_KEY_FILE).to_string(),
    }
}

#[derive(Debug)]
struct Unavailable(String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unavailable {}

fn unavailable(message: String) -> anyhow::Error {
    anyhow::Error::new(Unavailable(message))
}

pub(crate) fn is_unavailable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Unavailable>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::plugin_store::source::PluginSource;

    #[test]
    fn core_index_key_is_the_committed_minisign_public_key() {
        let key = core_index().public_key;
        assert!(
            minisign_verify::PublicKey::from_base64(&key).is_ok(),
            "{key}"
        );
        assert!(CORE_INDEX_PUBLIC_KEY_FILE.starts_with("untrusted comment:"));
    }

    #[tokio::test]
    async fn only_an_unavailable_index_falls_back_to_github_releases() {
        let indexed =
            PluginSource::new("core", "qol-tools/qol", "main").with_signed_index(core_index());
        let github_only = PluginSource::new("user", "someone/plugins", "main");
        let cases: Vec<(&PluginSource, anyhow::Result<&str>, Result<&str, &str>)> = vec![
            (&indexed, Ok("index"), Ok("index")),
            (
                &indexed,
                Err(unavailable("index.json answered 404".to_string())),
                Ok("github"),
            ),
            (
                &indexed,
                Err(unavailable("index.json answered 404".to_string()).context("listing")),
                Ok("github"),
            ),
            (
                &indexed,
                Err(anyhow::anyhow!("the plugin index signature does not match")),
                Err("signature"),
            ),
            (&github_only, Ok("index"), Ok("github")),
        ];
        for (source, from_index, expected) in cases {
            let label = format!("{} {:?}", source.name, from_index);
            let result = source
                .read_catalog(|_| async move { from_index }, || async { Ok("github") })
                .await;
            match (result, expected) {
                (Ok(got), Ok(want)) => assert_eq!(got, want, "{label}"),
                (Err(error), Err(want)) => assert!(format!("{error:#}").contains(want), "{label}"),
                (got, want) => panic!("{label}: got {got:?}, want {want:?}"),
            }
        }
    }
}
