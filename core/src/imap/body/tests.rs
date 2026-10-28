use super::*;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

async fn scripted_fetch(response: Vec<u8>, limits: Limits) -> Result<Option<Vec<u8>>, ImapError> {
    let (mut client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut command = String::new();
        server.read_line(&mut command).await.unwrap();
        assert_eq!(command, "B1 UID FETCH 42 (UID BODY.PEEK[])\r\n");
        // Early rejection deliberately closes the receiving side.
        let _ = server.get_mut().write_all(&response).await;
    });
    let result = fetch(&mut client, "B1", 42, limits).await;
    drop(client);
    server.await.unwrap();
    result
}

fn response(body: &[u8], completion: &[u8]) -> Vec<u8> {
    let mut response = format!("* 7 FETCH (UID 42 BODY[] {{{}}}\r\n", body.len()).into_bytes();
    response.extend_from_slice(body);
    response.extend_from_slice(b")\r\n");
    response.extend_from_slice(completion);
    response
}

#[tokio::test]
async fn complete_message_is_binary_safe_and_uses_peek() {
    // CRLF, delimiters and fake tags inside a literal are not protocol syntax.
    let raw = b"Subject: test\r\n\r\n\0\xff)\r\nB1 OK fake\r\n{999999999999}\r\n";
    let fetched = scripted_fetch(response(raw, b"B1 OK done\r\n"), Limits::default())
        .await
        .unwrap();
    assert_eq!(fetched.as_deref(), Some(raw.as_slice()));
}

