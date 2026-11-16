use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, CredentialUse, Credentials, CredentialsBuffers,
    DataRepresentation, Negotiate, Ntlm, SecurityBuffer, SecurityPackageType, SecurityStatus,
    ServerRequestFlags, Sspi, SspiImpl, Username,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BINDING: &[u8] = b"tls-server-end-point:controlled-channel";

fn success(operation: &str, body: &str) -> String {
    format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"{}\" xmlns:t=\"{}\"><s:Body><m:{operation}Response><m:ResponseMessages><m:{operation}ResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{body}</m:{operation}ResponseMessage></m:ResponseMessages></m:{operation}Response></s:Body></s:Envelope>",
        soap::MESSAGES,
        soap::TYPES
    )
}

/// A response carrying several already-built response messages.
fn batch(operation: &str, messages: &str) -> String {
    format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"{}\" xmlns:t=\"{}\"><s:Body><m:{operation}Response><m:ResponseMessages>{messages}</m:ResponseMessages></m:{operation}Response></s:Body></s:Envelope>",
        soap::MESSAGES,
        soap::TYPES
    )
}

fn folder_success() -> String {
    success(
        "GetFolder",
        "<m:Folders><t:Folder><t:FolderId Id=\"inbox-id\"/></t:Folder></m:Folders>",
    )
}

async fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count =
            tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert!(count > 0, "client closed before complete request");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let size = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + size {
                return (head, bytes[end + 4..end + 4 + size].to_vec());
            }
        }
    }
}

