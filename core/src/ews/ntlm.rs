//! NTLMv2 as EWS needs it: three messages on one connection, no negotiation
//! beyond what the server offers.
//!
//! The server this was built against answers `WWW-Authenticate: Negotiate, NTLM`
//! and nothing else, so basic authentication is not an option and NTLMv1 or LM
//! must never be used. Only `NTLMv2` is computed here; a challenge that does not
//! offer NTLM is refused rather than answered with something weaker.

use crate::secrets::SecretValue;

use super::crypto::{hmac_md5, md4, utf16le};

const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";
const NEGOTIATE_MESSAGE: u32 = 1;
const CHALLENGE_MESSAGE: u32 = 2;
const AUTHENTICATE_MESSAGE: u32 = 3;

/// `NEGOTIATE_UNICODE | OEM | REQUEST_TARGET | NTLM |
/// EXTENDED_SESSIONSECURITY | TARGET_INFO | 128 | 56`.
///
/// `NEGOTIATE_ALWAYS_SIGN` is deliberately absent: it forces the client to send
/// a MIC in the `Type 3` message, which this client does not compute, and a
/// server that enforces it answers the handshake with `500` instead of `401`.
/// NTLM over HTTP does not sign per-message traffic, so declining the flag is
/// the configuration every mainstream NTLM HTTP client uses.
pub(crate) const NEGOTIATE_FLAGS: u32 = 0xa088_0207;

const FLAG_UNICODE: u32 = 0x0000_0001;
const FLAG_NTLM: u32 = 0x0000_0200;
const FLAG_ALWAYS_SIGN: u32 = 0x0000_8000;
const FLAG_EXTENDED_SESSION_SECURITY: u32 = 0x0008_0000;

/// Seconds between 1601-01-01 and 1970-01-01, the FILETIME epoch.
const FILETIME_EPOCH_OFFSET: u64 = 11_644_473_600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NtlmError {
    /// The server's challenge is malformed or too short to trust.
    Malformed,
    /// The server does not offer NTLM, so only a weaker scheme is left.
    Unsupported,
    /// The user name carries no domain and no realm to derive one from.
    MissingDomain,
}

/// What the account stores: a domain user and the password it signed in with.
pub(crate) struct Credentials<'a> {
    pub user: &'a str,
    pub domain: &'a str,
    pub password: &'a SecretValue,
}

impl<'a> Credentials<'a> {
    /// Splits `DOMAIN\user`, `domain/user` and `user@realm` into the two fields
    /// NTLM puts on the wire. A realm-only user name is sent as it is, with no
    /// domain, which is what Exchange expects for a UPN.
    pub(crate) fn parse(input: &'a str, password: &'a SecretValue) -> Self {
        let trimmed = input.trim();
        if let Some((domain, user)) = trimmed.split_once('\\').or_else(|| trimmed.split_once('/'))
            && !domain.is_empty()
            && !user.is_empty()
        {
            return Self {
                user,
                domain,
                password,
            };
        }
        Self {
            user: trimmed,
            domain: "",
            password,
        }
    }
}

/// A challenge read out of the server's `Type 2` message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub flags: u32,
    pub server_challenge: [u8; 8],
    pub target_info: Vec<u8>,
}

impl Challenge {
    /// Whether the server left only a weaker scheme on the table.
    pub(crate) fn lacks_ntlm(&self) -> bool {
        self.flags & FLAG_NTLM == 0 || self.flags & FLAG_EXTENDED_SESSION_SECURITY == 0
    }
}

/// Builds the `Type 1` message: signature, type, and the flags we support.
pub(crate) fn negotiate() -> Vec<u8> {
    let mut message = Vec::with_capacity(16);
    message.extend_from_slice(SIGNATURE);
    message.extend_from_slice(&NEGOTIATE_MESSAGE.to_le_bytes());
    message.extend_from_slice(&NEGOTIATE_FLAGS.to_le_bytes());
    message
}