#[tokio::test]
async fn body_before_uid_is_supported_and_zero_length_is_not_missing() {
    let fetched = scripted_fetch(
        b"* 7 FETCH (BODY[] {0}\r\n UID 42)\r\nB1 OK\r\n".to_vec(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, Some(Vec::new()));
}

#[tokio::test]
async fn quoted_bodies_decode_imap_escapes() {
    let fetched = scripted_fetch(
        b"* 7 FETCH (UID 42 BODY[] \"a\\\"b\\\\c\")\r\nB1 OK\r\n".to_vec(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(fetched.as_deref(), Some(b"a\"b\\c".as_slice()));
    assert_eq!(
        scripted_fetch(
            b"* 7 FETCH (UID 42 BODY[] \"\")\r\nB1 OK\r\n".to_vec(),
            Limits::default(),
        )
        .await
        .unwrap(),
        Some(Vec::new())
    );
}

#[tokio::test]
async fn only_a_complete_tagged_ok_can_report_missing() {
    assert_eq!(
        scripted_fetch(b"* 0 EXISTS\r\nB1 OK\r\n".to_vec(), Limits::default())
            .await
            .unwrap(),
        None
    );
    for reply in [
        "",
        "B1 NO private-marker\r\n",
        "B1 BAD private-marker\r\n",
        "B2 OK\r\n",
        "* BYE private-marker\r\nB1 OK\r\n",
        "+ private-marker\r\n",
    ] {
        let error = scripted_fetch(reply.as_bytes().to_vec(), Limits::default())
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("private-marker"));
    }
}

#[tokio::test]
async fn partial_body_never_succeeds_without_matching_completion() {
    for completion in [
        b"".as_slice(),
        b"B1 NO private-marker\r\n",
        b"B1 BAD private-marker\r\n",
        b"B2 OK\r\n",
    ] {
        assert!(
            scripted_fetch(response(b"complete body", completion), Limits::default())
                .await
                .is_err()
        );
    }
    for reply in [
        "* 7 FETCH (UID 42 BODY[] {100}\r\ntruncated",
        "* 7 FETCH (UID 42 BODY[] {3}\r\nabc)",
        "* 7 FETCH (UID 42 BODY[] {3}\r\nabc)\nB1 OK\r\n",
    ] {
        assert!(
            scripted_fetch(reply.as_bytes().to_vec(), Limits::default())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn rejects_wrong_missing_duplicate_uids_and_incomplete_sections() {
    for attributes in [
        "UID 41 BODY[] {3}\r\nabc",
        "BODY[] {3}\r\nabc",
        "UID 42 UID 42 BODY[] {3}\r\nabc",
        "UID 42 UID 43 BODY[] {3}\r\nabc",
        "UID 42 BODY[TEXT] {3}\r\nabc",
        "UID 42 BODY[]<0> {3}\r\nabc",
        "UID 42 BODY[] NIL",
        "UID 42 FLAGS (\\Seen)",
        "UID 42 BODY[] {3}\r\nabc BODY[] {3}\r\ndef",
        "UID 42 RFC822 {3}\r\nabc",
    ] {
        let reply = format!("* 7 FETCH ({attributes})\r\nB1 OK\r\n");
        assert!(
            scripted_fetch(reply.into_bytes(), Limits::default())
                .await
                .is_err(),
            "accepted invalid response attributes"
        );
    }
}

#[tokio::test]
async fn rejects_repeated_and_multiple_message_responses() {
    for second in [
        "* 7 FETCH (UID 42 BODY[] {3}\r\nabc)\r\n",
        "* 7 FETCH (UID 42 BODY[] {3}\r\ndef)\r\n",
        "* 8 FETCH (UID 43 BODY[] {3}\r\nabc)\r\n",
    ] {
        let mut reply = response(b"abc", b"");
        reply.extend_from_slice(second.as_bytes());
        reply.extend_from_slice(b"B1 OK\r\n");
        assert!(scripted_fetch(reply, Limits::default()).await.is_err());
    }
}

#[tokio::test]
async fn rejects_huge_literal_declarations_without_reading_the_payload() {
    for declaration in [
        (MAX_MESSAGE_BYTES + 1).to_string(),
        "536870912".into(),
        "99999999999999999999999999999999999".into(),
    ] {
        let (mut client, server) = tokio::io::duplex(1024);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let mut command = String::new();
            server.read_line(&mut command).await.unwrap();
            server
                .get_mut()
                .write_all(format!("* 7 FETCH (UID 42 BODY[] {{{declaration}}}\r\n").as_bytes())
                .await
                .unwrap();
            // No payload follows and the peer remains open. Waiting to allocate
            // or read the declared body would cause the test timeout to fail.
            std::future::pending::<()>().await;
        });
        assert_eq!(
            timeout(
                Duration::from_secs(1),
                fetch(&mut client, "B1", 42, Limits::default())
            )
            .await,
            Ok(Err(ImapError::TooLarge))
        );
        server.abort();
    }
}

#[tokio::test]
async fn actual_payload_is_bounded_independently_of_advertised_size() {
    let limits = Limits {
        payload: 7,
        ..Limits::default()
    };
    assert_eq!(
        scripted_fetch(response(b"12345678", b"B1 OK\r\n"), limits).await,
        Err(ImapError::TooLarge)
    );
    let fetched = scripted_fetch(response(b"1234567", b"B1 OK\r\n"), limits)
        .await
        .unwrap();
    assert_eq!(fetched.unwrap(), b"1234567");
}

#[tokio::test]
async fn cumulative_literals_share_a_single_payload_budget() {
    let limits = Limits {
        payload: 7,
        ..Limits::default()
    };
    let reply = b"* 7 FETCH (UID 42 BODY[] {4}\r\nabcd X-EXTRA {4}\r\nefgh)\r\nB1 OK\r\n";
    assert_eq!(
        scripted_fetch(reply.to_vec(), limits).await,
        Err(ImapError::TooLarge)
    );
}

#[tokio::test]
async fn protocol_lines_total_metadata_and_nesting_are_bounded() {
    assert!(
        scripted_fetch(
            b"* OK a very long response line\r\nB1 OK\r\n".to_vec(),
            Limits {
                line: 20,
                ..Limits::default()
            }
        )
        .await
        .is_err()
    );
    assert!(
        scripted_fetch(
            b"* 1 EXISTS\r\n* 1 EXISTS\r\n* 1 EXISTS\r\nB1 OK\r\n".to_vec(),
            Limits {
                control: 24,
                ..Limits::default()
            }
        )
        .await
        .is_err()
    );
    let reply = format!("* 7 FETCH (UID 42 BODYSTRUCTURE {}\r\n", "(".repeat(33));
    assert!(
        scripted_fetch(reply.into_bytes(), Limits::default())
            .await
            .is_err()
    );
}

#[test]
fn literals_inside_quotes_are_not_interpreted_as_lengths() {
    let mut nesting = 0;
    assert_eq!(
        literal_length(
            b"* 1 FETCH (UID 42 BODY[] \"text{9999}\")\r\n",
            &mut nesting
        )
        .unwrap(),
        None
    );
    assert_eq!(nesting, 0);
    assert_eq!(
        literal_length(b"B1 OK note {9999}\r\n", &mut nesting).unwrap(),
        None
    );
}

#[test]
fn timeout_budget_grows_from_thirty_seconds_to_ten_minutes() {
    assert_eq!(COMMAND_TIMEOUT, Duration::from_secs(30));
    assert_eq!(body_timeout(Some(0)), COMMAND_TIMEOUT);
    assert_eq!(body_timeout(Some(30 * 64 * 1024)), COMMAND_TIMEOUT);
    assert_eq!(
        body_timeout(Some(30 * 64 * 1024 + 1)),
        Duration::from_secs(31)
    );
    assert_eq!(
        body_timeout(Some(120 * 64 * 1024)),
        Duration::from_secs(120)
    );
    assert_eq!(body_timeout(Some(100 * 1024 * 1024)), MAX_BODY_TIMEOUT);
    assert_eq!(body_timeout(Some(u64::MAX)), MAX_BODY_TIMEOUT);
    assert_eq!(body_timeout(None), MAX_BODY_TIMEOUT);
}

fn login() -> ImapLogin {
    ImapLogin {
        host: "imap.example.test".into(),
        port: 993,
        tls: TlsMode::Implicit,
        username: "fixture@example.test".into(),
        password: crate::secrets::SecretValue::new("synthetic-password".into()),
    }
}

async fn command(reader: &mut BufReader<tokio::io::DuplexStream>, expected: &str) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let (tag, actual) = line.trim_end().split_once(' ').unwrap();
    assert!(actual.starts_with(expected));
    tag.to_owned()
}

async fn authenticated_peer(reader: &mut BufReader<tokio::io::DuplexStream>) {
    reader.get_mut().write_all(b"* OK ready\r\n").await.unwrap();
    let tag = command(reader, "LOGIN").await;
    reader
        .get_mut()
        .write_all(format!("{tag} OK\r\n").as_bytes())
        .await
        .unwrap();
    let tag = command(reader, "CAPABILITY").await;
    reader
        .get_mut()
        .write_all(format!("* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n").as_bytes())
        .await
        .unwrap();
}

#[tokio::test]
async fn handoff_preserves_unsolicited_bytes_queued_after_examine() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        let tag = command(&mut server, "EXAMINE \"INBOX\"").await;
        server
            .get_mut()
            .write_all(
                format!("* OK [UIDVALIDITY 123]\r\n{tag} OK [READ-ONLY]\r\n* 3 EXISTS\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let tag = command(&mut server, "UID FETCH 42 (UID BODY.PEEK[])").await;
        server
            .get_mut()
            .write_all(&response(b"fixture", format!("{tag} OK\r\n").as_bytes()))
            .await
            .unwrap();
    });
    let mut session = test_body_session(Box::new(client), &login()).await.unwrap();
    assert_eq!(session.examine("INBOX").await.unwrap().uid_validity, 123);
    assert_eq!(
        session.fetch_message(42, Some(7)).await.unwrap(),
        Some(b"fixture".to_vec())
    );
    server.await.unwrap();
}

#[tokio::test]
async fn malicious_literal_after_examine_is_not_lost_in_library_read_ahead() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        let tag = command(&mut server, "EXAMINE").await;
        server
            .get_mut()
            .write_all(
                format!("* OK [UIDVALIDITY 123]\r\n{tag} OK [READ-ONLY]\r\n* 7 FETCH (UID 42 BODY[] {{536870912}}\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        command(&mut server, "UID FETCH").await;
        std::future::pending::<()>().await;
    });
    let mut session = test_body_session(Box::new(client), &login()).await.unwrap();
    session.examine("INBOX").await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), session.fetch_message(42, None)).await,
        Ok(Err(ImapError::TooLarge))
    );
    server.abort();
}

