//! Offline language detection. CJK scripts are decided by script counts, since
//! trigram models are weakest exactly where one script serves several
//! languages; everything else goes through whatlang.
use whatlang::Lang;

/// Fewer alphabetic characters than this say nothing about the language.
const MIN_ALPHABETIC: usize = 10;
/// Weighted length (a CJK character counts 3) needed to call a result reliable.
const RELIABLE_WEIGHT: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    /// Lowercase ISO 639-1 code when one exists, else ISO 639-3.
    pub language: String,
    pub reliable: bool,
}

#[derive(Default)]
struct Counts {
    han: usize,
    kana: usize,
    hangul: usize,
    other: usize,
}

impl Counts {
    fn cjk(&self) -> usize {
        self.han + self.kana + self.hangul
    }

    fn total(&self) -> usize {
        self.cjk() + self.other
    }
}

pub fn detect_language(text: &str) -> Option<Detection> {
    let cleaned = strip_noise(text);
    let mut counts = Counts::default();
    for ch in cleaned.chars().filter(|ch| ch.is_alphabetic()) {
        match ch {
            '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{323af}' => counts.han += 1,
            '\u{3040}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' | '\u{ff66}'..='\u{ff9f}' => {
                counts.kana += 1
            }
            '\u{1100}'..='\u{11ff}'
            | '\u{3130}'..='\u{318f}'
            | '\u{a960}'..='\u{a97f}'
            | '\u{ac00}'..='\u{d7ff}' => counts.hangul += 1,
            _ => counts.other += 1,
        }
    }
    if counts.total() < MIN_ALPHABETIC {
        return None;
    }
    let enough = counts.cjk() * 3 + counts.other >= RELIABLE_WEIGHT;
    if counts.cjk() * 5 >= counts.total() {
        // Simplified and Traditional are not told apart.
        let language = if counts.hangul > counts.han + counts.kana {
            "ko"
        } else if counts.kana * 20 >= counts.cjk() {
            "ja"
        } else {
            "zh"
        };
        return Some(Detection {
            language: language.to_owned(),
            reliable: enough,
        });
    }
    let info = whatlang::detect(&cleaned)?;
    Some(Detection {
        language: iso_code(info.lang()),
        reliable: enough && info.is_reliable(),
    })
}

/// ISO 639-1 where one exists, otherwise whatlang's ISO 639-3 code.
fn iso_code(lang: Lang) -> String {
    let code = lang.code();
    let short = match code {
        "epo" => "eo",
        "eng" => "en",
        "rus" => "ru",
        "cmn" => "zh",
        "spa" => "es",
        "por" => "pt",
        "ita" => "it",
        "ben" => "bn",
        "fra" => "fr",
        "deu" => "de",
        "ukr" => "uk",
        "kat" => "ka",
        "ara" => "ar",
        "hin" => "hi",
        "jpn" => "ja",
        "heb" => "he",
        "yid" => "yi",
        "pol" => "pl",
        "amh" => "am",
        "jav" => "jv",
        "kor" => "ko",
        "nob" => "nb",
        "dan" => "da",
        "swe" => "sv",
        "fin" => "fi",
        "tur" => "tr",
        "nld" => "nl",
        "hun" => "hu",
        "ces" => "cs",
        "ell" => "el",
        "bul" => "bg",
        "bel" => "be",
        "mar" => "mr",
        "kan" => "kn",
        "ron" => "ro",
        "slv" => "sl",
        "hrv" => "hr",
        "srp" => "sr",
        "mkd" => "mk",
        "lit" => "lt",
        "lav" => "lv",
        "est" => "et",
        "tam" => "ta",
        "vie" => "vi",
        "urd" => "ur",
        "tha" => "th",
        "guj" => "gu",
        "uzb" => "uz",
        "pan" => "pa",
        "aze" => "az",
        "ind" => "id",
        "tel" => "te",
        "pes" => "fa",
        "mal" => "ml",
        "ori" => "or",
        "mya" => "my",
        "nep" => "ne",
        "sin" => "si",
        "khm" => "km",
        "tuk" => "tk",
        "aka" => "ak",
        "zul" => "zu",
        "sna" => "sn",
        "afr" => "af",
        "lat" => "la",
        "slk" => "sk",
        "cat" => "ca",
        "tgl" => "tl",
        "hye" => "hy",
        "cym" => "cy",
        other => other,
    };
    short.to_owned()
}

/// Drop what carries no language signal: URLs, e-mail addresses and digits.
fn strip_noise(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        let word_start = at == 0 || !chars[at - 1].is_alphanumeric();
        if word_start
            && (starts_with(&chars, at, "http://")
                || starts_with(&chars, at, "https://")
                || starts_with(&chars, at, "www."))
        {
            // A URL ends at whitespace, a quote/bracket or the first non-ASCII
            // character, which is where CJK text usually resumes.
            while at < chars.len()
                && chars[at].is_ascii()
                && !chars[at].is_whitespace()
                && !matches!(chars[at], '<' | '>' | '"' | '\'' | ')' | ']')
            {
                at += 1;
            }
            out.push(' ');
        } else if ch == '@' && out.ends_with(is_local_char) {
            while out.ends_with(is_local_char) {
                out.pop();
            }
            at += 1;
            while at < chars.len()
                && (chars[at].is_ascii_alphanumeric() || matches!(chars[at], '.' | '-' | '_'))
            {
                at += 1;
            }
            out.push(' ');
        } else if ch.is_numeric() {
            at += 1;
        } else {
            out.push(ch);
            at += 1;
        }
    }
    out
}

fn is_local_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-')
}

fn starts_with(chars: &[char], at: usize, prefix: &str) -> bool {
    prefix.chars().enumerate().all(|(offset, want)| {
        chars
            .get(at + offset)
            .is_some_and(|have| have.to_ascii_lowercase() == want)
    })
}
