//! A single HTTP/1.1 connection owns its NTLM context. There is no pool,
//! redirect handler or request retry that could change its authentication peer.
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{HeaderMap, Request, header};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256, Sha384, Sha512};
use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, ClientRequestFlags, CredentialUse, Credentials,
    CredentialsBuffers, DataRepresentation, Negotiate, NegotiateConfig, Ntlm, SecurityBuffer,
    SecurityPackageType, SecurityStatus, Sspi, SspiImpl, Username, ntlm::NtlmConfig,
};

use super::{Endpoint, EwsError, soap};
use crate::net::{self, BoxedStream, NetRoute};
use crate::secrets::SecretValue;
use crate::smtp::DataStarted;

pub(super) const MAX_RESPONSE: usize = 48 * 1024 * 1024;
const REQUEST_BUDGET: Duration = Duration::from_secs(30);

pub(super) struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub(super) struct Connection {
    sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    task: tokio::task::JoinHandle<()>,
    authority: String,
    path: String,
    authorization_scheme: &'static str,
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Cancellation or a failed callback must close the socket promptly.
        self.task.abort();
    }
}

impl Connection {
    pub(super) async fn connect(
        route: &NetRoute,
        endpoint: &Endpoint,
        username: &str,
        password: &SecretValue,
    ) -> Result<Self, EwsError> {
        tokio::time::timeout(Duration::from_secs(60), async {
            let stream = net::connect(route, endpoint.host(), endpoint.port())
                .await
                .map_err(EwsError::Net)?;
            let (stream, certificate) =
                net::upgrade_tls_with_peer_certificate(stream, endpoint.host())
                    .await
                    .map_err(EwsError::Net)?;
            let channel_binding = channel_binding(&certificate)?;
            let mut connection = Self::with_stream(stream, endpoint).await?;
            connection
                .authenticate(username, password, &channel_binding)
                .await?;
            Ok(connection)
        })
        .await
        .map_err(|_| EwsError::Timeout)?
    }

    pub(super) async fn with_stream(
        stream: BoxedStream,
        endpoint: &Endpoint,
    ) -> Result<Self, EwsError> {
        let (sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_headers(64)
            .max_buf_size(32 * 1024)
            .handshake(TokioIo::new(stream))
            .await
            .map_err(|_| EwsError::Protocol)?;
        let task = tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self {
            sender,
            task,
            authority: endpoint.url[url::Position::BeforeHost..url::Position::AfterPort].to_owned(),
            path: endpoint.url.path().to_owned(),
            authorization_scheme: "NTLM",
        })
    }

    pub(super) async fn authenticate(
        &mut self,
        username: &str,
        password: &SecretValue,
        channel_binding: &[u8],
    ) -> Result<(), EwsError> {
        let response = self
            .authenticate_request(
                "GetFolder",
                soap::probe(),
                username,
                password,
                channel_binding,
            )
            .await?;
        soap::check(&response.body, "GetFolderResponseMessage")
    }

    pub(super) async fn authenticate_request(
        &mut self,
        action: &str,
        body: String,
        username: &str,
        password: &SecretValue,
        channel_binding: &[u8],
    ) -> Result<Reply, EwsError> {
        let response_limit = if action == "Autodiscover" {
            256 * 1024
        } else {
            MAX_RESPONSE
        };
        let response = self
            .exchange(action, body.clone(), None, response_limit)
            .await?;
        if response.status != 401 {
            return Err(if response.status == 403 {
                EwsError::Authentication
            } else {
                EwsError::AuthUnsupported
            });
        }
        let identity = AuthIdentity {
            username: Username::parse(username).map_err(|_| EwsError::Authentication)?,
            password: password.expose_secret().to_owned().into(),
        };
        if !offers(&response.headers, "NTLM") && offers(&response.headers, "Negotiate") {
            return self
                .authenticate_spnego(action, body, identity, channel_binding, response_limit)
                .await;
        }
        if challenge(&response.headers, "NTLM")?.is_some() {
            return Err(EwsError::Protocol);
        }
        // Explicitly omit the machine name. Only caller-supplied credentials
        // are available; SSPI's network clients and credential discovery are off.
        let mut ntlm = Ntlm::with_config(NtlmConfig::new(String::new()));
        ntlm.set_channel_bindings(channel_binding);
        let mut credentials = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .map_err(|_| EwsError::Authentication)?
            .credentials_handle;
        let type1 = ntlm_step(&mut ntlm, &mut credentials, None, &self.authority)?;
        let response = self
            .exchange(action, body.clone(), Some(&type1), response_limit)
            .await?;
        if response.status != 401 {
            return Err(EwsError::Authentication);
        }
        let type2 = challenge(&response.headers, "NTLM")?.ok_or(EwsError::Protocol)?;
        // SSPI always emits NTLMv2; refuse a peer that does not offer its
        // required extended-session-security negotiation rather than downgrade.
        if type2.len() < 48 || &type2[..8] != b"NTLMSSP\0" || type2[8..12] != 2u32.to_le_bytes() {
            return Err(EwsError::Protocol);
        }
        let flags = u32::from_le_bytes(type2[20..24].try_into().map_err(|_| EwsError::Protocol)?);
        if flags & 0x0008_0200 != 0x0008_0200 {
            return Err(EwsError::AuthUnsupported);
        }
        let type3 = ntlm_step(&mut ntlm, &mut credentials, Some(type2), &self.authority)?;
        let response = self
            .exchange(action, body, Some(&type3), response_limit)
            .await?;
        status(response.status)?;
        Ok(response)
    }