/// Reads a `Type 2` message. Every offset is range-checked: the payload is
/// server-controlled.
pub(crate) fn parse_challenge(message: &[u8]) -> Result<Challenge, NtlmError> {
    if message.len() < 32 || &message[..8] != SIGNATURE {
        return Err(NtlmError::Malformed);
    }
    if read_u32(message, 8)? != CHALLENGE_MESSAGE {
        return Err(NtlmError::Malformed);
    }
    let flags = read_u32(message, 20)?;
    let mut server_challenge = [0u8; 8];
    server_challenge.copy_from_slice(&message[24..32]);
    // A `Type 2` without target info is legal; then the blob carries only the
    // reserved terminator.
    let target_info = if message.len() >= 48 {
        read_payload(message, 40)?
    } else {
        Vec::new()
    };
    let challenge = Challenge {
        flags,
        server_challenge,
        target_info,
    };
    if challenge.lacks_ntlm() {
        return Err(NtlmError::Unsupported);
    }
    Ok(challenge)
}

/// Builds the `Type 3` message for a challenge, at a given time and with a given
/// client challenge so tests are reproducible.
pub(crate) fn authenticate(
    credentials: &Credentials<'_>,
    challenge: &Challenge,
    timestamp: u64,
    client_challenge: [u8; 8],
    channel_binding: Option<[u8; 16]>,
) -> Vec<u8> {
    let response = ntlmv2_response(
        credentials,
        challenge,
        timestamp,
        client_challenge,
        channel_binding,
    );

    let domain = utf16le(credentials.domain);
    let user = utf16le(credentials.user);
    let workstation = utf16le("");

    let mut payload = Vec::new();
    let lm_offset = 64;
    payload.extend_from_slice(&response.lm);
    let nt_offset = lm_offset + response.lm.len();
    payload.extend_from_slice(&response.nt);
    let domain_offset = nt_offset + response.nt.len();
    payload.extend_from_slice(&domain);
    let user_offset = domain_offset + domain.len();
    payload.extend_from_slice(&user);
    let workstation_offset = user_offset + user.len();
    payload.extend_from_slice(&workstation);

    let mut message = Vec::with_capacity(64 + payload.len());
    message.extend_from_slice(SIGNATURE);
    message.extend_from_slice(&AUTHENTICATE_MESSAGE.to_le_bytes());
    push_field(&mut message, response.lm.len(), lm_offset);
    push_field(&mut message, response.nt.len(), nt_offset);
    push_field(&mut message, domain.len(), domain_offset);
    push_field(&mut message, user.len(), user_offset);
    push_field(&mut message, workstation.len(), workstation_offset);
    // No encrypted session key: `NEGOTIATE_KEY_EXCH` is not requested.
    push_field(&mut message, 0, workstation_offset + workstation.len());
    message.extend_from_slice(&(NEGOTIATE_FLAGS & challenge.flags).to_le_bytes());
    message.extend_from_slice(&payload);
    message
}

struct Response {
    lm: Vec<u8>,
    nt: Vec<u8>,
}

