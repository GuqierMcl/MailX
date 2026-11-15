use super::*;

#[test]
fn candidate_subject_filter_is_broad_but_does_not_admit_replies_or_unrelated_mail() {
    for subject in [
        "Your verification code",
        "Sign in to Example",
        "Verify your email",
        "登录通知",
        "驗證電郵",
    ] {
        assert!(is_candidate_subject(subject), "{subject}");
    }
    for subject in [
        "Re: Your verification code",
        "Fwd: Sign in",
        "Invoice 123456",
        "A verifier's report",
        "",
        "otpidentifier",
    ] {
        assert!(!is_candidate_subject(subject), "{subject}");
    }
    assert!(!is_candidate_subject(&"OTP".repeat(MAX_SUBJECT_BYTES)));
}

fn found(subject: &str, body: &str, expected: &str) {
    let result = detect(subject, body).unwrap();
    let Detection::Found { candidate } = result else {
        panic!("expected a unique code for synthetic input {subject:?} / {body:?}: {result:?}");
    };
    assert_eq!(candidate.code, expected);
    assert!(!candidate.evidence.is_empty());
}

#[test]
fn recognizes_explicit_english_chinese_and_mixed_codes() {
    for (subject, body, expected) in [
        (
            "",
            "Your verification code is 001234. Valid for 5 minutes.",
            "001234",
        ),
        ("", "验证码：482913，5分钟内有效。", "482913"),
        ("", "您的驗證碼為482913，請勿分享。", "482913"),
        ("", "482913 is your verification code.", "482913"),
        (
            "",
            "Your one-time password:\n\n482913\n\nExpires in 5 minutes.",
            "482913",
        ),
        ("", "Security code: A1b2C3", "A1b2C3"),
        ("", "VERIFICATION CODE:482913", "482913"),
        (
            "Your verification code",
            "482913\nThis code expires in five minutes.",
            "482913",
        ),
        ("Your sign-in code", "Use this code: 482913", "482913"),
        ("", "Use 482913 to verify your email.", "482913"),
        ("", "验证码为 1234", "1234"),
        ("", "验证码：12345678", "12345678"),
        ("", "Your security code is:\r\nA1B2C3", "A1B2C3"),
        ("", "Your verification code is\u{00a0}482913", "482913"),
    ] {
        found(subject, body, expected);
    }
}

#[test]
fn repeated_code_merges_evidence_but_conflicts_never_pick_a_winner() {
    let result = detect(
        "482913 is your verification code",
        "Verification code: 482913.\nUse 482913 to sign in.",
    )
    .unwrap();
    let Detection::Found { candidate } = result else {
        panic!("{result:?}");
    };
    assert_eq!(candidate.code, "482913");
    assert!(
        candidate
            .evidence
            .iter()
            .any(|e| e.source == Source::Subject)
    );
    assert!(candidate.evidence.iter().any(|e| e.source == Source::Body));
    for (subject, body) in [
        (
            "Your verification code: 482913",
            "Your verification code: 762951",
        ),
        ("", "Login code: 482913\nSecurity code: 762951"),
        ("", "Security code: A1b2C3\nSecurity code: A1B2C3"),
        ("Verification code", "482913\n\nSecurity code: 762951"),
        ("", "Verification code: 123456 or 654321"),
        ("", "验证码：123456 或 654321"),
        ("", "Verification code: 123456, 654321"),
        ("", "验证码：123456，654321"),
    ] {
        let result = detect(subject, body).unwrap();
        assert!(matches!(result, Detection::Ambiguous { candidates } if candidates.len() == 2));
    }
}

#[test]
fn rejects_contextless_and_unrelated_numbers() {
    for (subject, body) in [
        ("", "482913"),
        ("", "Your code is 482913"),
        (
            "Your verification code",
            "Order 123456 has shipped. Your code will arrive separately.",
        ),
        ("Your verification code", "Previous code:\n123456"),
        ("Your verification code", "Order number:\n123456"),
        (
            "",
            "Your verification code expires in 2026. Please request a new one.",
        ),
        (
            "",
            "Never share a verification code. Support ticket 482913.",
        ),
        ("", "Verification code: order number 123456"),
        ("", "Verification code: $1234.50"),
        ("", "Verification code: 1234%"),
        ("", "Verification code: +12345678"),
        (
            "",
            "Your verification code is at https://example.com/?code=482913",
        ),
        (
            "Verification code",
            "https://example.com/482913\ncode=482913@example.com",
        ),
        ("", "Verification code: https://example.com/482913"),
        ("", "xotp: 123456"),
        ("", "Verification codes: 123456"),
        ("", "Your previous verification code: 123456"),
        ("", "Do not use verification code: 123456"),
        ("", "示例验证码：123456"),
    ] {
        assert_eq!(
            detect(subject, body),
            Ok(Detection::NotFound),
            "{subject:?} / {body:?}"
        );
    }
    found(
        "",
        "Verification code: 482913. Order: 762951. Amount: $1234.50. Call 800-555-1234.",
        "482913",
    );
}

