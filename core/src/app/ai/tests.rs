use super::*;
use crate::{
    accounts::types::ProxyKind,
    ai::{AiModelSource, catalog::CatalogData},
    net::test_support::{MockHttp, MockReply, fake_http_connect, mock_http},
    secrets::test_support::MemorySecrets,
};

const SECRET: &str = "sk-super-secret-key-123";
const LIST: &str = r#"{"data":[{"id":"gpt-a","name":"Own name"},{"id":"embed"},{"id":"zzz"}]}"#;
const CHAT: &str = r#"{"choices":[{"message":{"content":"OK"}}]}"#;

struct Fixture {
    directory: tempfile::TempDir,
    secrets: Arc<MemorySecrets>,
    settings: Arc<SettingsStore>,
    service: Arc<AiService>,
}

fn fixture_with(catalog_url: &str, icon_base: &str) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let settings = Arc::new(SettingsStore::open(directory.path()).unwrap());
    let service = Arc::new(
        AiService::new(settings.clone(), secrets.clone(), directory.path())
            .with_endpoints(catalog_url, icon_base),
    );
    Fixture {
        directory,
        secrets,
        settings,
        service,
    }
}

fn fixture() -> Fixture {
    fixture_with("http://127.0.0.1:1/api.json", "http://127.0.0.1:1")
}

fn request(url: &str, key: Option<&str>, verify: bool) -> SaveAiConnection {
    SaveAiConnection {
        id: None,
        provider_id: "custom".into(),
        name: "  Mine ".into(),
        protocol: AiProtocol::OpenaiCompatible,
        base_url: format!("{url}/v1/"),
        use_proxy: false,
        api_key: key.map(|key| SecretValue::new(key.into())),
        verify,
    }
}

async fn save(f: &Fixture, url: &str, key: Option<&str>) -> String {
    f.service
        .save_connection(request(url, key, false))
        .await
        .unwrap()
        .connection_id
}

fn stored_key(f: &Fixture, id: &str) -> Option<String> {
    f.secrets
        .get(&key_secret(id))
        .unwrap()
        .map(|key| key.expose_secret().to_owned())
}

fn connections(f: &Fixture) -> Vec<AiConnection> {
    f.settings.get().unwrap().settings.ai.connections
}

fn model_ref(connection_id: &str) -> AiModelRef {
    AiModelRef {
        connection_id: connection_id.into(),
        model_id: "m".into(),
        name: "M".into(),
        context_length: Some(1000),
        output_length: None,
    }
}

async fn ok_server() -> MockHttp {
    mock_http(|request| {
        if request.method == "POST" {
            MockReply::json(200, CHAT)
        } else {
            MockReply::json(200, LIST)
        }
    })
    .await
}

fn write_catalog(f: &Fixture, fetched_at: i64) {
    let providers =
        crate::ai::catalog::trim(crate::ai::catalog::tests::FIXTURE.as_bytes()).unwrap();
    crate::ai::catalog::write_cache(
        &f.directory.path().join("ai-cache"),
        &CatalogData::new(Some("\"v1\"".into()), fetched_at, providers),
    )
    .unwrap();
}

// ------------------------------------------------------------ connections

#[tokio::test]
async fn saving_a_connection_stores_it_and_its_key_separately() {
    let f = fixture();
    let saved = f
        .service
        .save_connection(request(
            "https://api.example.com",
            Some(&format!("  {SECRET} ")),
            false,
        ))
        .await
        .unwrap();
    assert!(saved.models.is_none());
    let id = saved.connection_id.clone();
    assert!(Uuid::parse_str(&id).is_ok());
    assert_eq!(stored_key(&f, &id).as_deref(), Some(SECRET));
    assert_eq!(
        connections(&f),
        vec![AiConnection {
            id: id.clone(),
            provider_id: "custom".into(),
            name: "Mine".into(),
            protocol: AiProtocol::OpenaiCompatible,
            base_url: "https://api.example.com/v1".into(),
            use_proxy: false,
        }]
    );
    assert_eq!(saved.state.connections.len(), 1);
    assert!(saved.state.connections[0].has_key);
    // It survives a restart, and the key is not in the settings file.
    let reopened = SettingsStore::open(f.directory.path()).unwrap();
    assert_eq!(reopened.get().unwrap().settings.ai.connections.len(), 1);
    let file = std::fs::read_to_string(f.directory.path().join("settings.json")).unwrap();
    assert!(!file.contains(SECRET));
}