async fn reply(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
    stream.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nContent-Type: text/xml\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

fn token(head: &str) -> Vec<u8> {
    let value = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .unwrap()
        .1
        .trim();
    STANDARD
        .decode(value.strip_prefix("NTLM ").unwrap())
        .unwrap()
}

fn accept(
    ntlm: &mut Ntlm,
    credentials: &mut Option<AuthIdentityBuffers>,
    token: Vec<u8>,
) -> Result<Vec<u8>, sspi::Error> {
    let mut input = [SecurityBuffer::new(token, BufferType::Token)];
    let mut output = [SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let builder = ntlm
        .accept_security_context()
        .with_credentials_handle(credentials)
        .with_context_requirements(ServerRequestFlags::ALLOCATE_MEMORY)
        .with_target_data_representation(DataRepresentation::Native)
        .with_input(&mut input)
        .with_output(&mut output);
    let result = ntlm
        .accept_security_context_impl(builder)?
        .resolve_to_result()?;
    if result.status == SecurityStatus::CompleteNeeded
        || result.status == SecurityStatus::CompleteAndContinue
    {
        ntlm.complete_auth_token(&mut output)?;
    }
    Ok(std::mem::take(&mut output[0].buffer))
}

async fn ntlm_server(listener: TcpListener, expected_binding: &'static [u8], expect_success: bool) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let (head, _) = read_request(&mut stream).await;
    assert!(!head.to_ascii_lowercase().contains("authorization"));
    reply(
        &mut stream,
        "401 Unauthorized",
        "WWW-Authenticate: Negotiate\r\nWWW-Authenticate: NTLM\r\n",
        "",
    )
    .await;
    let mut server = Ntlm::new();
    server.set_channel_bindings(expected_binding);
    let identity = AuthIdentity {
        username: Username::parse("EXAMPLE\\alice").unwrap(),
        password: "controlled-password".to_owned().into(),
    };
    let mut credentials = server
        .acquire_credentials_handle()
        .with_credential_use(CredentialUse::Inbound)
        .with_auth_data(&identity)
        .execute(&mut server)
        .unwrap()
        .credentials_handle;
    let (head, _) = read_request(&mut stream).await;
    let type1 = token(&head);
    let type2 = accept(&mut server, &mut credentials, type1).unwrap();
    reply(
        &mut stream,
        "401 Unauthorized",
        &format!("WWW-Authenticate: NTLM {}\r\n", STANDARD.encode(type2)),
        "",
    )
    .await;
    let (head, body) = read_request(&mut stream).await;
    assert!(String::from_utf8(body).unwrap().contains("GetFolder"));
    let result = accept(&mut server, &mut credentials, token(&head));
    if expect_success {
        result.unwrap(); // SSPI validates NTLMv2 proof, MIC and channel binding.
        reply(&mut stream, "200 OK", "", &folder_success()).await;
    } else {
        assert!(
            result.is_err(),
            "a different TLS channel must not authenticate"
        );
        reply(
            &mut stream,
            "401 Unauthorized",
            "WWW-Authenticate: NTLM\r\n",
            "",
        )
        .await;
    }
}

#[tokio::test]
async fn ntlmv2_mic_and_cbt_use_one_connection_and_reject_wrong_channel() {
    for valid in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(ntlm_server(
            listener,
            if valid {
                BINDING
            } else {
                b"different TLS peer"
            },
            valid,
        ));
        let endpoint = Endpoint::parse("https://mail.example:8443/EWS/Exchange.asmx").unwrap();
        let stream = TcpStream::connect(address).await.unwrap();
        let mut http = http::Connection::with_stream(Box::new(stream), &endpoint)
            .await
            .unwrap();
        let result = http
            .authenticate(
                "EXAMPLE\\alice",
                &SecretValue::new("controlled-password".into()),
                BINDING,
            )
            .await;
        if valid {
            result.unwrap();
        } else {
            assert_eq!(result, Err(EwsError::Authentication));
        }
        server.await.unwrap();
    }
}

fn accept_negotiate(
    server: &mut Negotiate,
    credentials: &mut Option<CredentialsBuffers>,
    token: Vec<u8>,
    binding: &[u8],
) -> sspi::Result<(Vec<u8>, bool)> {
    let mut input = [
        SecurityBuffer::new(token, BufferType::Token),
        SecurityBuffer::new(
            http::channel_binding_buffer(binding).unwrap(),
            BufferType::ChannelBindings,
        ),
    ];
    let mut output = [SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let builder = server
        .accept_security_context()
        .with_credentials_handle(credentials)
        .with_context_requirements(ServerRequestFlags::empty())
        .with_target_data_representation(DataRepresentation::Native)
        .with_input(&mut input)
        .with_output(&mut output);
    let result = server
        .accept_security_context_impl(builder)?
        .resolve_to_result()?;
    assert!(matches!(
        server.query_context_negotiation_package()?.name,
        SecurityPackageType::Ntlm
    ));
    Ok((
        std::mem::take(&mut output[0].buffer),
        result.status == SecurityStatus::Ok,
    ))
}

#[tokio::test]
async fn negotiate_only_spnego_selects_ntlm_and_validates_mic_and_cbt() {
    // Some servers accept with 200 but omit the optional final SPNEGO token.
    for (valid, final_token) in [(true, true), (true, false), (false, true)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let (head, _) = read_request(&mut peer).await;
            assert!(!head.to_lowercase().contains("authorization"));
            reply(
                &mut peer,
                "401 Unauthorized",
                "WWW-Authenticate: Negotiate\r\n",
                "",
            )
            .await;
            // A UPN exercises the path where an unrestricted Negotiate context
            // would try realm/KDC lookup. Both peers explicitly allow NTLM only.
            let identity = AuthIdentity {
                username: Username::parse("alice@example.test").unwrap(),
                password: "controlled-password".to_owned().into(),
            };
            let mut negotiate =
                Negotiate::new_server(http::ntlm_only_config(), vec![identity.clone()]).unwrap();
            let credentials = Credentials::AuthIdentity(identity);
            let mut handle = negotiate
                .acquire_credentials_handle()
                .with_credential_use(CredentialUse::Inbound)
                .with_auth_data(&credentials)
                .execute(&mut negotiate)
                .unwrap()
                .credentials_handle;
            for _ in 0..6 {
                let (head, body) = read_request(&mut peer).await;
                assert!(String::from_utf8(body).unwrap().contains("GetFolder"));
                let authorization = head
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .unwrap()
                    .1
                    .trim();
                let encoded = authorization
                    .strip_prefix("Negotiate ")
                    .expect("HTTP scheme must remain Negotiate");
                let token = STANDARD.decode(encoded).unwrap();
                let result = accept_negotiate(
                    &mut negotiate,
                    &mut handle,
                    token,
                    if valid { BINDING } else { b"wrong TLS peer" },
                );
                match result {
                    Ok((output, complete)) => {
                        let headers = if complete && !final_token {
                            String::new()
                        } else {
                            format!(
                                "WWW-Authenticate: Negotiate {}\r\n",
                                STANDARD.encode(output)
                            )
                        };
                        let response_body = if complete {
                            folder_success()
                        } else {
                            String::new()
                        };
                        reply(
                            &mut peer,
                            if complete {
                                "200 OK"
                            } else {
                                "401 Unauthorized"
                            },
                            &headers,
                            &response_body,
                        )
                        .await;
                        if complete {
                            assert!(valid);
                            return;
                        }
                    }
                    Err(_) => {
                        assert!(!valid, "matching NTLM proof, MIC and CBT must succeed");
                        reply(
                            &mut peer,
                            "401 Unauthorized",
                            "WWW-Authenticate: Negotiate\r\n",
                            "",
                        )
                        .await;
                        return;
                    }
                }
            }
            panic!("bounded SPNEGO exchange did not finish");
        });
        let endpoint = Endpoint::parse("https://mail.example/EWS/Exchange.asmx").unwrap();
        let mut http = http::Connection::with_stream(
            Box::new(TcpStream::connect(address).await.unwrap()),
            &endpoint,
        )
        .await
        .unwrap();
        let result = http
            .authenticate(
                "alice@example.test",
                &SecretValue::new("controlled-password".into()),
                BINDING,
            )
            .await;
        if valid {
            result.unwrap();
        } else {
            assert_eq!(result, Err(EwsError::Authentication));
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn spnego_rejects_early_success_with_or_without_an_incomplete_token() {
    for include_token in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let (head, _) = read_request(&mut peer).await;
            assert!(!head.to_lowercase().contains("authorization"));
            reply(
                &mut peer,
                "401 Unauthorized",
                "WWW-Authenticate: Negotiate\r\n",
                "",
            )
            .await;
            let (head, _) = read_request(&mut peer).await;
            let authorization = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .unwrap()
                .1
                .trim();
            let token = STANDARD
                .decode(authorization.strip_prefix("Negotiate ").unwrap())
                .unwrap();
            let identity = AuthIdentity {
                username: Username::parse("alice@example.test").unwrap(),
                password: "controlled-password".to_owned().into(),
            };
            let mut negotiate =
                Negotiate::new_server(http::ntlm_only_config(), vec![identity.clone()]).unwrap();
            let credentials = Credentials::AuthIdentity(identity);
            let mut handle = negotiate
                .acquire_credentials_handle()
                .with_credential_use(CredentialUse::Inbound)
                .with_auth_data(&credentials)
                .execute(&mut negotiate)
                .unwrap()
                .credentials_handle;
            let (challenge, complete) =
                accept_negotiate(&mut negotiate, &mut handle, token, BINDING).unwrap();
            assert!(
                !complete,
                "only Type1 arrived; the password proof has not been sent"
            );
            let headers = if include_token {
                // Genuine SSPI-generated challenge: valid ASN.1 is not proof
                // of a completed exchange, even if a proxy labels it HTTP 200.
                format!(
                    "WWW-Authenticate: Negotiate {}\r\n",
                    STANDARD.encode(challenge)
                )
            } else {
                String::new()
            };
            reply(&mut peer, "200 OK", &headers, &folder_success()).await;
        });
        let endpoint = Endpoint::parse("https://mail.example/EWS/Exchange.asmx").unwrap();
        let mut client = http::Connection::with_stream(
            Box::new(TcpStream::connect(address).await.unwrap()),
            &endpoint,
        )
        .await
        .unwrap();
        let result = client
            .authenticate(
                "alice@example.test",
                &SecretValue::new("controlled-password".into()),
                BINDING,
            )
            .await;
        assert_eq!(result, Err(EwsError::Protocol));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn spnego_rejects_a_present_final_token_without_required_completion_mic() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        read_request(&mut peer).await;
        reply(
            &mut peer,
            "401 Unauthorized",
            "WWW-Authenticate: Negotiate\r\n",
            "",
        )
        .await;
        let identity = AuthIdentity {
            username: Username::parse("alice@example.test").unwrap(),
            password: "controlled-password".to_owned().into(),
        };
        let mut negotiate =
            Negotiate::new_server(http::ntlm_only_config(), vec![identity.clone()]).unwrap();
        let credentials = Credentials::AuthIdentity(identity);
        let mut handle = negotiate
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Inbound)
            .with_auth_data(&credentials)
            .execute(&mut negotiate)
            .unwrap()
            .credentials_handle;
        for _ in 0..6 {
            let (head, _) = read_request(&mut peer).await;
            let authorization = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .unwrap()
                .1
                .trim();
            let token = STANDARD
                .decode(authorization.strip_prefix("Negotiate ").unwrap())
                .unwrap();
            let (output, complete) =
                accept_negotiate(&mut negotiate, &mut handle, token, BINDING).unwrap();
            if complete {
                // DER fixture for SPNEGO NegTokenResp { negState: accept-completed }
                // without mechListMIC. SSPI parses it, but must keep waiting for
                // the MIC required by this context rather than report completion.
                reply(
                    &mut peer,
                    "200 OK",
                    "WWW-Authenticate: Negotiate oQcwBaADCgEA\r\n",
                    &folder_success(),
                )
                .await;
                return;
            }
            reply(
                &mut peer,
                "401 Unauthorized",
                &format!(
                    "WWW-Authenticate: Negotiate {}\r\n",
                    STANDARD.encode(output)
                ),
                "",
            )
            .await;
        }
        panic!("controlled NTLM authentication did not complete");
    });
    let endpoint = Endpoint::parse("https://mail.example/EWS/Exchange.asmx").unwrap();
    let mut client = http::Connection::with_stream(
        Box::new(TcpStream::connect(address).await.unwrap()),
        &endpoint,
    )
    .await
    .unwrap();
    let result = client
        .authenticate(
            "alice@example.test",
            &SecretValue::new("controlled-password".into()),
            BINDING,
        )
        .await;
    assert_eq!(result, Err(EwsError::Protocol));
    server.await.unwrap();
}