#[tokio::test]
async fn malicious_literal_during_authentication_is_rejected_before_allocation() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        server.get_mut().write_all(b"* OK ready\r\n").await.unwrap();
        command(&mut server, "LOGIN").await;
        server
            .get_mut()
            .write_all(b"* 7 FETCH (UID 42 BODY[] {536870912}\r\n")
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let result = timeout(
        Duration::from_secs(1),
        test_body_session(Box::new(client), &login()),
    )
    .await;
    assert!(matches!(result, Ok(Err(ImapError::TooLarge))));
    server.abort();
}

#[tokio::test]
async fn malicious_literal_during_examine_retains_the_size_failure_code() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        command(&mut server, "EXAMINE").await;
        server
            .get_mut()
            .write_all(b"* 7 FETCH (UID 42 BODY[] {536870912}\r\n")
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let mut session = test_body_session(Box::new(client), &login()).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), session.examine("INBOX")).await,
        Ok(Err(ImapError::TooLarge))
    );
    server.abort();
}

#[tokio::test]
async fn known_oversize_message_sends_no_fetch() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        let mut remaining = Vec::new();
        server.read_to_end(&mut remaining).await.unwrap();
        assert!(remaining.is_empty());
    });
    let session = test_body_session(Box::new(client), &login()).await.unwrap();
    assert_eq!(
        session
            .fetch_message(42, Some(MAX_MESSAGE_BYTES as u64 + 1))
            .await,
        Err(ImapError::TooLarge)
    );
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn fetch_timeout_uses_the_extended_size_budget() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        let tag = command(&mut server, "UID FETCH").await;
        tokio::time::sleep(Duration::from_secs(31)).await;
        server
            .get_mut()
            .write_all(&response(b"fixture", format!("{tag} OK\r\n").as_bytes()))
            .await
            .unwrap();
    });
    let session = test_body_session(Box::new(client), &login()).await.unwrap();
    assert_eq!(
        session
            .fetch_message(42, Some(60 * 64 * 1024))
            .await
            .unwrap(),
        Some(b"fixture".to_vec())
    );
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn unknown_size_has_a_ten_minute_hard_timeout() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        command(&mut server, "UID FETCH").await;
        std::future::pending::<()>().await;
    });
    let session = test_body_session(Box::new(client), &login()).await.unwrap();
    let started = tokio::time::Instant::now();
    assert_eq!(
        session.fetch_message(42, None).await,
        Err(ImapError::Timeout)
    );
    assert_eq!(started.elapsed(), MAX_BODY_TIMEOUT);
    server.abort();
}