#[test]
fn never_extracts_a_fragment_of_an_unsupported_format() {
    for value in [
        "123",
        "123456789",
        "A123456789Z",
        "123456.789",
        "123456,789",
        "1234-5678",
        "A1B2-C3",
        "123 456",
        "1234 5678",
        "1234\n56",
        "12:3456",
        "2026-11-13",
        "１２３４５６",
        "1234\u{200b}56",
        "1234\u{2060}56",
        "1234\u{0301}56",
        "1234_56",
        "é123456",
        "123456é",
        "123456７",
        "123456\u{fffd}",
        "1234- 5678",
        "1234 / 5678",
        "1234\u{00ad}56",
        "1234\u{2011}56",
        "1234\u{fe0f}56",
        "1234\u{0000}56",
        "1234\u{001b}56",
        "ABC123 XYZ789",
    ] {
        let body = format!("Verification code: {value}");
        assert_eq!(detect("", &body), Ok(Detection::NotFound), "{value:?}");
    }
}

#[test]
fn ignores_reply_subjects_and_common_plain_quotes() {
    for (subject, body) in [
        (
            "Re: Verification code: 762951",
            "This code did not work, please help.",
        ),
        (" Fwd: Verification code: 762951", "See below"),
        ("回复：验证码：762951", "这是昨天收到的。"),
        (
            "",
            "Please see the earlier message.\n> Verification code: 762951",
        ),
        ("", "On Tue, Ada wrote:\nVerification code: 762951"),
        ("", "-----Original Message-----\nVerification code: 762951"),
        (
            "",
            "---------- Forwarded message ---------\nVerification code: 762951",
        ),
        ("", "在周二，小明写道：\n验证码：762951"),
    ] {
        assert_eq!(
            detect(subject, body),
            Ok(Detection::NotFound),
            "{subject:?} / {body:?}"
        );
    }
    found(
        "Re: Verification code: 762951",
        "Your current verification code: 482913\n\nOn Tue, Ada wrote:\n> Verification code: 762951",
        "482913",
    );
    found(
        "",
        "> Verification code: 762951\nYour verification code: 482913",
        "482913",
    );
}

#[test]
fn bounds_inputs_and_candidate_count_without_truncation() {
    assert_eq!(
        detect(&"x".repeat(MAX_SUBJECT_BYTES + 1), ""),
        Err(DetectionError::InputTooLarge)
    );
    assert_eq!(
        detect("Verification code: 123456", &"x".repeat(MAX_TEXT_BYTES + 1)),
        Err(DetectionError::InputTooLarge)
    );
    let body = (100_000..100_017)
        .map(|code| format!("Verification code: {code}\n"))
        .collect::<String>();
    assert_eq!(detect("", &body), Err(DetectionError::InputTooLarge));
    let body = "Verification code: 123456\n".repeat(1000);
    found("", &body, "123456");
    // UTF-8 context windows can start inside a multibyte character.
    found(
        "",
        &format!("{} Verification code: 123456", "🙂".repeat(30)),
        "123456",
    );
    assert_eq!(
        detect("Your verification code", &"1234!".repeat(13_000)),
        Ok(Detection::NotFound)
    );
}

#[test]
fn diagnostics_redact_values_but_explicit_probe_serialization_contains_them() {
    let found = detect("", "Verification code: 482913").unwrap();
    assert!(!format!("{found:?}").contains("482913"));
    let json = serde_json::to_value(found).unwrap();
    assert_eq!(json["status"], "found");
    assert_eq!(json["candidate"]["code"], "482913");
    assert_eq!(json["candidate"]["evidence"][0]["source"], "body");
    assert_eq!(
        json["candidate"]["evidence"][0]["keyword"],
        "verification code"
    );
}
