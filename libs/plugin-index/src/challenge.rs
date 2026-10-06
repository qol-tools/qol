use crate::RegistryLocation;
use anyhow::{Context, Result};
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub struct BearerChallenge {
    pub realm: String,
    pub service: Option<String>,
    pub scope: Option<String>,
}

impl BearerChallenge {
    pub fn parse(header: &str) -> Result<Self> {
        let (scheme, params) = header.trim().split_once(' ').unwrap_or((header, ""));
        if !scheme.eq_ignore_ascii_case("bearer") {
            anyhow::bail!("the registry asked for {scheme} credentials");
        }
        let mut realm = None;
        let mut service = None;
        let mut scope = None;
        for (key, value) in challenge_params(params) {
            match key.as_str() {
                "realm" => realm = Some(value),
                "service" => service = Some(value),
                "scope" => scope = Some(value),
                _ => {}
            }
        }
        Ok(Self {
            realm: realm.context("the registry bearer challenge has no realm")?,
            service,
            scope,
        })
    }

    pub fn token_url(&self, registry: &RegistryLocation) -> Result<Url> {
        let mut url = Url::parse(&self.realm).context("the registry token realm is not a URL")?;
        let registry_url = Url::parse(&registry.url).context("the registry URL is invalid")?;
        if url.scheme() != "https" && url.origin() != registry_url.origin() {
            anyhow::bail!("refusing a non-HTTPS token realm {}", self.realm);
        }
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = &self.service {
                query.append_pair("service", service);
            }
            if let Some(scope) = &self.scope {
                query.append_pair("scope", scope);
            }
        }
        Ok(url)
    }
}

fn challenge_params(params: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut rest = params.trim();
    while let Some((key, after_key)) = rest.split_once('=') {
        let (value, after_value) = challenge_value(after_key.trim_start());
        pairs.push((key.trim().to_ascii_lowercase(), value));
        rest = after_value
            .trim_start()
            .trim_start_matches(',')
            .trim_start();
    }
    pairs
}

fn challenge_value(input: &str) -> (String, &str) {
    let Some(quoted) = input.strip_prefix('"') else {
        let end = input.find(',').unwrap_or(input.len());
        return (input[..end].trim().to_string(), &input[end..]);
    };
    let mut value = String::new();
    let mut chars = quoted.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return (value, &quoted[index + 1..]),
            '\\' => value.extend(chars.next().map(|(_, escaped)| escaped)),
            _ => value.push(ch),
        }
    }
    (value, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(url: &str) -> RegistryLocation {
        RegistryLocation {
            url: url.to_string(),
            repository: "qol-tools/plugins".to_string(),
        }
    }

    #[test]
    fn parses_bearer_challenges_with_quoted_commas() {
        let cases: &[(&str, BearerChallenge)] = &[
            (
                r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:qol-tools/plugins:pull""#,
                BearerChallenge {
                    realm: "https://ghcr.io/token".to_string(),
                    service: Some("ghcr.io".to_string()),
                    scope: Some("repository:qol-tools/plugins:pull".to_string()),
                },
            ),
            (
                r#"bearer realm="https://auth.example/token", scope="repository:a/b:pull,push""#,
                BearerChallenge {
                    realm: "https://auth.example/token".to_string(),
                    service: None,
                    scope: Some("repository:a/b:pull,push".to_string()),
                },
            ),
        ];
        for (header, expected) in cases {
            assert_eq!(
                &BearerChallenge::parse(header).unwrap(),
                expected,
                "{header}"
            );
        }
        assert!(BearerChallenge::parse(r#"Basic realm="x""#).is_err());
        assert!(BearerChallenge::parse(r#"Bearer service="ghcr.io""#).is_err());
    }

    #[test]
    fn token_url_carries_service_and_scope_and_refuses_plain_http_elsewhere() {
        let challenge = BearerChallenge::parse(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:qol-tools/plugins:pull""#,
        )
        .unwrap();
        let url = challenge.token_url(&registry("https://ghcr.io")).unwrap();
        assert_eq!(
            url.as_str(),
            "https://ghcr.io/token?service=ghcr.io&scope=repository%3Aqol-tools%2Fplugins%3Apull"
        );

        let local =
            BearerChallenge::parse(r#"Bearer realm="http://127.0.0.1:5000/token""#).unwrap();
        assert!(local.token_url(&registry("http://127.0.0.1:5000")).is_ok());
        assert!(local.token_url(&registry("https://ghcr.io")).is_err());
    }
}
