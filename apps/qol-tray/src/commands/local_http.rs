use std::time::Duration;

use qol_runtime::local_http::{Client, Method};

pub fn get_from_daemon(path: &str) -> std::io::Result<(u16, String)> {
    let token = read_token()?;
    let response = Client::new(crate::features::plugin_store::DEFAULT_SERVER_PORT, token)
        .with_io_timeout(Duration::from_secs(5))
        .request(Method::Get, path, None)?;
    Ok((response.status, response.body))
}

pub fn post_to_daemon(path: &str, body: &str) -> std::io::Result<(u16, String)> {
    let token = read_token()?;
    let body = if body.is_empty() { "{}" } else { body };
    let response = Client::new(crate::features::plugin_store::DEFAULT_SERVER_PORT, token)
        .with_io_timeout(Duration::from_secs(5))
        .request(Method::Post, path, Some(body))?;
    Ok((response.status, response.body))
}

pub fn browser_url(route: &str, port: u16) -> String {
    let token = read_token().ok();
    qol_conventions::local_hash_url_with_token(route, port, token.as_deref())
}

fn read_token() -> std::io::Result<String> {
    crate::features::plugin_store::server::security::current_token().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "valid HTTP auth token is unavailable",
        )
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn browser_url_keeps_token_in_fragment() {
        let port = qol_conventions::DEFAULT_PORT;
        let url = qol_conventions::local_hash_url_with_token("shortcuts", port, Some("secret"));

        assert_eq!(
            url,
            format!("http://127.0.0.1:{port}/#shortcuts?qol_token=secret")
        );
    }
}