    async fn authenticate_spnego(
        &mut self,
        action: &str,
        body: String,
        identity: AuthIdentity,
        channel_binding: &[u8],
        response_limit: usize,
    ) -> Result<Reply, EwsError> {
        // The package allow-list is closed. Even a UPN or a peer selecting a
        // Kerberos OID cannot trigger KDC discovery, OS tickets or a fallback.
        let mut negotiate =
            Negotiate::new_client(ntlm_only_config()).map_err(|_| EwsError::AuthUnsupported)?;
        let credentials = Credentials::AuthIdentity(identity);
        let mut handle = negotiate
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&credentials)
            .execute(&mut negotiate)
            .map_err(|_| EwsError::Authentication)?
            .credentials_handle;
        self.authorization_scheme = "Negotiate";
        let (mut output, mut complete) = negotiate_step(
            &mut negotiate,
            &mut handle,
            None,
            channel_binding,
            &self.authority,
        )?;
        // SPNEGO may require an additional mechListMIC exchange after NTLM's
        // Type 3. Every round carries only this read-only authentication probe.
        for _ in 0..6 {
            if output.is_empty() {
                return Err(EwsError::Protocol);
            }
            let response = self
                .exchange(action, body.clone(), Some(&output), response_limit)
                .await?;
            if (200..300).contains(&response.status) && complete {
                return Ok(response);
            }
            if response.status != 401 && !(200..300).contains(&response.status) {
                return Err(if response.status == 403 {
                    EwsError::Authentication
                } else {
                    EwsError::Protocol
                });
            }
            let input =
                challenge(&response.headers, "Negotiate")?.ok_or(EwsError::Authentication)?;
            (output, complete) = negotiate_step(
                &mut negotiate,
                &mut handle,
                Some(input),
                channel_binding,
                &self.authority,
            )?;
            if (200..300).contains(&response.status) && complete && output.is_empty() {
                return Ok(response);
            }
        }
        Err(EwsError::Authentication)
    }

    pub(super) async fn post(&mut self, action: &str, body: String) -> Result<Vec<u8>, EwsError> {
        let reply = self.exchange(action, body, None, MAX_RESPONSE).await?;
        status(reply.status)?;
        Ok(reply.body)
    }

    pub(super) async fn post_delivery(
        &mut self,
        body: String,
        before_delivery: DataStarted,
    ) -> Result<Vec<u8>, EwsError> {
        tokio::time::timeout(REQUEST_BUDGET, self.sender.ready())
            .await
            .map_err(|_| EwsError::Timeout)?
            .map_err(|_| {
                EwsError::Net(net::NetError::Connect(
                    "connection closed before delivery".into(),
                ))
            })?;
        let request = self.request("CreateItem", body, None)?;
        before_delivery
            .run()
            .await
            .map_err(|_| EwsError::DataMarkFailed)?;
        // Once handed to hyper, transport errors/timeout/redirects are ambiguous.
        // No retry can be introduced here without risking duplicate delivery.
        let reply = tokio::time::timeout(REQUEST_BUDGET, async {
            let response = self
                .sender
                .send_request(request)
                .await
                .map_err(|_| EwsError::Unknown)?;
            collect(response, MAX_RESPONSE).await
        })
        .await
        .map_err(|_| EwsError::Unknown)?
        .map_err(|_| EwsError::Unknown)?;
        match reply.status {
            200..=299 => Ok(reply.body),
            401 | 403 => Err(EwsError::Authentication),
            // Even a 500 may follow execution (e.g. a proxy error). Parse only
            // a complete, explicit EWS refusal; anything else stays unknown.
            _ => match soap::check(&reply.body, "CreateItemResponseMessage") {
                Err(EwsError::Server { code }) => Err(EwsError::Server { code }),
                _ => Err(EwsError::Unknown),
            },
        }
    }

    pub(super) async fn exchange(
        &mut self,
        action: &str,
        body: String,
        token: Option<&[u8]>,
        limit: usize,
    ) -> Result<Reply, EwsError> {
        let request = self.request(action, body, token)?;
        tokio::time::timeout(REQUEST_BUDGET, async {
            self.sender.ready().await.map_err(|_| EwsError::Protocol)?;
            let response = self
                .sender
                .send_request(request)
                .await
                .map_err(|_| EwsError::Protocol)?;
            collect(response, limit).await
        })
        .await
        .map_err(|_| EwsError::Timeout)?
    }

    fn request(
        &self,
        action: &str,
        body: String,
        token: Option<&[u8]>,
    ) -> Result<Request<Full<Bytes>>, EwsError> {
        let mut request = Request::builder()
            .method("POST")
            .uri(&self.path)
            .header(header::HOST, &self.authority)
            .header(header::CONTENT_TYPE, "text/xml; charset=utf-8");
        if action != "Autodiscover" {
            request = request.header("SOAPAction", format!("\"{}/{action}\"", soap::MESSAGES));
        }
        if let Some(token) = token {
            let mut authorization = hyper::header::HeaderValue::from_str(&format!(
                "{} {}",
                self.authorization_scheme,
                STANDARD.encode(token)
            ))
            .map_err(|_| EwsError::Protocol)?;
            authorization.set_sensitive(true);
            request = request.header(header::AUTHORIZATION, authorization);
        }
        request
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| EwsError::Protocol)
    }
}

