use super::*;

use tokio::io::AsyncBufReadExt;

async fn scripted_fetch(
    uids: &[u32],
    response: Vec<u8>,
) -> Result<Vec<(u32, PartialFetch)>, ImapError> {
    scripted_sized(uids, &vec![None; uids.len()], response).await
}

async fn scripted_sized(
    uids: &[u32],
    sizes: &[Option<u64>],
    response: Vec<u8>,
) -> Result<Vec<(u32, PartialFetch)>, ImapError> {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let expected = command(uids);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut line = String::new();
        server.read_line(&mut line).await.unwrap();
        assert_eq!(line, expected);
        let _ = server.get_mut().write_all(&response).await;
    });
    let result = fetch(&mut client, uids, limits(sizes)).await;
    drop(client);
    server.await.unwrap();
    result
}

fn message(uid: u32, header: &[u8], text: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "* {uid} FETCH (UID {uid} BODY[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING MIME-VERSION)] {{{}}}\r\n",
        header.len()
    )
    .into_bytes();
    response.extend_from_slice(header);
    response.extend_from_slice(format!(" BODY[TEXT]<0> {{{}}}\r\n", text.len()).as_bytes());
    response.extend_from_slice(text);
    response.extend_from_slice(b")\r\n");
    response
}

fn done(reply: &str) -> Vec<u8> {
    format!("{TAG} {reply}\r\n").into_bytes()
}

#[test]
fn command_is_a_single_peek_partial_fetch() {
    assert_eq!(
        command(&[3, 7, 12]),
        "MAILXPREV UID FETCH 3,7,12 (UID BODY.PEEK[HEADER.FIELDS (CONTENT-TYPE CONTENT-TRANSFER-ENCODING MIME-VERSION)] BODY.PEEK[TEXT]<0.16384>)\r\n"
    );
}

#[tokio::test]
async fn batch_is_collected_per_uid_and_missing_uids_are_absent() {
    let mut response = message(3, b"Content-Type: text/plain\r\n\r\n", b"first body");
    response.extend(message(
        7,
        b"\r\n",
        b"\0\xffbinary)\r\nMAILXPREV OK fake\r\n",
    ));
    response.extend(b"* 3 EXISTS\r\n");
    response.extend(done("OK done"));
    let fetched = scripted_fetch(&[3, 7, 12], response).await.unwrap();
    assert_eq!(
        fetched,
        [
            (
                3,
                PartialFetch::Data {
                    header: b"Content-Type: text/plain\r\n\r\n".to_vec(),
                    text: b"first body".to_vec()
                }
            ),
            (
                7,
                PartialFetch::Data {
                    header: b"\r\n".to_vec(),
                    text: b"\0\xffbinary)\r\nMAILXPREV OK fake\r\n".to_vec()
                }
            ),
        ]
    );
}

#[tokio::test]
async fn nil_and_quoted_sections_are_decoded() {
    let response = [
        b"* 1 FETCH (UID 4 BODY[HEADER.FIELDS (CONTENT-TYPE)] \"a\\\"b\" BODY[TEXT]<0> NIL)\r\n"
            .to_vec(),
        done("OK"),
    ]
    .concat();
    assert_eq!(
        scripted_fetch(&[4], response).await.unwrap(),
        [(
            4,
            PartialFetch::Data {
                header: b"a\"b".to_vec(),
                text: Vec::new()
            }
        )]
    );
}

#[tokio::test]
async fn a_literal_longer_than_requested_is_truncated_locally() {
    // A server that ignored `<0.n>` and sent the whole text.
    let mut long = vec![b'x'; PREVIEW_BYTES as usize];
    long.extend(vec![b'y'; 40 * 1024]);
    let mut response = message(5, b"\r\n", &long);
    response.extend(message(6, b"\r\n", b"fine"));
    response.extend(done("OK"));
    let fetched = scripted_sized(&[5, 6], &[Some(50_000), Some(300)], response)
        .await
        .unwrap();
    let (5, PartialFetch::Data { text, .. }) = &fetched[0] else {
        panic!("{:?}", fetched[0]);
    };
    assert_eq!(text, &vec![b'x'; PREVIEW_BYTES as usize]);
    assert!(matches!(fetched[1], (6, PartialFetch::Data { .. })));
}

#[tokio::test]
async fn a_literal_beyond_the_fallback_cap_is_oversized() {
    let size = PREVIEW_FALLBACK_BYTES + 1;
    let long = vec![b'x'; size as usize];
    let mut response = message(5, b"\r\n", &long);
    response.extend(message(6, b"\r\n", b"fine"));
    response.extend(done("OK"));
    // The stored sizes claim less than was sent, so the budget still admits it.
    let fetched = scripted_sized(&[5, 6], &[None, Some(300)], response)
        .await
        .unwrap();
    assert_eq!(fetched[0], (5, PartialFetch::Oversized));
    assert!(matches!(fetched[1], (6, PartialFetch::Data { .. })));
}

#[tokio::test]
async fn a_single_huge_reply_exceeds_its_own_budget() {
    let long = vec![b'x'; 3 * PREVIEW_FALLBACK_BYTES as usize];
    let response = [message(5, b"\r\n", &long), done("OK")].concat();
    assert!(matches!(
        scripted_sized(&[5], &[Some(3 * PREVIEW_FALLBACK_BYTES)], response).await,
        Err(ImapError::TooLarge)
    ));
}

#[test]
fn the_budget_follows_the_batch_sizes() {
    let slack = |sizes: &[Option<u64>]| limits(sizes).payload;
    let one = slack(&[Some(1000)]);
    assert_eq!(one, 1000 + HEADER_LIMIT + PAYLOAD_SLACK);
    // Capped per message; unknown counts as the cap.
    assert_eq!(
        slack(&[Some(u64::MAX), None, Some(10)]),
        2 * PREVIEW_FALLBACK_BYTES as usize + 10 + 3 * HEADER_LIMIT + PAYLOAD_SLACK
    );
}

#[tokio::test]
async fn unrequested_duplicate_or_foreign_data_is_a_protocol_error() {
    for response in [
        [message(9, b"\r\n", b"x"), done("OK")].concat(),
        [
            message(5, b"\r\n", b"x"),
            message(5, b"\r\n", b"y"),
            done("OK"),
        ]
        .concat(),
        [
            b"* 1 FETCH (UID 5 BODY[] {1}\r\nx)\r\n".to_vec(),
            done("OK"),
        ]
        .concat(),
        [b"* 1 FETCH (UID 5)\r\n".to_vec(), done("OK")].concat(),
    ] {
        let error = scripted_fetch(&[5], response).await.unwrap_err();
        assert!(matches!(error, ImapError::Protocol(_)), "{error:?}");
    }
}

#[tokio::test]
async fn rejection_and_missing_completion_are_errors() {
    assert!(matches!(
        scripted_fetch(&[5], done("NO private-marker")).await,
        Err(ImapError::Rejected)
    ));
    assert!(scripted_fetch(&[5], Vec::new()).await.is_err());
    assert!(scripted_fetch(&[5], b"+ go\r\n".to_vec()).await.is_err());
}

#[tokio::test]
async fn a_declaration_beyond_the_batch_budget_is_rejected_before_reading() {
    let response = [
        b"* 1 FETCH (UID 5 BODY[TEXT]<0> {999999999}\r\n".to_vec(),
        done("OK"),
    ]
    .concat();
    assert!(matches!(
        scripted_fetch(&[5], response).await,
        Err(ImapError::TooLarge)
    ));
}