#[tokio::test]
async fn body_connection_honors_proxy_route_without_direct_fallback() {
    use crate::accounts::types::ProxyKind;
    use crate::net::{NetError, ProxyRoute};
    use tokio::net::TcpListener;

    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let route = NetRoute {
        image_proxy_dns: false,
        proxy: Some(ProxyRoute {
            kind: ProxyKind::HttpConnect,
            host: "127.0.0.1".into(),
            port: proxy.local_addr().unwrap().port(),
            auth: None,
        }),
    };
    let mut login = login();
    login.host = "127.0.0.1".into();
    login.port = target.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = proxy.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
            assert!(request.len() < 4096);
        }
        socket
            .write_all(b"HTTP/1.1 403 private-server-marker\r\n\r\n")
            .await
            .unwrap();
    });
    let error = match open_body_session(&route, &login).await {
        Ok(_) => panic!("a rejected body route must not connect"),
        Err(error) => error,
    };
    assert!(matches!(error, ImapError::Net(NetError::Proxy(_))));
    assert!(!error.to_string().contains("private-server-marker"));
    server.await.unwrap();
    assert!(matches!(
        target.into_std().unwrap().accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[tokio::test(start_paused = true)]
async fn examine_keeps_its_thirty_second_command_budget() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        command(&mut server, "EXAMINE").await;
        std::future::pending::<()>().await;
    });
    let mut session = test_body_session(Box::new(client), &login()).await.unwrap();
    let started = tokio::time::Instant::now();
    assert_eq!(session.examine("INBOX").await, Err(ImapError::Timeout));
    assert_eq!(started.elapsed(), COMMAND_TIMEOUT);
    server.abort();
}