#[test]
fn certificate_channel_binding_hashes_the_actual_der() {
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let der = certificate.cert.der();
    let binding = http::channel_binding(der).unwrap();
    let mut expected = b"tls-server-end-point:".to_vec();
    expected.extend_from_slice(&Sha256::digest(der));
    assert_eq!(binding, expected);
}

#[test]
fn endpoint_keeps_case_and_trailing_slash_and_rejects_unsafe_urls() {
    let endpoint = Endpoint::parse("https://MAIL.Example:443/EWS/Exchange.asmx/").unwrap();
    assert_eq!(endpoint.as_str(), "https://mail.example/EWS/Exchange.asmx/");
    for value in [
        "http://mail.example/EWS",
        "https://a:b@mail.example/EWS",
        "https://mail.example/EWS?secret=x",
        "https://mail.example/EWS#x",
        "https://mail.example:0/EWS",
    ] {
        assert!(Endpoint::parse(value).is_err(), "{value}");
    }
}

fn frozen() -> (Vec<u8>, SmtpEnvelope) {
    (b"From: Sender <sender@example.org>\r\nTo: Visible <visible@example.org>\r\nCc: copy@example.org\r\nSubject: Hello\r\nMessage-ID: <frozen@example.org>\r\n\r\nFrozen body\r\n".to_vec(), SmtpEnvelope { from: "sender@example.org".into(), recipients: vec!["visible@example.org".into(), "COPY@example.org".into(), "hidden@example.org".into()] })
}