#[tokio::test]
async fn saving_with_verify_returns_the_models_and_uses_the_submitted_key() {
    let f = fixture();
    let server = ok_server().await;
    let saved = f
        .service
        .save_connection(request(&server.url(), Some(SECRET), true))
        .await
        .unwrap();
    let models = saved.models.unwrap();
    assert_eq!(models.len(), 3);
    assert!(models.iter().all(|m| m.source == AiModelSource::Provider));
    assert_eq!(server.last().target, "/v1/models");
    assert_eq!(
        server.last().header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
    assert_eq!(connections(&f).len(), 1);
}

#[tokio::test]
async fn a_failed_verification_persists_nothing() {
    let f = fixture();
    let server = mock_http(|_| MockReply::json(401, r#"{"error":"nope"}"#)).await;
    // New connection.
    let error = f
        .service
        .save_connection(request(&server.url(), Some(SECRET), true))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AiKeyRejected);
    assert!(connections(&f).is_empty());
    assert!(!f.directory.path().join("ai-cache").exists());
    assert_eq!(f.settings.get().unwrap().revision, 0);
    assert!(f.secrets.get(PROXY_PASSWORD_SECRET).unwrap().is_none());

    // An edit changes neither the connection nor the key.
    let good = ok_server().await;
    let id = save(&f, &good.url(), Some("old-key")).await;
    let before = connections(&f);
    let mut edit = request(&server.url(), Some("new-key"), true);
    edit.id = Some(id.clone());
    edit.name = "Renamed".into();
    assert_eq!(
        f.service.save_connection(edit).await.unwrap_err().code,
        ErrorCode::AiKeyRejected
    );
    assert_eq!(connections(&f), before);
    assert_eq!(stored_key(&f, &id).as_deref(), Some("old-key"));
}

#[tokio::test]
async fn editing_keeps_the_stored_key_unless_told_otherwise() {
    let f = fixture();
    let server = ok_server().await;
    let id = save(&f, &server.url(), Some("first-key")).await;

    // `apiKey` null: verify with the stored key; the key stays.
    let mut edit = request(&server.url(), None, true);
    edit.id = Some(id.clone());
    edit.name = "Second".into();
    let saved = f.service.save_connection(edit).await.unwrap();
    assert_eq!(saved.connection_id, id);
    assert_eq!(
        server.last().header("authorization"),
        Some("Bearer first-key")
    );
    assert_eq!(stored_key(&f, &id).as_deref(), Some("first-key"));
    assert_eq!(connections(&f).len(), 1);
    assert_eq!(connections(&f)[0].name, "Second");

    // A new key replaces it.
    let mut edit = request(&server.url(), Some("second-key"), false);
    edit.id = Some(id.clone());
    f.service.save_connection(edit).await.unwrap();
    assert_eq!(stored_key(&f, &id).as_deref(), Some("second-key"));

    // Empty removes it, and the request goes out without credentials.
    let mut edit = request(&server.url(), Some("  "), true);
    edit.id = Some(id.clone());
    let saved = f.service.save_connection(edit).await.unwrap();
    assert_eq!(stored_key(&f, &id), None);
    assert!(!saved.state.connections[0].has_key);
    assert_eq!(server.last().header("authorization"), None);
}

#[tokio::test]
async fn input_is_validated() {
    let f = fixture();
    let invalid = |field: &str| (ErrorCode::Validation, Some(field.to_owned()));
    let outcome = |error: AppError| (error.code, error.field);
    let mut bad = request("https://x.example", None, false);
    bad.name = "  ".into();
    assert_eq!(
        outcome(f.service.save_connection(bad).await.unwrap_err()),
        invalid("name")
    );
    let mut bad = request("https://x.example", None, false);
    bad.base_url = "https://user:pw@x.example".into();
    assert_eq!(
        outcome(f.service.save_connection(bad).await.unwrap_err()),
        invalid("baseUrl")
    );
    let mut bad = request("https://x.example", None, false);
    bad.provider_id = "Bad/Id".into();
    assert_eq!(
        outcome(f.service.save_connection(bad).await.unwrap_err()),
        invalid("providerId")
    );
    let bad = request("https://x.example", Some("key\nwith newline"), false);
    assert_eq!(
        outcome(f.service.save_connection(bad).await.unwrap_err()),
        invalid("apiKey")
    );
    let mut bad = request("https://x.example", None, false);
    bad.id = Some("not-a-uuid".into());
    assert_eq!(
        outcome(f.service.save_connection(bad).await.unwrap_err()),
        invalid("id")
    );
    let mut missing = request("https://x.example", None, false);
    missing.id = Some(Uuid::new_v4().to_string());
    assert_eq!(
        f.service.save_connection(missing).await.unwrap_err().code,
        ErrorCode::AiNotConfigured
    );
    assert!(connections(&f).is_empty());

    for _ in 0..MAX_CONNECTIONS {
        save(&f, "https://x.example", None).await;
    }
    assert_eq!(
        outcome(
            f.service
                .save_connection(request("https://x.example", None, false))
                .await
                .unwrap_err()
        ),
        invalid("connections")
    );
    assert_eq!(connections(&f).len(), MAX_CONNECTIONS);
}

#[tokio::test]
async fn removing_a_connection_clears_its_key_and_a_default_model_on_it() {
    let f = fixture();
    let (a, b) = (
        save(&f, "https://a.example", Some("key-a")).await,
        save(&f, "https://b.example", Some("key-b")).await,
    );
    f.service
        .set_default_model(Some(model_ref(&a)))
        .await
        .unwrap();
    let state = f.service.remove_connection(&b).await.unwrap();
    assert_eq!(
        state
            .default_model
            .as_ref()
            .map(|m| m.connection_id.as_str()),
        Some(a.as_str())
    );
    assert_eq!(stored_key(&f, &b), None);
    assert_eq!(stored_key(&f, &a).as_deref(), Some("key-a"));

    let state = f.service.remove_connection(&a).await.unwrap();
    assert!(state.connections.is_empty());
    assert_eq!(state.default_model, None);
    assert_eq!(stored_key(&f, &a), None);
    assert_eq!(f.settings.get().unwrap().settings.ai.default_model, None);
    // Removing again is harmless; a malformed id is not accepted.
    f.service.remove_connection(&a).await.unwrap();
    assert_eq!(
        f.service.remove_connection("../x").await.unwrap_err().code,
        ErrorCode::Validation
    );
}

#[tokio::test]
async fn the_default_model_needs_an_existing_connection() {
    let f = fixture();
    assert_eq!(
        f.service
            .set_default_model(Some(model_ref(&Uuid::new_v4().to_string())))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AiNotConfigured
    );
    let id = save(&f, "https://a.example", None).await;
    let state = f
        .service
        .set_default_model(Some(model_ref(&id)))
        .await
        .unwrap();
    assert_eq!(state.default_model, Some(model_ref(&id)));
    let mut bad = model_ref(&id);
    bad.model_id = " x".into();
    assert_eq!(
        f.service
            .set_default_model(Some(bad))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    assert_eq!(
        f.service
            .set_default_model(None)
            .await
            .unwrap()
            .default_model,
        None
    );
}

#[tokio::test]
async fn the_proxy_route_and_password_are_kept_apart() {
    let f = fixture();
    let route = ServiceRoute::Proxy {
        proxy_kind: ProxyKind::Socks5,
        host: "proxy.example".into(),
        port: 1080,
        username: "me".into(),
    };
    let state = f.service.set_route(route.clone()).await.unwrap();
    assert_eq!(state.route, route);
    assert!(!state.proxy_password_set);
    assert!(
        f.service
            .set_proxy_password(SecretValue::new("pw-secret".into()))
            .await
            .unwrap()
            .proxy_password_set
    );
    assert!(f.secrets.get(PROXY_PASSWORD_SECRET).unwrap().is_some());
    let file = std::fs::read_to_string(f.directory.path().join("settings.json")).unwrap();
    assert!(!file.contains("pw-secret"));
    assert!(
        !f.service
            .remove_proxy_password()
            .await
            .unwrap()
            .proxy_password_set
    );

    let bad = ServiceRoute::Proxy {
        proxy_kind: ProxyKind::Socks5,
        host: String::new(),
        port: 1080,
        username: String::new(),
    };
    assert_eq!(
        f.service.set_route(bad).await.unwrap_err().code,
        ErrorCode::Validation
    );
    assert_eq!(
        f.service
            .set_proxy_password(SecretValue::new(String::new()))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    assert_eq!(
        f.service
            .set_route(ServiceRoute::Direct)
            .await
            .unwrap()
            .route,
        ServiceRoute::Direct
    );
}

// -------------------------------------------------------------- requests

#[tokio::test]
async fn model_lists_are_labelled_from_the_catalog() {
    let f = fixture();
    write_catalog(&f, now());
    let server = mock_http(|_| {
        MockReply::json(
            200,
            r#"{"data":[{"id":"gpt-a"},{"id":"embed"},{"id":"pic"},{"id":"unknown"},{"id":"nolimit","name":"Mine"}]}"#,
        )
    })
    .await;
    let mut known = request(&server.url(), Some("k"), false);
    known.provider_id = "openai".into();
    let id = f
        .service
        .save_connection(known)
        .await
        .unwrap()
        .connection_id;
    let models = f.service.list_models(&id).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    // The catalog says `embed` and `pic` do not output text.
    assert_eq!(ids, ["gpt-a", "nolimit", "unknown"]);
    assert_eq!(models[0].name, "GPT A");
    assert_eq!(models[0].context_length, Some(400_000));
    assert_eq!(models[0].output_length, Some(128_000));
    assert_eq!(models[0].tool_call, Some(true));
    assert_eq!(models[1].name, "Mine");
    assert_eq!(
        (
            models[2].name.as_str(),
            models[2].context_length,
            models[2].tool_call
        ),
        ("unknown", None, None)
    );
    assert!(models.iter().all(|m| m.source == AiModelSource::Provider));

    // A custom connection is never matched against the catalog.
    let custom = save(&f, &server.url(), Some("k")).await;
    let models = f.service.list_models(&custom).await.unwrap();
    assert_eq!(models.len(), 5);
    assert!(models.iter().all(|m| m.context_length.is_none()));
}

#[tokio::test]
async fn a_provider_without_a_list_falls_back_to_the_catalog() {
    let f = fixture();
    write_catalog(&f, now());
    let server = mock_http(|_| MockReply::status(404)).await;
    let mut known = request(&server.url(), None, false);
    known.provider_id = "openai".into();
    let id = f
        .service
        .save_connection(known)
        .await
        .unwrap()
        .connection_id;
    let models = f.service.list_models(&id).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-a", "nolimit"]);
    assert!(models.iter().all(|m| m.source == AiModelSource::Catalog));
    // Not an error when saving with verification either.
    let mut again = request(&server.url(), None, true);
    again.provider_id = "openai".into();
    assert_eq!(
        f.service
            .save_connection(again)
            .await
            .unwrap()
            .models
            .unwrap()
            .len(),
        2
    );

    // A provider the catalog does not know yields an empty list.
    let custom = save(&f, &server.url(), None).await;
    assert!(f.service.list_models(&custom).await.unwrap().is_empty());
    // Other errors stay errors.
    let denied = mock_http(|_| MockReply::status(403)).await;
    let id = save(&f, &denied.url(), Some("k")).await;
    assert_eq!(
        f.service.list_models(&id).await.unwrap_err().code,
        ErrorCode::AiKeyRejected
    );
}

#[tokio::test]
async fn testing_a_model_reports_the_latency_or_a_fixed_code() {
    let f = fixture();
    let server = ok_server().await;
    let id = save(&f, &server.url(), Some("k")).await;
    let result = f.service.test(&id, "gpt-a").await.unwrap();
    assert!(result.latency_ms < 10_000);
    assert_eq!(server.last().target, "/v1/chat/completions");
    assert_eq!(server.last().json()["model"], "gpt-a");

    assert_eq!(
        f.service
            .test(&Uuid::new_v4().to_string(), "m")
            .await
            .unwrap_err()
            .code,
        ErrorCode::AiNotConfigured
    );
    assert_eq!(
        f.service.test("nonsense", "m").await.unwrap_err().code,
        ErrorCode::AiNotConfigured
    );
    assert_eq!(
        f.service.test(&id, "  ").await.unwrap_err().code,
        ErrorCode::Validation
    );
    let missing = mock_http(|_| MockReply::status(404)).await;
    let id = save(&f, &missing.url(), None).await;
    assert_eq!(
        f.service.test(&id, "x").await.unwrap_err().code,
        ErrorCode::AiModelNotFound
    );
}

#[tokio::test]
async fn complete_uses_the_saved_connection() {
    let f = fixture();
    let server = ok_server().await;
    let id = save(&f, &server.url(), Some("k")).await;
    assert_eq!(
        f.service
            .complete(&id, "m", Some("sys"), "hi", Some(50))
            .await
            .unwrap(),
        "OK"
    );
    assert_eq!(server.last().json()["max_tokens"], 50);
}

#[tokio::test]
async fn the_proxy_is_used_only_when_the_connection_asks_for_it() {
    let f = fixture();
    let server = ok_server().await;
    let (proxy_port, targets) = fake_http_connect(server.port).await;
    f.service
        .set_route(ServiceRoute::Proxy {
            proxy_kind: ProxyKind::HttpConnect,
            host: "127.0.0.1".into(),
            port: proxy_port,
            username: String::new(),
        })
        .await
        .unwrap();

    // Direct: the proxy hears nothing.
    let direct = save(&f, &server.url(), None).await;
    f.service.list_models(&direct).await.unwrap();
    assert!(targets.lock().unwrap().is_empty());

    let mut proxied = request("http://models.invalid:8080", None, false);
    proxied.use_proxy = true;
    let id = f
        .service
        .save_connection(proxied)
        .await
        .unwrap()
        .connection_id;
    assert_eq!(f.service.list_models(&id).await.unwrap().len(), 3);
    assert!(targets.lock().unwrap()[0].starts_with("CONNECT models.invalid:8080"));

    // A dead proxy fails; it never falls back to a direct connection.
    let before = server.count();
    f.service
        .set_route(ServiceRoute::Proxy {
            proxy_kind: ProxyKind::Socks5,
            host: "127.0.0.1".into(),
            port: crate::net::test_support::dead_port().await,
            username: String::new(),
        })
        .await
        .unwrap();
    let mut proxied = request(&server.url(), None, false);
    proxied.use_proxy = true;
    let id = f
        .service
        .save_connection(proxied)
        .await
        .unwrap()
        .connection_id;
    assert_eq!(
        f.service.list_models(&id).await.unwrap_err().code,
        ErrorCode::AiProxy
    );
    assert_eq!(server.count(), before);
}

// --------------------------------------------------------------- catalog

#[tokio::test]
async fn without_a_cache_the_catalog_is_the_built_ins_and_stale() {
    let f = fixture();
    let catalog = f.service.catalog(false).await.unwrap();
    assert_eq!(catalog.providers.len(), crate::ai::catalog::BUILT_INS.len());
    assert!(catalog.stale);
    assert_eq!(catalog.updated_at, None);
    assert_eq!(catalog.refresh_error, None);
    assert!(
        !f.directory.path().join("ai-cache").exists(),
        "reading creates nothing"
    );
}

#[tokio::test]
async fn refreshing_caches_the_catalog_and_revalidates_with_the_etag() {
    let server = mock_http(|request| {
        if request.header("if-none-match") == Some("\"v1\"") {
            MockReply::status(304)
        } else {
            MockReply::json(200, crate::ai::catalog::tests::FIXTURE).header("ETag", "\"v1\"")
        }
    })
    .await;
    let f = fixture_with(&format!("{}/api.json", server.url()), &server.url());
    let first = f.service.catalog(true).await.unwrap();
    assert_eq!(first.refresh_error, None);
    assert!(!first.stale);
    assert!(first.updated_at.is_some());
    assert!(
        first
            .providers
            .iter()
            .any(|p| p.id == "acme" && !p.built_in)
    );
    assert!(f.directory.path().join("ai-cache/catalog.json").exists());

    // 304: the data stays, the clock restarts.
    write_catalog(&f, now() - 3 * STALE_AFTER);
    *f.service.catalog_cache.lock().unwrap() = None;
    assert!(f.service.catalog(false).await.unwrap().stale);
    let second = f.service.catalog(true).await.unwrap();
    assert_eq!(second.refresh_error, None);
    assert!(!second.stale);
    assert!(second.providers.iter().any(|p| p.id == "acme"));
    assert_eq!(server.last().header("if-none-match"), Some("\"v1\""));

    // A new run reads the cache without the network.
    let before = server.count();
    let again = Arc::new(
        AiService::new(f.settings.clone(), f.secrets.clone(), f.directory.path())
            .with_endpoints(&format!("{}/api.json", server.url()), &server.url()),
    );
    let cached = again.catalog(false).await.unwrap();
    assert!(cached.providers.iter().any(|p| p.id == "acme"));
    assert!(!cached.stale);
    assert_eq!(server.count(), before);
}

const STALE_AFTER: i64 = crate::ai::catalog::STALE_AFTER_SECONDS;

#[tokio::test]
async fn a_failed_refresh_keeps_the_old_cache() {
    let server = mock_http(|_| MockReply::status(503)).await;
    let f = fixture_with(&format!("{}/api.json", server.url()), &server.url());
    write_catalog(&f, now() - 2 * STALE_AFTER);
    let result = f.service.catalog(true).await.unwrap();
    assert_eq!(result.refresh_error, Some(ErrorCode::AiUnavailable));
    assert!(result.stale);
    assert!(result.providers.iter().any(|p| p.id == "acme"));
    // The file is untouched.
    assert!(crate::ai::catalog::read_cache(&f.directory.path().join("ai-cache")).is_some());

    // No cache: the built-ins, and the error is reported.
    let f = fixture_with(&format!("{}/api.json", server.url()), &server.url());
    let result = f.service.catalog(true).await.unwrap();
    assert_eq!(result.providers.len(), crate::ai::catalog::BUILT_INS.len());
    assert_eq!(result.refresh_error, Some(ErrorCode::AiUnavailable));
    assert!(!f.directory.path().join("ai-cache/catalog.json").exists());

    // A damaged answer is a failure as well.
    let junk = mock_http(|_| MockReply::json(200, "not json")).await;
    let f = fixture_with(&format!("{}/api.json", junk.url()), &junk.url());
    write_catalog(&f, now());
    let result = f.service.catalog(true).await.unwrap();
    assert_eq!(result.refresh_error, Some(ErrorCode::AiInvalidResponse));
    assert!(result.providers.iter().any(|p| p.id == "acme"));
}

#[tokio::test]
async fn the_catalog_goes_through_the_ai_proxy_when_one_is_set() {
    let server = mock_http(|_| MockReply::json(200, crate::ai::catalog::tests::FIXTURE)).await;
    let (proxy_port, targets) = fake_http_connect(server.port).await;
    let f = fixture_with(
        "http://catalog.invalid:81/api.json",
        "http://catalog.invalid:81",
    );
    f.service
        .set_route(ServiceRoute::Proxy {
            proxy_kind: ProxyKind::HttpConnect,
            host: "127.0.0.1".into(),
            port: proxy_port,
            username: String::new(),
        })
        .await
        .unwrap();
    assert_eq!(f.service.catalog(true).await.unwrap().refresh_error, None);
    assert!(targets.lock().unwrap()[0].starts_with("CONNECT catalog.invalid:81"));
}

// ----------------------------------------------------------------- icons

#[tokio::test]
async fn icons_are_fetched_once_cached_and_failures_remembered() {
    let good = crate::ai::icons::tests::GOOD;
    let server = mock_http(move |request| {
        if request.target == "/acme.svg" {
            MockReply::json(200, good)
        } else {
            MockReply::status(404)
        }
    })
    .await;
    let f = fixture_with("http://127.0.0.1:1/api.json", &server.url());
    let icon = f.service.provider_icon("acme").await.unwrap().unwrap();
    assert_eq!(icon.mime_type, "image/svg+xml");
    assert_eq!(server.count(), 1);
    f.service.provider_icon("acme").await.unwrap().unwrap();
    assert_eq!(server.count(), 1, "served from disk");
    // Still cached for a new run.
    let again = Arc::new(
        AiService::new(f.settings.clone(), f.secrets.clone(), f.directory.path())
            .with_endpoints("http://127.0.0.1:1/api.json", &server.url()),
    );
    assert!(again.provider_icon("acme").await.unwrap().is_some());
    assert_eq!(server.count(), 1);

    assert_eq!(f.service.provider_icon("nothing").await.unwrap(), None);
    assert_eq!(server.count(), 2);
    assert_eq!(f.service.provider_icon("nothing").await.unwrap(), None);
    assert_eq!(server.count(), 2, "a failure is not asked for again");
    // But it is only remembered in memory.
    let fresh = Arc::new(
        AiService::new(f.settings.clone(), f.secrets.clone(), f.directory.path())
            .with_endpoints("http://127.0.0.1:1/api.json", &server.url()),
    );
    assert_eq!(fresh.provider_icon("nothing").await.unwrap(), None);
    assert_eq!(server.count(), 3);

    for bad in ["", "../x", "A", "a/b"] {
        assert_eq!(
            f.service.provider_icon(bad).await.unwrap_err().code,
            ErrorCode::Validation
        );
    }
    assert_eq!(server.count(), 3);
}

// --------------------------------------------------------------- secrets

#[tokio::test]
async fn keys_never_appear_in_debug_output_errors_or_state() {
    let f = fixture();
    let mut save_request = request("https://api.example.com", Some(SECRET), false);
    assert!(!format!("{save_request:?}").contains(SECRET));
    let id = f
        .service
        .save_connection(save_request.clone())
        .await
        .unwrap()
        .connection_id;
    save_request.id = Some(id.clone());

    let state = f.service.state().await.unwrap();
    assert!(!format!("{state:?}").contains(SECRET));
    assert!(!serde_json::to_string(&state).unwrap().contains(SECRET));
    assert!(!format!("{:?}", f.settings.get().unwrap()).contains(SECRET));

    // The same for every error a request with this key can produce.
    for (status, code) in [
        (401, ErrorCode::AiKeyRejected),
        (402, ErrorCode::AiQuotaExceeded),
        (500, ErrorCode::AiUnavailable),
    ] {
        let echo = mock_http(move |request| {
            // A server that echoes the credentials back.
            MockReply::json(
                status,
                &format!(
                    "{{\"echo\":\"{}\"}}",
                    request.header("authorization").unwrap_or("")
                ),
            )
        })
        .await;
        let mut edit = save_request.clone();
        edit.base_url = echo.url();
        edit.verify = true;
        let error = f.service.save_connection(edit).await.unwrap_err();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains(SECRET));
        assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
        assert!(!error.to_string().contains(SECRET));
    }
    // A dead address too.
    let mut edit = save_request;
    edit.base_url = format!(
        "http://127.0.0.1:{}",
        crate::net::test_support::dead_port().await
    );
    edit.verify = true;
    let error = f.service.save_connection(edit).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AiNetwork);
    assert!(!format!("{error:?}").contains(SECRET));
}

// ------------------------------------------------------------ translation

fn model_with_output(connection_id: &str, model_id: &str, output: Option<u32>) -> AiModelRef {
    AiModelRef {
        model_id: model_id.into(),
        output_length: output,
        ..model_ref(connection_id)
    }
}

#[tokio::test]
async fn the_translation_model_needs_a_connection_and_is_cleared_with_it() {
    let f = fixture();
    assert_eq!(
        f.service
            .set_translation_model(Some(model_ref(&Uuid::new_v4().to_string())))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AiNotConfigured
    );
    let (a, b) = (
        save(&f, "https://a.example", None).await,
        save(&f, "https://b.example", None).await,
    );
    let mut bad = model_ref(&a);
    bad.model_id = " x".into();
    assert_eq!(
        f.service
            .set_translation_model(Some(bad))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Validation
    );
    f.service
        .set_default_model(Some(model_ref(&a)))
        .await
        .unwrap();
    let state = f
        .service
        .set_translation_model(Some(model_ref(&b)))
        .await
        .unwrap();
    assert_eq!(state.translation_model, Some(model_ref(&b)));
    assert_eq!(state.default_model, Some(model_ref(&a)));

    let state = f.service.remove_connection(&b).await.unwrap();
    assert_eq!(state.translation_model, None);
    assert_eq!(state.default_model, Some(model_ref(&a)));
    assert_eq!(
        f.settings.get().unwrap().settings.ai.translation_model,
        None
    );
    f.service
        .set_translation_model(Some(model_ref(&a)))
        .await
        .unwrap();
    assert_eq!(
        f.service
            .set_translation_model(None)
            .await
            .unwrap()
            .translation_model,
        None
    );
}

#[tokio::test]
async fn translation_uses_its_model_else_the_default_else_nothing() {
    let f = fixture();
    let (a, b) = (
        save(&f, "https://a.example", Some("key-a")).await,
        save(&f, "https://b.example", Some("key-b")).await,
    );
    // Nothing chosen: not configured, and no binding.
    assert_eq!(f.service.translation_binding().await.unwrap(), None);
    assert_eq!(
        f.service.translation_target().await.unwrap_err().code,
        ErrorCode::AiNotConfigured
    );

    // The default model is what translation follows.
    f.service
        .set_default_model(Some(model_with_output(&a, "default-model", Some(4000))))
        .await
        .unwrap();
    let target = f.service.translation_target().await.unwrap();
    assert_eq!(target.binding.connection_id, a);
    assert_eq!(target.binding.base_url, "https://a.example/v1");
    assert_eq!(target.model_id, "default-model");
    assert_eq!(target.output_length, Some(4000));
    assert_eq!(target.key.expose_secret(), "key-a");
    assert_eq!(
        f.service.translation_binding().await.unwrap(),
        Some(target.binding.clone())
    );

    // Its own model wins, with its own connection and key.
    f.service
        .set_translation_model(Some(model_with_output(&b, "translate-model", None)))
        .await
        .unwrap();
    let target = f.service.translation_target().await.unwrap();
    assert_eq!(target.binding.connection_id, b);
    assert_eq!(target.model_id, "translate-model");
    assert_eq!(target.key.expose_secret(), "key-b");

    // Clearing it follows the default again.
    f.service.set_translation_model(None).await.unwrap();
    assert_eq!(
        f.service
            .translation_target()
            .await
            .unwrap()
            .binding
            .connection_id,
        a
    );
}

#[tokio::test]
async fn the_translation_target_uses_the_ai_route_and_hides_its_key() {
    let f = fixture();
    let id = save(&f, "https://a.example", Some(SECRET)).await;
    f.service
        .set_default_model(Some(model_ref(&id)))
        .await
        .unwrap();
    let target = f.service.translation_target().await.unwrap();
    assert!(target.route.proxy.is_none());
    assert!(!format!("{target:?}").contains(SECRET));

    // A key that was never set resolves with an empty one (local servers).
    let keyless = save(&f, "https://c.example", None).await;
    f.service
        .set_translation_model(Some(model_ref(&keyless)))
        .await
        .unwrap();
    assert_eq!(
        f.service
            .translation_target()
            .await
            .unwrap()
            .key
            .expose_secret(),
        ""
    );
}
