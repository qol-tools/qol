use std::sync::OnceLock;
use std::time::Duration;

const TARGET: &str = "https://api.github.com/rate_limit";
pub(super) const TIMEOUT: Duration = Duration::from_secs(6);

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub(super) async fn run(watch: super::Watch) {
    let result = client().get(TARGET).send().await;
    watch.end(super::outcome(&result));
}

fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent("qol-tray")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}