#[tokio::test]
async fn short_command_tags_interoperate_with_servers_that_limit_tag_echoes() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        let tag = command(&mut server, "EXAMINE").await;
        server
            .get_mut()
            .write_all(format!("* OK [UIDVALIDITY 123]\r\n{tag} OK [READ-ONLY]\r\n").as_bytes())
            .await
            .unwrap();
        let tag = command(&mut server, "UID FETCH 42 (UID BODY.PEEK[])").await;
        // A real provider returned a 15-byte completion tag for the previous
        // UUID-length tag. Keep exact completion validation; send a short tag.
        let echoed: String = tag.chars().take(15).collect();
        server
            .get_mut()
            .write_all(&response(b"fixture", format!("{echoed} OK\r\n").as_bytes()))
            .await
            .unwrap();
    });
    let mut session = test_body_session(Box::new(client), &login()).await.unwrap();
    session.examine("INBOX").await.unwrap();
    assert_eq!(
        session.fetch_message(42, Some(7)).await.unwrap(),
        Some(b"fixture".to_vec())
    );
    server.await.unwrap();
}

#[tokio::test]
async fn body_guard_preserves_provider_policy_at_every_negative_response_phase() {
    for failed in ["LOGIN", "CAPABILITY", "ID", "EXAMINE", "UID FETCH"] {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let peer = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            server.get_mut().write_all(b"* OK ready\r\n").await.unwrap();
            for (expected, success) in [
                ("LOGIN", "{tag} OK\r\n"),
                ("CAPABILITY", "* CAPABILITY IMAP4rev1 ID\r\n{tag} OK\r\n"),
                ("ID", "* ID NIL\r\n{tag} OK\r\n"),
                (
                    "EXAMINE",
                    "* 1 EXISTS\r\n* OK [UIDVALIDITY 7] x\r\n{tag} OK\r\n",
                ),
                ("UID FETCH", ""),
            ] {
                let tag = command(&mut server, expected).await;
                let reply = if expected == failed {
                    "{tag} NO Unsafe Login. Please contact kefu@188.com for help private-marker\r\n"
                } else {
                    success
                };
                server
                    .get_mut()
                    .write_all(reply.replace("{tag}", &tag).as_bytes())
                    .await
                    .unwrap();
                if expected == failed {
                    return;
                }
            }
        });
        let result = async {
            let mut session = test_body_session(Box::new(client), &login()).await?;
            session.examine("INBOX").await?;
            session.fetch_message(42, None).await
        }
        .await;
        assert_eq!(result, Err(ImapError::ProviderPolicy), "{failed}");
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn policy_text_inside_message_literals_is_ordinary_mail_content() {
    let raw = b"Subject: Unsafe Login. Please contact kefu@188.com for help\r\n\r\n* NO Unsafe Login. Please contact kefu@188.com for help\r\n";
    let (client, server) = tokio::io::duplex(16 * 1024);
    let peer = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        authenticated_peer(&mut server).await;
        let tag = command(&mut server, "EXAMINE").await;
        server
            .get_mut()
            .write_all(format!("* 1 EXISTS\r\n* OK [UIDVALIDITY 7] x\r\n{tag} OK\r\n").as_bytes())
            .await
            .unwrap();
        let tag = command(&mut server, "UID FETCH").await;
        let mut reply = format!("* 1 FETCH (UID 42 BODY[] {{{}}}\r\n", raw.len()).into_bytes();
        reply.extend_from_slice(raw);
        reply.extend_from_slice(format!(")\r\n{tag} OK\r\n").as_bytes());
        server.get_mut().write_all(&reply).await.unwrap();
    });
    let mut session = test_body_session(Box::new(client), &login()).await.unwrap();
    session.examine("INBOX").await.unwrap();
    assert_eq!(
        session.fetch_message(42, None).await.unwrap(),
        Some(raw.to_vec())
    );
    peer.await.unwrap();
}