#[test]
fn ews_delivery_preserves_frozen_mime_and_uses_envelope_for_bcc() {
    let (mime, envelope) = frozen();
    let request = soap::send(&mime, &envelope).unwrap();
    let doc = soap::document(request.as_bytes()).unwrap();
    let encoded = doc
        .descendants()
        .find(|node| node.has_tag_name((soap::TYPES, "MimeContent")))
        .unwrap()
        .text()
        .unwrap();
    assert_eq!(STANDARD.decode(encoded).unwrap(), mime);
    let bcc = doc
        .descendants()
        .find(|node| node.has_tag_name((soap::TYPES, "BccRecipients")))
        .unwrap();
    let address = bcc
        .descendants()
        .find(|node| node.has_tag_name((soap::TYPES, "EmailAddress")))
        .unwrap()
        .text()
        .unwrap();
    assert_eq!(address, "hidden@example.org");
    let message = doc
        .descendants()
        .find(|node| node.has_tag_name((soap::TYPES, "Message")))
        .unwrap();
    let order: Vec<_> = message
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .collect();
    assert_eq!(
        order,
        [
            "MimeContent",
            "ToRecipients",
            "CcRecipients",
            "BccRecipients",
            "From"
        ]
    );
    assert!(
        !String::from_utf8(mime)
            .unwrap()
            .to_lowercase()
            .contains("bcc:")
    );
}