fn ntlmv2_response(
    credentials: &Credentials<'_>,
    challenge: &Challenge,
    timestamp: u64,
    client_challenge: [u8; 8],
    channel_binding: Option<[u8; 16]>,
) -> Response {
    // NT hash of the password, then the keyed hash over the uppercase user name
    // and the domain, exactly as MS-NLMP 3.3.2 defines it.
    let nt_hash = md4(&utf16le(credentials.password.expose_secret()));
    let mut identity = utf16le(&credentials.user.to_uppercase());
    identity.extend_from_slice(&utf16le(credentials.domain));
    let response_key = hmac_md5(&nt_hash, &identity);

    let mut blob = Vec::with_capacity(40 + challenge.target_info.len() + 4);
    blob.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    blob.extend_from_slice(&timestamp.to_le_bytes());
    blob.extend_from_slice(&client_challenge);
    blob.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    blob.extend_from_slice(target_info(challenge, channel_binding).as_slice());
    // Reserved4 terminates the blob.
    blob.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    let mut sealed = Vec::with_capacity(8 + blob.len());
    sealed.extend_from_slice(&challenge.server_challenge);
    sealed.extend_from_slice(&blob);
    let proof = hmac_md5(&response_key, &sealed);

    let mut nt = Vec::with_capacity(16 + blob.len());
    nt.extend_from_slice(&proof);
    nt.extend_from_slice(&blob);

    let mut lm_sealed = Vec::with_capacity(16);
    lm_sealed.extend_from_slice(&challenge.server_challenge);
    lm_sealed.extend_from_slice(&client_challenge);
    let mut lm = Vec::with_capacity(24);
    lm.extend_from_slice(&hmac_md5(&response_key, &lm_sealed));
    lm.extend_from_slice(&client_challenge);

    Response { lm, nt }
}

/// The server's target info, with the channel-binding pair added when the
/// deployment requires extended protection (MS-NLMP 2.2.2.1 `MsvAvChannelBindings`).
fn target_info(challenge: &Challenge, channel_binding: Option<[u8; 16]>) -> Vec<u8> {
    const AV_ID_CHANNEL_BINDINGS: u16 = 0x000a;
    const AV_EOL: u16 = 0x0000;
    let Some(hash) = channel_binding else {
        return challenge.target_info.clone();
    };
    // Drop the server's trailing end-of-list pair, insert ours, then terminate.
    let mut trimmed = challenge.target_info.clone();
    while trimmed.len() >= 4 {
        let end = trimmed.len();
        if trimmed[end - 4..end - 2] != AV_EOL.to_le_bytes()[..] {
            break;
        }
        trimmed.truncate(end - 4);
    }
    trimmed.extend_from_slice(&AV_ID_CHANNEL_BINDINGS.to_le_bytes());
    trimmed.extend_from_slice(&(hash.len() as u16).to_le_bytes());
    trimmed.extend_from_slice(&hash);
    trimmed.extend_from_slice(&AV_EOL.to_le_bytes());
    trimmed.extend_from_slice(&[0u8; 2]);
    trimmed
}

fn push_field(message: &mut Vec<u8>, length: usize, offset: usize) {
    message.extend_from_slice(&(length as u16).to_le_bytes());
    message.extend_from_slice(&(length as u16).to_le_bytes());
    message.extend_from_slice(&(offset as u32).to_le_bytes());
}

