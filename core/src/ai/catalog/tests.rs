use super::*;
use crate::net::test_support::{MockReply, mock_http};

pub(crate) const FIXTURE: &str = r#"{
  "openai": {"id":"openai","env":["OPENAI_API_KEY"],"npm":"@ai-sdk/openai","name":"OpenAI","models":{
    "gpt-a":{"id":"gpt-a","name":"GPT A","tool_call":true,"limit":{"context":400000,"output":128000},"modalities":{"input":["text"],"output":["text"]}},
    "embed":{"id":"embed","name":"Embed","type":"embedding","limit":{"context":8000,"output":0},"modalities":{"output":["text"]}},
    "pic":{"id":"pic","name":"Pic","modalities":{"output":["image"]}},
    "odd":{"id":"odd","name":"Odd","provider":{"npm":"@ai-sdk/other"}},
    "nolimit":{"id":"nolimit","name":"No limit"}
  }},
  "openrouter": {"id":"openrouter","env":["K"],"npm":"@openrouter/ai-sdk-provider","api":"https://elsewhere.example/v9","name":"OpenRouter","models":{"x/y":{"id":"x/y","name":"XY","limit":{"context":1000,"output":10}}}},
  "acme": {"id":"acme","env":["ACME_KEY"],"npm":"@ai-sdk/openai-compatible","api":"https://api.acme.ai/v1/","name":"Acme","models":{"a1":{"id":"a1","name":"A1","limit":{"context":4294967296,"output":5}}}},
  "zeta": {"id":"zeta","env":[],"npm":"@ai-sdk/anthropic","api":"https://zeta.example/anthropic/v1","name":"beta Zeta","models":{}},
  "multi": {"id":"multi","env":["A","B"],"npm":"@ai-sdk/openai-compatible","api":"https://multi.example/v1","name":"Multi","models":{}},
  "templated": {"id":"templated","env":["A"],"npm":"@ai-sdk/openai-compatible","api":"https://${HOST}/v1","name":"Templated","models":{}},
  "azure": {"id":"azure","env":["A"],"npm":"@ai-sdk/azure","api":"https://azure.example/v1","name":"Azure","models":{}},
  "noapi": {"id":"noapi","env":["A"],"npm":"@ai-sdk/openai-compatible","name":"No API","models":{}},
  "badurl": {"id":"badurl","env":["A"],"npm":"@ai-sdk/openai-compatible","api":"ftp://x.example","name":"Bad URL","models":{}},
  "Bad_Id": {"id":"Bad_Id","env":["A"],"npm":"@ai-sdk/openai-compatible","api":"https://b.example/v1","name":"Bad id","models":{}},
  "custom": {"id":"custom","env":["A"],"npm":"@ai-sdk/openai-compatible","api":"https://c.example/v1","name":"Custom","models":{}},
  "broken": 5
}"#;

fn trimmed() -> Vec<CatalogProvider> {
    trim(FIXTURE.as_bytes()).unwrap()
}

fn find<'a>(providers: &'a [CatalogProvider], id: &str) -> Option<&'a CatalogProvider> {
    providers.iter().find(|provider| provider.id == id)
}

#[test]
fn trimming_keeps_only_usable_providers() {
    let providers = trimmed();
    let mut ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["acme", "openai", "openrouter", "zeta"]);

    // Built-ins keep the table's protocol and address, whatever the catalog says.
    let openai = find(&providers, "openai").unwrap();
    assert_eq!(openai.protocol, AiProtocol::OpenaiCompatible);
    assert_eq!(openai.base_url, "https://api.openai.com/v1");
    assert_eq!(
        find(&providers, "openrouter").unwrap().base_url,
        "https://openrouter.ai/api/v1"
    );

    let acme = find(&providers, "acme").unwrap();
    assert_eq!(acme.protocol, AiProtocol::OpenaiCompatible);
    assert_eq!(acme.base_url, "https://api.acme.ai/v1");
    assert_eq!(
        find(&providers, "zeta").unwrap().protocol,
        AiProtocol::Anthropic
    );
}

#[test]
fn trimming_reduces_models_to_the_fields_mailx_uses() {
    let providers = trimmed();
    let openai = find(&providers, "openai").unwrap();
    // The model with a `provider` override is skipped.
    let ids: Vec<&str> = openai.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["embed", "gpt-a", "nolimit", "pic"]);
    let gpt = &openai.models[1];
    assert_eq!(
        gpt,
        &CatalogModel {
            id: "gpt-a".into(),
            name: "GPT A".into(),
            context: Some(400_000),
            output: Some(128_000),
            tool_call: Some(true),
            text: true,
        }
    );
    // Not text models stay only as exclusions.
    assert!(!openai.models[0].text && openai.models[0].context.is_none());
    assert!(!openai.models[3].text);
    let nolimit = &openai.models[2];
    assert_eq!(
        (nolimit.context, nolimit.output, nolimit.tool_call),
        (None, None, None)
    );
    // A number that does not fit is unknown, not wrong.
    assert_eq!(find(&providers, "acme").unwrap().models[0].context, None);
    assert_eq!(find(&providers, "acme").unwrap().models[0].output, Some(5));
}

