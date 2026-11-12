//! Exchange Web Services: the only channel a local Exchange deployment is
//! guaranteed to expose.
//!
//! Scope and staging: `docs/architecture/ews-backend.md`. This module is being
//! built bottom-up — authentication and transport first, then the operations
//! that map onto the protocol-neutral mail backend. Nothing outside the module
//! calls it yet, so unused pieces are expected until the account model lands.
#![allow(dead_code, reason = "built before the EWS account kind is wired in")]

pub(crate) mod crypto;
pub(crate) mod http;
pub(crate) mod ntlm;
pub(crate) mod soap;

use crate::net::NetRoute;
use crate::secrets::SecretValue;

/// Where one account's EWS endpoint lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Endpoint {
    /// The default endpoint on `host`. A load balancer may publish another path,
    /// which is why it is a field rather than a constant.
    pub(crate) fn new(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            port: 443,
            path: "/EWS/Exchange.asmx".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EwsError {
    Http(http::HttpError),
    Soap(soap::SoapError),
}

/// An authenticated EWS session. It owns one connection: dropping it closes the
/// NTLM context, and a new one has to authenticate from the start.
pub(crate) struct EwsClient {
    http: http::EwsHttp,
}

impl EwsClient {
    pub(crate) async fn connect(
        route: &NetRoute,
        endpoint: &Endpoint,
        user: &str,
        password: &SecretValue,
    ) -> Result<Self, EwsError> {
        let credentials = ntlm::Credentials::parse(user, password);
        let http = http::EwsHttp::open(
            route,
            &endpoint.host,
            endpoint.port,
            &endpoint.path,
            &credentials,
        )
        .await
        .map_err(EwsError::Http)?;
        Ok(Self { http })
    }

    /// The folders visible to this mailbox, deep from the message-folder root.
    pub(crate) async fn folders(&mut self) -> Result<Vec<soap::Folder>, EwsError> {
        let request = soap::find_folders();
        let response = self
            .http
            .post("FindFolder", &request)
            .await
            .map_err(EwsError::Http)?;
        soap::parse_folders(&response).map_err(EwsError::Soap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_endpoint_is_the_standard_path_on_443() {
        let endpoint = Endpoint::new("mail.example.com");
        assert_eq!(endpoint.port, 443);
        assert_eq!(endpoint.path, "/EWS/Exchange.asmx");
    }

    /// Explicit opt-in against a real mailbox. Set `MAILX_EWS_HOST`,
    /// `MAILX_EWS_USER` (`DOMAIN\user`) and `MAILX_EWS_PASSWORD`, then:
    ///
    /// ```text
    /// cargo test -p mailx-core --lib a_real_exchange -- --ignored --nocapture
    /// ```
    ///
    /// It prints folder names and counts only, never ids or credentials.
    #[tokio::test]
    #[ignore = "needs a real Exchange account in MAILX_EWS_HOST/USER/PASSWORD"]
    async fn a_real_exchange_lists_its_folders() {
        let host = std::env::var("MAILX_EWS_HOST").expect("MAILX_EWS_HOST");
        let user = std::env::var("MAILX_EWS_USER").expect("MAILX_EWS_USER");
        let password =
            SecretValue::new(std::env::var("MAILX_EWS_PASSWORD").expect("MAILX_EWS_PASSWORD"));
        let route = NetRoute::default();
        let endpoint = Endpoint::new(&host);
        let mut client = EwsClient::connect(&route, &endpoint, &user, &password)
            .await
            .expect("EWS sign-in succeeds");
        let folders = client.folders().await.expect("FindFolder succeeds");
        assert!(!folders.is_empty(), "a mailbox always has folders");
        for folder in &folders {
            println!("{} ({})", folder.display_name, folder.total_count);
        }
    }
}