fn read_u32(message: &[u8], offset: usize) -> Result<u32, NtlmError> {
    let bytes = message
        .get(offset..offset + 4)
        .ok_or(NtlmError::Malformed)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Reads a `(length, max_length, offset)` field pair at `offset`.
fn read_payload(message: &[u8], offset: usize) -> Result<Vec<u8>, NtlmError> {
    let length = message
        .get(offset..offset + 2)
        .ok_or(NtlmError::Malformed)?;
    let length = u16::from_le_bytes([length[0], length[1]]) as usize;
    let start = message
        .get(offset + 4..offset + 8)
        .ok_or(NtlmError::Malformed)?;
    let start = u32::from_le_bytes([start[0], start[1], start[2], start[3]]) as usize;
    if length == 0 {
        return Ok(Vec::new());
    }
    message
        .get(start..start + length)
        .map(<[u8]>::to_vec)
        .ok_or(NtlmError::Malformed)
}

/// The FILETIME a `Type 3` message carries.
pub(crate) fn filetime(now: std::time::SystemTime) -> u64 {
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (since_epoch.as_secs() + FILETIME_EPOCH_OFFSET) * 10_000_000
        + u64::from(since_epoch.subsec_nanos()) / 100
}

/// Eight random bytes for the client challenge. `uuid` v4 draws from the OS
/// entropy source, which keeps this module free of a direct `getrandom`
/// dependency; a UUID has 122 random bits, far more than the eight bytes used.
pub(crate) fn client_challenge() -> [u8; 8] {
    let id = uuid::Uuid::new_v4();
    let bytes = id.as_bytes();
    [
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// MS-NLMP 4.2.4.1.1: `User` / `Domain` / `Password`, server challenge
    /// `0123456789abcdef`, client challenge `aaaaaaaaaaaaaaaa`, time 0.
    fn sample() -> (Credentials<'static>, Challenge) {
        let password = Box::leak(Box::new(SecretValue::new("Password".to_owned())));
        let credentials = Credentials {
            user: "User",
            domain: "Domain",
            password,
        };
        let mut target_info = Vec::new();
        target_info.extend_from_slice(&[0x02, 0x00, 0x0c, 0x00]);
        target_info.extend_from_slice(&utf16le("Domain"));
        target_info.extend_from_slice(&[0x01, 0x00, 0x0c, 0x00]);
        target_info.extend_from_slice(&utf16le("Server"));
        target_info.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        let challenge = Challenge {
            flags: NEGOTIATE_FLAGS,
            server_challenge: [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef],
            target_info,
        };
        (credentials, challenge)
    }

    #[test]
    fn the_documented_ntlmv2_vector_matches() {
        let (credentials, challenge) = sample();
        let response = ntlmv2_response(&credentials, &challenge, 0, [0xaa; 8], None);
        assert_eq!(
            hex(&response.lm),
            "86c35097ac9cec102554764a57cccc19aaaaaaaaaaaaaaaa",
            "lm response"
        );
        assert_eq!(
            &hex(&response.nt)[..32],
            "68cd0ab851e51c96aabc927bebef6a1c",
            "nt proof"
        );
    }

    #[test]
    fn a_negotiate_message_carries_the_unmodified_key_flags() {
        let message = negotiate();
        assert_eq!(&message[..8], SIGNATURE);
        assert_eq!(read_u32(&message, 8).unwrap(), NEGOTIATE_MESSAGE);
        assert_eq!(read_u32(&message, 12).unwrap(), NEGOTIATE_FLAGS);
        assert!(message.len() <= 40, "a client name is not advertised");
    }

    fn challenge_message(flags: u32, target_info: &[u8]) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(SIGNATURE);
        message.extend_from_slice(&CHALLENGE_MESSAGE.to_le_bytes());
        // Target name: empty, pointing at the end of the message.
        push_field(&mut message, 0, 48);
        message.extend_from_slice(&flags.to_le_bytes());
        message.extend_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        message.extend_from_slice(&[0; 8]);
        let offset = 48;
        push_field(&mut message, target_info.len(), offset);
        message.extend_from_slice(target_info);
        message
    }

    #[test]
    fn a_challenge_is_read_with_its_flags_and_target_info() {
        let target_info = [0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let parsed = parse_challenge(&challenge_message(NEGOTIATE_FLAGS, &target_info)).unwrap();
        assert_eq!(parsed.flags, NEGOTIATE_FLAGS);
        assert_eq!(
            parsed.server_challenge,
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]
        );
        assert_eq!(parsed.target_info, target_info);
    }

    #[test]
    fn a_challenge_that_only_offers_weaker_ntlm_is_refused() {
        let flags = NEGOTIATE_FLAGS & !FLAG_NTLM;
        assert_eq!(
            parse_challenge(&challenge_message(flags, &[])),
            Err(NtlmError::Unsupported)
        );
        let flags = NEGOTIATE_FLAGS & !FLAG_EXTENDED_SESSION_SECURITY;
        assert_eq!(
            parse_challenge(&challenge_message(flags, &[])),
            Err(NtlmError::Unsupported)
        );
    }

    #[test]
    fn a_truncated_or_foreign_challenge_is_malformed() {
        assert_eq!(parse_challenge(&[]), Err(NtlmError::Malformed));
        assert_eq!(parse_challenge(SIGNATURE), Err(NtlmError::Malformed));
        let mut wrong_type = challenge_message(NEGOTIATE_FLAGS, &[]);
        wrong_type[8] = 3;
        assert_eq!(parse_challenge(&wrong_type), Err(NtlmError::Malformed));
        // A target-info offset past the end must not read memory.
        let mut bad_offset = challenge_message(NEGOTIATE_FLAGS, &[0u8; 8]);
        bad_offset[44..48].copy_from_slice(&9000u32.to_le_bytes());
        assert_eq!(parse_challenge(&bad_offset), Err(NtlmError::Malformed));
    }

    #[test]
    fn a_type_three_message_lays_out_the_documented_fields() {
        let (credentials, challenge) = sample();
        let message = authenticate(&credentials, &challenge, 0, [0xaa; 8], None);
        assert_eq!(&message[..8], SIGNATURE);
        assert_eq!(read_u32(&message, 8).unwrap(), AUTHENTICATE_MESSAGE);
        // Domain and user are UTF-16LE and sit where the fields say.
        let domain_len = u16::from_le_bytes([message[28], message[29]]) as usize;
        let domain_offset =
            u32::from_le_bytes([message[32], message[33], message[34], message[35]]) as usize;
        assert_eq!(
            &message[domain_offset..domain_offset + domain_len],
            utf16le("Domain").as_slice()
        );
        let user_len = u16::from_le_bytes([message[36], message[37]]) as usize;
        let user_offset =
            u32::from_le_bytes([message[40], message[41], message[42], message[43]]) as usize;
        assert_eq!(
            &message[user_offset..user_offset + user_len],
            utf16le("User").as_slice()
        );
        assert_eq!(
            read_u32(&message, 60).unwrap(),
            NEGOTIATE_FLAGS & challenge.flags
        );
        // The first payload field ends where the second begins.
        let lm_len = u16::from_le_bytes([message[12], message[13]]) as usize;
        let nt_offset =
            u32::from_le_bytes([message[24], message[25], message[26], message[27]]) as usize;
        assert_eq!(lm_len, 24);
        assert_eq!(nt_offset, 64 + lm_len);
    }

    #[test]
    fn a_channel_binding_hash_replaces_the_end_of_the_target_info() {
        let challenge = Challenge {
            flags: NEGOTIATE_FLAGS,
            server_challenge: [0; 8],
            target_info: vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        };
        let with_binding = target_info(&challenge, Some([0x11; 16]));
        assert_eq!(&with_binding[..4], &[0x02, 0x00, 0x00, 0x00]);
        assert_eq!(&with_binding[4..6], &0x000au16.to_le_bytes());
        assert_eq!(&with_binding[6..8], &16u16.to_le_bytes());
        assert_eq!(&with_binding[8..24], &[0x11; 16]);
        assert_eq!(&with_binding[24..], &[0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn user_names_split_into_domain_and_account() {
        let password = SecretValue::new("synthetic".to_owned());
        for input in ["contoso.com\\alice", "contoso.com/alice"] {
            let credentials = Credentials::parse(input, &password);
            assert_eq!(credentials.domain, "contoso.com", "{input}");
            assert_eq!(credentials.user, "alice", "{input}");
        }
        let credentials = Credentials::parse("alice@contoso.com", &password);
        assert_eq!(credentials.domain, "");
        assert_eq!(credentials.user, "alice@contoso.com");
        let credentials = Credentials::parse("  alice  ", &password);
        assert_eq!(credentials.user, "alice");
    }

    #[test]
    fn a_filetime_counts_100_nanosecond_steps_from_1601() {
        assert_eq!(filetime(std::time::UNIX_EPOCH), 116_444_736_000_000_000);
        let later = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
        assert_eq!(filetime(later), 116_444_736_010_000_000);
    }
}