#[test]
fn trimming_rejects_a_document_that_is_not_a_provider_map() {
    assert!(trim(b"[]").is_none());
    assert!(trim(b"nope").is_none());
    assert_eq!(trim(b"{}").unwrap().len(), 0);
}

#[test]
fn the_provider_list_puts_built_ins_first_then_the_rest_by_name() {
    let data = CatalogData::new(None, 0, trimmed());
    let providers = providers(Some(&data));
    let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
    let built_in_ids: Vec<&str> = BUILT_INS.iter().map(|b| b.id).collect();
    assert_eq!(&ids[..BUILT_INS.len()], built_in_ids.as_slice());
    // "Acme" before "beta Zeta" (case-insensitive).
    assert_eq!(&ids[BUILT_INS.len()..], ["acme", "zeta"]);
    assert!(providers[..BUILT_INS.len()].iter().all(|p| p.built_in));
    assert!(providers[BUILT_INS.len()..].iter().all(|p| !p.built_in));
    let ollama = providers.iter().find(|p| p.id == "ollama").unwrap();
    assert_eq!(ollama.local, Some(true));
    assert_eq!(ollama.base_url, "http://127.0.0.1:11434/v1");
    assert_eq!(providers[0].local, None);
    // Offline: only the built-ins.
    assert_eq!(super::providers(None).len(), BUILT_INS.len());
}

#[test]
fn built_ins_are_valid_and_unique() {
    let mut seen = std::collections::HashSet::new();
    for entry in BUILT_INS {
        assert!(seen.insert(entry.id));
        assert!(valid_provider_id(entry.id));
        assert_eq!(
            normalize_base_url(entry.base_url).as_deref(),
            Some(entry.base_url)
        );
        assert_eq!(entry.local, entry.base_url.starts_with("http://127.0.0.1"));
        assert!(entry.name.chars().count() <= 64);
    }
    assert!(!seen.contains(CUSTOM_PROVIDER_ID));
}

#[test]
fn the_cache_round_trips_and_damage_reads_as_no_cache() {
    let directory = tempfile::tempdir().unwrap();
    assert!(read_cache(directory.path()).is_none());
    let data = CatalogData::new(Some("\"abc\"".into()), 1_700_000_000, trimmed());
    write_cache(&directory.path().join("nested"), &data).unwrap();
    assert_eq!(read_cache(&directory.path().join("nested")).unwrap(), data);
    // No temporary files stay behind.
    assert_eq!(
        std::fs::read_dir(directory.path().join("nested"))
            .unwrap()
            .count(),
        1
    );

    let path = directory.path().join("nested").join(CACHE_FILE);
    std::fs::write(&path, b"{ broken").unwrap();
    assert!(read_cache(&directory.path().join("nested")).is_none());
    let mut value = serde_json::to_value(&data).unwrap();
    value["version"] = 99.into();
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(read_cache(&directory.path().join("nested")).is_none());
}

#[test]
fn staleness_is_24_hours() {
    let data = CatalogData::new(None, 1000, Vec::new());
    assert!(!data.stale(1000 + STALE_AFTER_SECONDS));
    assert!(data.stale(1000 + STALE_AFTER_SECONDS + 1));
}

#[tokio::test]
async fn fetching_is_conditional_on_the_etag() {
    let server = mock_http(|request| {
        if request.header("if-none-match") == Some("\"v1\"") {
            MockReply::status(304)
        } else {
            MockReply::json(200, FIXTURE).header("ETag", "\"v1\"")
        }
    })
    .await;
    let url = Url::parse(&format!("{}/api.json", server.url())).unwrap();
    let route = NetRoute::default();
    let Fetched::Fresh { etag, providers } = fetch(&route, &url, None).await.unwrap() else {
        panic!("expected data");
    };
    assert_eq!(etag.as_deref(), Some("\"v1\""));
    assert_eq!(providers.len(), 4);
    assert_eq!(server.last().header("if-none-match"), None);
    assert!(matches!(
        fetch(&route, &url, Some("\"v1\"")).await.unwrap(),
        Fetched::NotModified
    ));
    assert_eq!(server.last().header("if-none-match"), Some("\"v1\""));
}

#[tokio::test]
async fn fetch_failures_are_fixed_codes() {
    let route = NetRoute::default();
    for (status, body, code) in [
        (500, "x", ErrorCode::AiUnavailable),
        (404, "x", ErrorCode::AiRequestRejected),
        (200, "not json", ErrorCode::AiInvalidResponse),
        (200, "[]", ErrorCode::AiInvalidResponse),
    ] {
        let server = mock_http(move |_| MockReply::json(status, body)).await;
        let url = Url::parse(&server.url()).unwrap();
        assert_eq!(
            fetch(&route, &url, None).await.unwrap_err(),
            code,
            "{status} {body}"
        );
    }
}
