use qol_conventions::local_hash_url_with_token;

pub fn browser_url(route: &str, port: u16) -> String {
    let token = crate::features::plugin_store::server::security::current_token();
    local_hash_url_with_token(route, port, token.as_deref())
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
