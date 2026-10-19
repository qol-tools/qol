use std::time::Duration;

use qol_connection_health::Outcome;

const TARGET: (&str, u16) = ("api.github.com", 443);
pub(super) const TIMEOUT: Duration = Duration::from_secs(6);

pub(super) async fn run(watch: super::Watch) {
    let outcome = reach().await;
    watch.end(Some(outcome));
}

async fn reach() -> Outcome {
    match tokio::time::timeout(TIMEOUT, tokio::net::TcpStream::connect(TARGET)).await {
        Ok(Ok(_)) => Outcome::Answered,
        Ok(Err(_)) | Err(_) => Outcome::Unreachable,
    }
}
