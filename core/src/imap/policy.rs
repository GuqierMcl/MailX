//! Recognize one documented rejection without retaining or exposing server text.
use async_imap::{
    error::Error,
    imap_proto::{Response, Status},
};

const MAX_REASON_BYTES: usize = 4096;
const UNSAFE_LOGIN: &[u8] = b"Unsafe Login. Please contact kefu@188.com for help";

fn unsafe_login(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes[..bytes.len().min(MAX_REASON_BYTES)]
        .windows(UNSAFE_LOGIN.len())
        .any(|window| window.eq_ignore_ascii_case(UNSAFE_LOGIN))
}

pub(super) fn response(response: &Response<'_>) -> bool {
    match response {
        Response::Done {
            status: Status::No | Status::Bad,
            outcome,
            ..
        }
        | Response::Data {
            status: Status::No | Status::Bad | Status::Bye,
            outcome,
        } => outcome.information.as_deref().is_some_and(unsafe_login),
        _ => false,
    }
}

pub(super) fn library_rejection(error: &Error) -> bool {
    match error {
        // async-imap formats the status code/information into these strings.
        Error::No(reason) | Error::Bad(reason) => unsafe_login(reason),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_documented_signal_with_a_bounded_scan() {
        assert!(unsafe_login(
            "Unsafe Login. Please contact kefu@188.com for help private"
        ));
        assert!(unsafe_login(
            "UNSAFE LOGIN. PLEASE CONTACT KEFU@188.COM FOR HELP"
        ));
        for text in [
            "invalid credentials",
            "unsafe login",
            "Unsafe Logins",
            "not safe",
        ] {
            assert!(!unsafe_login(text));
        }
        assert!(!unsafe_login(&format!(
            "{}Unsafe Login. Please contact kefu@188.com for help",
            "x".repeat(MAX_REASON_BYTES)
        )));
    }

    #[test]
    fn only_negative_status_information_is_a_policy_signal() {
        for status in ["NO", "BAD", "BYE"] {
            let raw = format!("* {status} Unsafe Login. Please contact kefu@188.com for help\r\n");
            let (_, parsed) = Response::parse(raw.as_bytes()).unwrap();
            assert!(response(&parsed));
        }
        let raw = b"* OK Unsafe Login. Please contact kefu@188.com for help\r\n";
        let (_, parsed) = Response::parse(raw).unwrap();
        assert!(!response(&parsed));
    }
}
