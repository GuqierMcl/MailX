//! Local-only probe for synthetic or explicitly supplied mail. JSON output
//! intentionally contains detected codes; the core itself never logs them.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    process::ExitCode,
};

use mailx_core::mail::verification_code::{
    DetectionError, MAX_EML_BYTES, MAX_SUBJECT_BYTES, MAX_TEXT_BYTES, detect, detect_eml,
};

const HELP: &str = "Usage:
  cargo run -p mailx-core --example verification_code_probe -- --eml PATH
  cargo run -p mailx-core --example verification_code_probe -- --subject TEXT --body-file PATH

Use - as PATH to read stdin. In text mode, either --subject or --body-file may
be omitted. --eml cannot be combined with text mode. Inputs are read locally;
no account, database, network, notification or clipboard is accessed.
Output is JSON and intentionally includes detected codes. Use synthetic or
redacted samples when sharing output. This probe does not verify code validity.
";

#[derive(Default)]
struct Arguments {
    eml: Option<OsString>,
    subject: Option<String>,
    body_file: Option<OsString>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), &'static str> {
    let mut input = std::env::args_os().skip(1);
    let mut args = Arguments::default();
    while let Some(arg) = input.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                print!("{HELP}");
                return Ok(());
            }
            Some("--eml") if args.eml.is_none() => {
                args.eml = Some(input.next().ok_or("Missing --eml path; use --help.")?);
            }
            Some("--subject") if args.subject.is_none() => {
                args.subject = Some(
                    input
                        .next()
                        .ok_or("Missing --subject text; use --help.")?
                        .into_string()
                        .map_err(|_| "Subject must be UTF-8 text.")?,
                );
            }
            Some("--body-file") if args.body_file.is_none() => {
                args.body_file = Some(
                    input
                        .next()
                        .ok_or("Missing --body-file path; use --help.")?,
                );
            }
            _ => return Err("Unknown or repeated option; use --help."),
        }
    }
    let detection = if let Some(path) = args.eml {
        if args.subject.is_some() || args.body_file.is_some() {
            return Err("--eml cannot be combined with text mode; use --help.");
        }
        detect_eml(&read_input(path, MAX_EML_BYTES)?).map_err(detection_error)?
    } else {
        if args.subject.is_none() && args.body_file.is_none() {
            return Err("Supply --eml, --subject or --body-file; use --help.");
        }
        let subject = args.subject.unwrap_or_default();
        if subject.len() > MAX_SUBJECT_BYTES {
            return Err("Input exceeds the verification-code probe size limit.");
        }
        let body = args
            .body_file
            .map(|path| read_input(path, MAX_TEXT_BYTES))
            .transpose()?
            .unwrap_or_default();
        let body = std::str::from_utf8(&body).map_err(|_| "Body must be UTF-8 text.")?;
        detect(&subject, body).map_err(detection_error)?
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &detection)
        .map_err(|_| "Could not write probe result.")?;
    writeln!(out).map_err(|_| "Could not write probe result.")
}

fn read_input(path: OsString, limit: usize) -> Result<Vec<u8>, &'static str> {
    let input: Box<dyn Read> = if path == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(File::open(path).map_err(|_| "Could not open input file.")?)
    };
    let mut bytes = Vec::new();
    input
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read input.")?;
    if bytes.len() > limit {
        return Err("Input exceeds the verification-code probe size limit.");
    }
    Ok(bytes)
}

fn detection_error(error: DetectionError) -> &'static str {
    match error {
        DetectionError::InputTooLarge => "Input exceeds the verification-code probe size limit.",
        DetectionError::InvalidMessage => "Input is not a supported complete MIME message.",
    }
}