#[tokio::test]
async fn item_pagination_loads_recipient_metadata_without_fetching_bodies() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        for offset in 0..2 {
            let (_, request) = read_request(&mut peer).await;
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains(&format!("Offset=\"{offset}\"")));
            assert!(
                !request.contains("message:ToRecipients"),
                "FindItem only accepts summary properties"
            );
            assert!(!request.contains("item:Body"));
            let page = success(
                "FindItem",
                &format!(
                    "<m:RootFolder IncludesLastItemInRange=\"{}\" IndexedPagingOffset=\"1\"><t:Items><t:Message><t:ItemId Id=\"item{offset}\" ChangeKey=\"key\"/><t:Subject>Subject</t:Subject><t:IsRead>true</t:IsRead></t:Message></t:Items></m:RootFolder>",
                    if offset == 0 { "false" } else { "true" }
                ),
            );
            reply(&mut peer, "200 OK", "", &page).await;
            let (_, request) = read_request(&mut peer).await;
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains("message:ToRecipients"));
            assert!(!request.contains("MimeContent"));
            assert!(!request.contains("item:Body"));
            let metadata = success(
                "GetItem",
                &format!(
                    "<m:Items><t:Message><t:ItemId Id=\"item{offset}\"/><t:ToRecipients><t:Mailbox><t:Name>Visible</t:Name><t:EmailAddress>visible@example.org</t:EmailAddress></t:Mailbox></t:ToRecipients><t:ReplyTo><t:Mailbox><t:EmailAddress>reply@example.org</t:EmailAddress></t:Mailbox></t:ReplyTo></t:Message></m:Items>"
                ),
            );
            reply(&mut peer, "200 OK", "", &metadata).await;
        }
    });
    let mut client =
        EwsClient::test_stream(Box::new(TcpStream::connect(address).await.unwrap())).await;
    let mut first = client.items("folder", Utc::now(), 0, 50).await.unwrap();
    assert_eq!(first.next_offset, Some(1));
    assert_eq!(first.scanned, 1);
    assert!(
        first.items[0].to.is_empty(),
        "FindItem alone has no recipients"
    );
    client.recipients(&mut first.items).await.unwrap();
    assert_eq!(first.items[0].to[0].address, "visible@example.org");
    assert_eq!(first.items[0].reply_to[0].address, "reply@example.org");
    let mut last = client
        .items("folder", Utc::now(), first.next_offset.unwrap(), 50)
        .await
        .unwrap();
    client.recipients(&mut last.items).await.unwrap();
    assert_eq!(last.next_offset, None);
    assert_eq!(last.items[0].id, "item1");
    server.await.unwrap();
}

