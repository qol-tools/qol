use super::Shared;

const TARGET: &str = "https://api.github.com/rate_limit";

pub(super) async fn run(shared: &'static Shared) {
    let request = crate::features::plugin_store::github::build_github_request(
        crate::updates::http_client(),
        TARGET,
        None,
    );
    let result = super::observe(shared, request).await;
    shared.probe_finished(super::outcome(&result));
}