fn ntlm_step(
    ntlm: &mut Ntlm,
    credentials: &mut Option<AuthIdentityBuffers>,
    token: Option<Vec<u8>>,
    authority: &str,
) -> Result<Vec<u8>, EwsError> {
    let mut output = [SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let mut input = token.map(|token| [SecurityBuffer::new(token, BufferType::Token)]);
    let target = format!("HTTP/{authority}");
    let builder = ntlm
        .initialize_security_context()
        .with_credentials_handle(credentials)
        .with_context_requirements(ClientRequestFlags::ALLOCATE_MEMORY)
        .with_target_data_representation(DataRepresentation::Native)
        .with_target_name(&target)
        .with_output(&mut output);
    let result = if let Some(input) = &mut input {
        let mut builder = builder.with_input(input);
        ntlm.initialize_security_context_impl(&mut builder)
            .and_then(|mut result| result.resolve_to_result())
    } else {
        let mut builder = builder;
        ntlm.initialize_security_context_impl(&mut builder)
            .and_then(|mut result| result.resolve_to_result())
    };
    result.map_err(|_| EwsError::Authentication)?;
    Ok(std::mem::take(&mut output[0].buffer))
}

pub(super) async fn collect(
    response: hyper::Response<hyper::body::Incoming>,
    limit: usize,
) -> Result<Reply, EwsError> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| EwsError::Protocol)?;
        if let Ok(data) = frame.into_data() {
            if data.len() > limit.saturating_sub(bytes.len()) {
                return Err(EwsError::TooLarge);
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(Reply {
        status,
        headers,
        body: bytes,
    })
}

fn status(code: u16) -> Result<(), EwsError> {
    match code {
        200..=299 => Ok(()),
        401 | 403 => Err(EwsError::Authentication),
        404 => Err(EwsError::NotFound),
        _ => Err(EwsError::Protocol),
    }
}

fn offers(headers: &HeaderMap, scheme: &str) -> bool {
    headers
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| {
            value
                .split_whitespace()
                .next()
                .is_some_and(|value| value.eq_ignore_ascii_case(scheme))
        })
}

fn challenge(headers: &HeaderMap, scheme: &str) -> Result<Option<Vec<u8>>, EwsError> {
    let mut offered = false;
    for value in headers.get_all(header::WWW_AUTHENTICATE) {
        let text = value.to_str().map_err(|_| EwsError::Protocol)?;
        for part in text.split(',') {
            let mut words = part.split_whitespace();
            if !words
                .next()
                .is_some_and(|value| value.eq_ignore_ascii_case(scheme))
            {
                continue;
            }
            offered = true;
            if let Some(encoded) = words.next() {
                if encoded.len() > 16 * 1024 || words.next().is_some() {
                    return Err(EwsError::Protocol);
                }
                return STANDARD
                    .decode(encoded)
                    .map(Some)
                    .map_err(|_| EwsError::Protocol);
            }
        }
    }
    if offered {
        Ok(None)
    } else {
        Err(EwsError::AuthUnsupported)
    }
}