#[tokio::test]
async fn folders_walk_all_pages_and_get_roles_by_identity() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        for offset in 0..2 {
            let (_, request) = read_request(&mut peer).await;
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .contains(&format!("Offset=\"{offset}\""))
            );
            let folder = format!(
                "<t:Folder><t:FolderId Id=\"folder{offset}\"/><t:ParentFolderId Id=\"{}\"/><t:DisplayName>Localized name</t:DisplayName><t:FolderClass>IPF.Note</t:FolderClass><t:TotalCount>1</t:TotalCount><t:UnreadCount>1</t:UnreadCount></t:Folder>",
                if offset == 0 { "root" } else { "folder0" }
            );
            let page = success(
                "FindFolder",
                &format!(
                    "<m:RootFolder IncludesLastItemInRange=\"{}\" IndexedPagingOffset=\"1\"><t:Folders>{folder}</t:Folders></m:RootFolder>",
                    if offset == 0 { "false" } else { "true" }
                ),
            );
            reply(&mut peer, "200 OK", "", &page).await;
        }
        // All roles share one GetFolder; missing ones fail only their own message.
        let (_, request) = read_request(&mut peer).await;
        let request = String::from_utf8(request).unwrap();
        assert_eq!(request.matches("<t:DistinguishedFolderId").count(), 5);
        let mut messages = String::new();
        for role in ["inbox", "sentitems", "drafts", "deleteditems", "junkemail"] {
            assert!(request.contains(&format!("Id=\"{role}\"")));
            messages.push_str(&match role {
                "inbox" | "sentitems" => format!(
                    "<m:GetFolderResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Folders><t:Folder><t:FolderId Id=\"folder{}\"/></t:Folder></m:Folders></m:GetFolderResponseMessage>",
                    if role == "inbox" { 0 } else { 1 }
                ),
                "drafts" => "<m:GetFolderResponseMessage ResponseClass=\"Error\"><m:ResponseCode>ErrorMailboxStoreUnavailable</m:ResponseCode></m:GetFolderResponseMessage>".into(),
                _ => "<m:GetFolderResponseMessage ResponseClass=\"Error\"><m:ResponseCode>ErrorFolderNotFound</m:ResponseCode></m:GetFolderResponseMessage>".into(),
            });
        }
        let response = batch("GetFolder", &messages);
        reply(&mut peer, "200 OK", "", &response).await;
    });
    let mut client =
        EwsClient::test_stream(Box::new(TcpStream::connect(address).await.unwrap())).await;
    let folders = client.folders().await.unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].role, Some(EwsFolderRole::Inbox));
    assert_eq!(folders[1].role, Some(EwsFolderRole::Sent));
    assert_eq!(folders[1].parent_id.as_deref(), Some("folder0"));
    server.await.unwrap();
}

#[test]
fn folder_role_batches_reject_missing_or_mislabelled_messages() {
    let ok = "<m:GetFolderResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Folders><t:Folder><t:FolderId Id=\"inbox-id\"/></t:Folder></m:Folders></m:GetFolderResponseMessage>";
    let response = batch("GetFolder", ok);
    assert_eq!(
        soap::folder_ids(response.as_bytes(), 1).unwrap(),
        [Ok("inbox-id".to_owned())]
    );
    // Fewer answers than ids cannot be matched back to roles.
    assert_eq!(
        soap::folder_ids(response.as_bytes(), 5),
        Err(EwsError::Protocol)
    );
    let mislabelled = batch(
        "GetFolder",
        &ok.replace("GetFolderResponseMessage", "GetItemResponseMessage"),
    );
    assert_eq!(
        soap::folder_ids(mislabelled.as_bytes(), 1),
        Err(EwsError::Protocol)
    );
    // A transport-level auth refusal for one role still fails the round.
    let denied = batch(
        "GetFolder",
        "<m:GetFolderResponseMessage ResponseClass=\"Error\"><m:ResponseCode>ErrorAccessDenied</m:ResponseCode></m:GetFolderResponseMessage>",
    );
    assert_eq!(
        soap::folder_ids(denied.as_bytes(), 1).unwrap(),
        [Err(EwsError::Authentication)]
    );
}

