//! Provider-neutral OpenID Connect `id_token` checks. Providers that issue
//! access tokens the client must not parse take the mailbox identity from this
//! token instead.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Consumed by provider identity verifiers, which are added with the first provider"
    )
)]
use super::{OAuthError, now};
use crate::secrets::SecretValue;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

const MAX_ID_TOKEN_BYTES: usize = 16384;
/// Allowed clock difference between this machine and the token issuer.
const CLOCK_SKEW_SECS: u64 = 120;

/// Claims of an `id_token` that passed [`parse_id_token`].
#[derive(Clone, Debug)]
pub(crate) struct IdTokenClaims {
    pub issuer: String,
    pub subject: String,
    claims: serde_json::Map<String, serde_json::Value>,
}
impl IdTokenClaims {
    /// A string claim such as `email` or `preferred_username`.
    pub(crate) fn string(&self, name: &str) -> Option<&str> {
        self.claims.get(name)?.as_str()
    }
}

/// Decodes the payload of an `id_token` and checks `aud`, `exp`, `iss`, `sub`
/// and, when one was sent, `nonce`.
///
/// The signature is deliberately not verified. The token came straight from
/// the token endpoint over a TLS connection the client opened itself, and OIDC
/// Core 3.1.3.7 allows that TLS server validation to replace the signature
/// check. Never pass a token received any other way to this function.
pub(crate) fn parse_id_token(
    token: &SecretValue,
    client_id: &str,
    nonce: Option<&str>,
) -> Result<IdTokenClaims, OAuthError> {
    parse_at(token, client_id, nonce, now())
}

fn parse_at(
    token: &SecretValue,
    client_id: &str,
    nonce: Option<&str>,
    now: u64,
) -> Result<IdTokenClaims, OAuthError> {
    let token = token.expose_secret();
    if token.is_empty() || token.len() > MAX_ID_TOKEN_BYTES {
        return Err(OAuthError::InvalidResponse);
    }
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(_signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(OAuthError::InvalidResponse);
    };
    let decode = |part: &str| -> Result<serde_json::Map<String, serde_json::Value>, OAuthError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| OAuthError::InvalidResponse)?;
        match serde_json::from_slice(&bytes) {
            Ok(serde_json::Value::Object(map)) => Ok(map),
            _ => Err(OAuthError::InvalidResponse),
        }
    };
    decode(header)?;
    let claims = decode(payload)?;
    let text = |name: &str| -> Result<String, OAuthError> {
        match claims.get(name).and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() && v.len() <= 2048 && !v.chars().any(char::is_control) => {
                Ok(v.to_owned())
            }
            _ => Err(OAuthError::InvalidResponse),
        }
    };
    let audience_ok = match claims.get("aud") {
        Some(serde_json::Value::String(aud)) => aud == client_id,
        // OIDC Core 3.1.3.7: with several audiences, `azp` must name this client.
        Some(serde_json::Value::Array(list)) => {
            list.iter().any(|aud| aud.as_str() == Some(client_id))
                && (list.len() == 1
                    || claims.get("azp").and_then(|v| v.as_str()) == Some(client_id))
        }
        _ => false,
    };
    let expires = claims
        .get("exp")
        .and_then(|v| v.as_u64())
        .ok_or(OAuthError::InvalidResponse)?;
    if !audience_ok || expires.saturating_add(CLOCK_SKEW_SECS) <= now {
        return Err(OAuthError::InvalidResponse);
    }
    if let Some(expected) = nonce
        && claims.get("nonce").and_then(|v| v.as_str()) != Some(expected)
    {
        return Err(OAuthError::InvalidResponse);
    }
    Ok(IdTokenClaims {
        issuer: text("iss")?,
        subject: text("sub")?,
        claims,
    })
}

#[cfg(test)]
pub(crate) fn fixture_token(claims: serde_json::Value) -> SecretValue {
    let part = |value: serde_json::Value| URL_SAFE_NO_PAD.encode(value.to_string());
    SecretValue::new(format!(
        "{}.{}.fake-signature",
        part(serde_json::json!({"alg":"RS256","typ":"JWT"})),
        part(claims)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claims() -> serde_json::Value {
        json!({"iss":"https://login.fixture.invalid/tenant-1/v2.0","sub":"subject-1",
            "aud":"public-client","exp":2_000,"nonce":"n-1","email":"lin@example.com"})
    }
    fn check(value: serde_json::Value, nonce: Option<&str>) -> Result<IdTokenClaims, OAuthError> {
        parse_at(&fixture_token(value), "public-client", nonce, 1_000)
    }
    #[test]
    fn accepts_a_matching_token_and_returns_the_claims() {
        let parsed = check(claims(), Some("n-1")).unwrap();
        assert_eq!(parsed.issuer, "https://login.fixture.invalid/tenant-1/v2.0");
        assert_eq!(parsed.subject, "subject-1");
        assert_eq!(parsed.string("email"), Some("lin@example.com"));
        assert!(check(claims(), None).is_ok());
    }
    #[test]
    fn rejects_wrong_audience_expiry_nonce_and_missing_claims() {
        let mut other_aud = claims();
        other_aud["aud"] = json!("someone-else");
        let mut expired = claims();
        expired["exp"] = json!(800);
        let mut no_sub = claims();
        no_sub.as_object_mut().unwrap().remove("sub");
        let mut no_iss = claims();
        no_iss["iss"] = json!("");
        for (name, value, nonce) in [
            ("audience", other_aud, Some("n-1")),
            ("expired", expired, Some("n-1")),
            ("nonce", claims(), Some("n-2")),
            ("subject", no_sub, Some("n-1")),
            ("issuer", no_iss, Some("n-1")),
        ] {
            assert_eq!(
                check(value, nonce).err(),
                Some(OAuthError::InvalidResponse),
                "{name}"
            );
        }
        let mut no_nonce = claims();
        no_nonce.as_object_mut().unwrap().remove("nonce");
        assert!(check(no_nonce, Some("n-1")).is_err());
    }
    #[test]
    fn several_audiences_need_the_client_as_authorized_party() {
        let mut list = claims();
        list["aud"] = json!(["public-client", "other"]);
        assert!(check(list.clone(), Some("n-1")).is_err());
        list["azp"] = json!("public-client");
        assert!(check(list, Some("n-1")).is_ok());
    }
    #[test]
    fn rejects_malformed_tokens() {
        for token in [
            "",
            "a.b",
            "a.b.c.d",
            "!!.!!.x",
            &"x".repeat(MAX_ID_TOKEN_BYTES + 1),
        ] {
            assert_eq!(
                parse_at(
                    &SecretValue::new(token.into()),
                    "public-client",
                    None,
                    1_000
                )
                .err(),
                Some(OAuthError::InvalidResponse),
                "{token:.10}"
            );
        }
    }
}