pub(super) fn ntlm_only_config() -> NegotiateConfig {
    NegotiateConfig::new(
        Box::new(NtlmConfig::new(String::new())),
        Some("ntlm,!kerberos,!pku2u".into()),
        String::new(),
    )
}

/// SEC_CHANNEL_BINDINGS with empty initiator/acceptor addresses. ASN.1/SPNEGO
/// and its MIC tokens are entirely constructed and parsed by SSPI.
pub(super) fn channel_binding_buffer(binding: &[u8]) -> Result<Vec<u8>, EwsError> {
    let length = u32::try_from(binding.len()).map_err(|_| EwsError::TooLarge)?;
    let mut bytes = vec![0; 32];
    bytes[24..28].copy_from_slice(&length.to_le_bytes());
    bytes[28..32].copy_from_slice(&32u32.to_le_bytes());
    bytes.extend_from_slice(binding);
    Ok(bytes)
}

fn negotiate_step(
    negotiate: &mut Negotiate,
    credentials: &mut Option<CredentialsBuffers>,
    token: Option<Vec<u8>>,
    binding: &[u8],
    authority: &str,
) -> Result<(Vec<u8>, bool), EwsError> {
    let mut input = [
        SecurityBuffer::new(token.unwrap_or_default(), BufferType::Token),
        SecurityBuffer::new(
            channel_binding_buffer(binding)?,
            BufferType::ChannelBindings,
        ),
    ];
    let mut output = [SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let target = format!("HTTP/{authority}");
    let mut builder = negotiate
        .initialize_security_context()
        .with_credentials_handle(credentials)
        .with_context_requirements(
            ClientRequestFlags::ALLOCATE_MEMORY
                | ClientRequestFlags::INTEGRITY
                | ClientRequestFlags::USE_SESSION_KEY,
        )
        .with_target_data_representation(DataRepresentation::Native)
        .with_target_name(&target)
        .with_input(&mut input)
        .with_output(&mut output);
    let result = negotiate
        .initialize_security_context_impl(&mut builder)
        .and_then(|mut generator| generator.resolve_to_result())
        .map_err(|_| EwsError::AuthUnsupported)?;
    if !matches!(
        negotiate
            .query_context_negotiation_package()
            .map_err(|_| EwsError::AuthUnsupported)?
            .name,
        SecurityPackageType::Ntlm
    ) {
        return Err(EwsError::AuthUnsupported);
    }
    let complete = match result.status {
        SecurityStatus::Ok => true,
        SecurityStatus::ContinueNeeded => false,
        _ => return Err(EwsError::Protocol),
    };
    Ok((std::mem::take(&mut output[0].buffer), complete))
}

/// RFC 5929 tls-server-end-point. MD5/SHA1 certificate signatures use SHA256;
/// RSA-PSS parameters select its hash. Unknown algorithms fail closed.
pub(super) fn channel_binding(der: &[u8]) -> Result<Vec<u8>, EwsError> {
    let (rest, certificate) =
        x509_parser::parse_x509_certificate(der).map_err(|_| EwsError::Protocol)?;
    if !rest.is_empty() {
        return Err(EwsError::Protocol);
    }
    let signature = &certificate.signature_algorithm;
    let oid = signature.algorithm.to_id_string();
    let digest_oid = if oid == "1.2.840.113549.1.1.10" {
        x509_parser::signature_algorithm::RsaSsaPssParams::try_from(
            signature.parameters.as_ref().ok_or(EwsError::Protocol)?,
        )
        .map_err(|_| EwsError::Protocol)?
        .hash_algorithm_oid()
        .to_id_string()
    } else {
        oid
    };
    let hash = match digest_oid.as_str() {
        "1.2.840.113549.1.1.4"
        | "1.2.840.113549.1.1.5"
        | "1.2.840.10045.4.1"
        | "1.3.14.3.2.26"
        | "2.16.840.1.101.3.4.2.1"
        | "1.2.840.113549.1.1.11"
        | "1.2.840.10045.4.3.2" => Sha256::digest(der).to_vec(),
        "2.16.840.1.101.3.4.2.2" | "1.2.840.113549.1.1.12" | "1.2.840.10045.4.3.3" => {
            Sha384::digest(der).to_vec()
        }
        "2.16.840.1.101.3.4.2.3" | "1.2.840.113549.1.1.13" | "1.2.840.10045.4.3.4" => {
            Sha512::digest(der).to_vec()
        }
        _ => return Err(EwsError::AuthUnsupported),
    };
    let mut binding = b"tls-server-end-point:".to_vec();
    binding.extend_from_slice(&hash);
    Ok(binding)
}