#[tokio::test]
async fn connection_closed_before_delivery_does_not_mark_or_send() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (peer, _) = listener.accept().await.unwrap();
    drop(peer);
    let mut client = EwsClient::test_stream(Box::new(stream)).await;
    // Observe the closure through a read-only operation first. This keeps the
    // test deterministic without an arbitrary sleep before ready().
    assert!(client.verify().await.is_err());
    let marked = Arc::new(AtomicBool::new(false));
    let observed = marked.clone();
    let (mime, envelope) = frozen();
    let result = client
        .send(
            &mime,
            &envelope,
            DataStarted::new(move || async move {
                marked.store(true, Ordering::SeqCst);
                Ok(())
            }),
        )
        .await;
    assert!(matches!(
        result,
        Err(EwsError::Net(crate::net::NetError::Connect(_)))
    ));
    assert!(!observed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn failed_durable_marker_sends_no_create_item() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    let endpoint = Endpoint::parse("https://example.org/EWS/Exchange.asmx").unwrap();
    let mut client = EwsClient {
        http: http::Connection::with_stream(Box::new(stream), &endpoint)
            .await
            .unwrap(),
    };
    let (mime, envelope) = frozen();
    assert_eq!(
        client
            .send(&mime, &envelope, DataStarted::new(|| async { Err(()) }))
            .await,
        Err(EwsError::DataMarkFailed)
    );
    let mut bytes = [0u8; 1024];
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), peer.read(&mut bytes))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn delivered_request_marks_first_and_lost_response_is_unknown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let marked = Arc::new(AtomicBool::new(false));
    let observed = marked.clone();
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let (_, body) = read_request(&mut peer).await;
        assert!(observed.load(Ordering::SeqCst));
        assert!(String::from_utf8(body).unwrap().contains("CreateItem"));
        // Drop after reading the full request. Repeating would duplicate mail.
    });
    let endpoint = Endpoint::parse("https://example.org/EWS/Exchange.asmx").unwrap();
    let mut client = EwsClient {
        http: http::Connection::with_stream(
            Box::new(TcpStream::connect(address).await.unwrap()),
            &endpoint,
        )
        .await
        .unwrap(),
    };
    let (mime, envelope) = frozen();
    let result = client
        .send(
            &mime,
            &envelope,
            DataStarted::new(move || async move {
                marked.store(true, Ordering::SeqCst);
                Ok(())
            }),
        )
        .await;
    assert_eq!(result, Err(EwsError::Unknown));
    server.await.unwrap();
}

#[test]
fn soap_rejects_dtd_namespace_spoofing_and_stuck_page_cursors() {
    let xml = success("GetFolder", "");
    assert!(
        soap::check(
            xml.replace(soap::MESSAGES, "https://attacker.invalid/messages")
                .as_bytes(),
            "GetFolderResponseMessage"
        )
        .is_err()
    );
    assert!(
        soap::check(
            format!("<!DOCTYPE x [<!ENTITY x 'secret'>]>{xml}").as_bytes(),
            "GetFolderResponseMessage"
        )
        .is_err()
    );
    let page = success(
        "FindItem",
        "<m:RootFolder IncludesLastItemInRange=\"false\" IndexedPagingOffset=\"0\"><t:Items><t:Message><t:ItemId Id=\"one\"/></t:Message></t:Items></m:RootFolder>",
    );
    assert!(matches!(
        soap::parse_items(page.as_bytes(), 0),
        Err(EwsError::Protocol)
    ));
    let page = page.replace("IndexedPagingOffset=\"0\"", "IndexedPagingOffset=\"1\"");
    let result = soap::parse_items(page.as_bytes(), 0).unwrap();
    assert_eq!(result.next_offset, Some(1));
    assert_eq!(result.items[0].id, "one");
}

#[tokio::test]
async fn discovery_credentials_require_an_explicit_target_before_network() {
    let result = discovery::discover(
        &NetRoute::default(),
        "a@example.org",
        None,
        Some(("a", &SecretValue::new("secret".into()))),
    )
    .await;
    assert!(matches!(result, Err(EwsError::Protocol)));
}
