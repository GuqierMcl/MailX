//! AI provider connections for the host: settings, keys, model lists, test
//! calls, the provider catalog and icons. It needs only the settings store,
//! the secret store and a cache directory, so it works without the mail
//! store. Keys and the proxy password live in the `SecretStore` and are never
//! returned; provider answers never reach the interface as text.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use url::Url;
use uuid::Uuid;

use crate::{
    AppError, ErrorCode,
    ai::{
        AiCatalog, AiConnection, AiModel, AiModelRef, AiProtocol, AiProviderIcon, AiSettings,
        AiTestResult, catalog,
        client::{self, Endpoint},
        icons, models,
        validation::{
            CUSTOM_PROVIDER_ID, MAX_CONNECTIONS, normalize_base_url, normalize_key, normalize_name,
            valid_model_id, valid_provider_id,
        },
    },
    net::NetRoute,
    secrets::{SecretStore, SecretValue},
    settings::{ServiceRoute, SettingsStore},
};

const PROXY_PASSWORD_SECRET: &str = "mailx/ai/proxy-password";
/// Next to the app's other caches (`image-cache`) in the data directory.
const CACHE_DIRECTORY: &str = "ai-cache";
const ICON_DIRECTORY: &str = "icons";

fn key_secret(connection_id: &str) -> String {
    format!("mailx/ai/{connection_id}/api-key")
}

/// A connection as the interface sees it: no key, only whether one is set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiConnectionState {
    #[serde(flatten)]
    #[ts(flatten)]
    pub connection: AiConnection,
    pub has_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiState {
    pub connections: Vec<AiConnectionState>,
    pub default_model: Option<AiModelRef>,
    pub route: ServiceRoute,
    pub proxy_password_set: bool,
}

/// Creates (`id` null) or changes a connection. `api_key` null keeps the
/// stored key; an empty string removes it. With `verify`, nothing is saved
/// unless the provider answers the model list request with these values.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAiConnection {
    #[serde(default)]
    pub id: Option<String>,
    pub provider_id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    pub use_proxy: bool,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub api_key: Option<SecretValue>,
    pub verify: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SavedAiConnection {
    pub state: AiState,
    /// The saved connection, which is new when the request had no `id`.
    pub connection_id: String,
    /// The provider's models when `verify` was set.
    pub models: Option<Vec<AiModel>>,
}

/// What a request needs from a connection, whether saved or about to be.
struct Target {
    provider_id: String,
    protocol: AiProtocol,
    base_url: String,
    use_proxy: bool,
}

impl Target {
    fn of(connection: &AiConnection) -> Self {
        Self {
            provider_id: connection.provider_id.clone(),
            protocol: connection.protocol,
            base_url: connection.base_url.clone(),
            use_proxy: connection.use_proxy,
        }
    }
}

pub struct AiService {
    settings: Arc<SettingsStore>,
    secrets: Arc<dyn SecretStore>,
    cache_directory: PathBuf,
    catalog_url: Url,
    icon_base_url: String,
    /// The catalog file, read once and replaced when a refresh writes it.
    catalog_cache: Mutex<Option<Arc<catalog::CatalogData>>>,
    /// One refresh at a time.
    refresh: tokio::sync::Mutex<()>,
    /// Icons that failed in this run; never persisted.
    failed_icons: Mutex<HashSet<String>>,
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}

fn not_configured() -> AppError {
    AppError::new(ErrorCode::AiNotConfigured)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl AiService {
    pub fn new(
        settings: Arc<SettingsStore>,
        secrets: Arc<dyn SecretStore>,
        data_directory: &Path,
    ) -> Self {
        Self {
            settings,
            secrets,
            cache_directory: data_directory.join(CACHE_DIRECTORY),
            catalog_url: Url::parse(catalog::CATALOG_URL).expect("constant URL"),
            icon_base_url: icons::ICON_BASE_URL.to_owned(),
            catalog_cache: Mutex::new(None),
            refresh: tokio::sync::Mutex::new(()),
            failed_icons: Mutex::default(),
        }
    }

    /// Points the catalog and icon downloads at a local server.
    #[cfg(test)]
    pub(crate) fn with_endpoints(mut self, catalog_url: &str, icon_base_url: &str) -> Self {
        self.catalog_url = Url::parse(catalog_url).unwrap();
        self.icon_base_url = icon_base_url.to_owned();
        self
    }

    async fn blocking<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Self) -> Result<T, AppError> + Send + 'static,
    ) -> Result<T, AppError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || work(&this))
            .await
            .map_err(|_| internal())?
    }

    // ------------------------------------------------------------- state

    fn ai_settings(&self) -> Result<AiSettings, AppError> {
        Ok(self.settings.get()?.settings.ai)
    }

    fn state_now(&self) -> Result<AiState, AppError> {
        let ai = self.ai_settings()?;
        let connections = ai
            .connections
            .into_iter()
            .map(|connection| {
                let has_key = self.secrets.get(&key_secret(&connection.id))?.is_some();
                Ok(AiConnectionState {
                    connection,
                    has_key,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(AiState {
            connections,
            default_model: ai.default_model,
            route: ai.route,
            proxy_password_set: self.secrets.get(PROXY_PASSWORD_SECRET)?.is_some(),
        })
    }

    pub async fn state(self: &Arc<Self>) -> Result<AiState, AppError> {
        self.blocking(|this| this.state_now()).await
    }

    /// The route for a request: through the AI proxy when it is wanted and
    /// set, otherwise direct. A proxy password that cannot be read fails the
    /// request; it never goes out unsigned.
    async fn route(
        self: &Arc<Self>,
        ai_route: &ServiceRoute,
        use_proxy: bool,
    ) -> Result<NetRoute, AppError> {
        if !use_proxy {
            return Ok(NetRoute::default());
        }
        let password = if ai_route.needs_password() {
            let secrets = self.secrets.clone();
            self.blocking(move |_| Ok(secrets.get(PROXY_PASSWORD_SECRET)?))
                .await?
        } else {
            None
        };
        Ok(ai_route.net_route(password))
    }

    // ------------------------------------------------------ connections

    fn parse_id(id: &str) -> Result<String, AppError> {
        Uuid::parse_str(id)
            .map(|id| id.to_string())
            .map_err(|_| AppError::validation("id"))
    }

    pub async fn save_connection(
        self: &Arc<Self>,
        request: SaveAiConnection,
    ) -> Result<SavedAiConnection, AppError> {
        let provider_id = if valid_provider_id(&request.provider_id) {
            request.provider_id.clone()
        } else {
            return Err(AppError::validation("providerId"));
        };
        let name = normalize_name(&request.name).ok_or_else(|| AppError::validation("name"))?;
        let base_url =
            normalize_base_url(&request.base_url).ok_or_else(|| AppError::validation("baseUrl"))?;
        let new_key = request
            .api_key
            .as_ref()
            .map(|key| {
                normalize_key(key.expose_secret()).ok_or_else(|| AppError::validation("apiKey"))
            })
            .transpose()?;
        let existing_id = request.id.as_deref().map(Self::parse_id).transpose()?;

        let ai = self.blocking(|this| this.ai_settings()).await?;
        match &existing_id {
            Some(id) if !ai.connections.iter().any(|c| &c.id == id) => {
                return Err(not_configured());
            }
            None if ai.connections.len() >= MAX_CONNECTIONS => {
                return Err(AppError::validation("connections"));
            }
            _ => {}
        }
        let id = existing_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let connection = AiConnection {
            id: id.clone(),
            provider_id,
            name,
            protocol: request.protocol,
            base_url,
            use_proxy: request.use_proxy,
        };

        // Check the provider first, with the values that would be saved.
        let models = if request.verify {
            let key = match (&new_key, &existing_id) {
                (Some(key), _) => key.clone(),
                (None, Some(id)) => self.stored_key(id).await?.unwrap_or_default(),
                (None, None) => String::new(),
            };
            Some(
                self.list_models_for(&Target::of(&connection), &key, &ai.route)
                    .await?,
            )
        } else {
            None
        };

        let is_new = existing_id.is_none();
        if let Some(key) = new_key {
            let name = key_secret(&id);
            let secrets = self.secrets.clone();
            self.blocking(move |_| {
                if key.is_empty() {
                    secrets.delete(&name)?;
                } else {
                    secrets.set(&name, &SecretValue::new(key))?;
                }
                Ok(())
            })
            .await?;
        }
        let saved = connection.clone();
        let stored = self
            .blocking(move |this| {
                this.settings.update_ai(|ai| {
                    match ai.connections.iter_mut().find(|c| c.id == saved.id) {
                        Some(slot) => *slot = saved,
                        None => {
                            if ai.connections.len() >= MAX_CONNECTIONS {
                                return Err(AppError::validation("connections"));
                            }
                            ai.connections.push(saved);
                        }
                    }
                    Ok(())
                })
            })
            .await;
        if let Err(error) = stored {
            if is_new {
                // Nothing refers to the new key; do not leave it behind.
                let (secrets, name) = (self.secrets.clone(), key_secret(&id));
                let _ = self.blocking(move |_| Ok(secrets.delete(&name)?)).await;
            }
            return Err(error);
        }
        Ok(SavedAiConnection {
            state: self.state().await?,
            connection_id: id,
            models,
        })
    }

    pub async fn remove_connection(self: &Arc<Self>, id: &str) -> Result<AiState, AppError> {
        let id = Self::parse_id(id)?;
        self.blocking(move |this| {
            // The key first: if the settings write then fails, the connection
            // is still there and removing it again finishes the job.
            this.secrets.delete(&key_secret(&id))?;
            this.settings.update_ai(|ai| {
                ai.connections.retain(|connection| connection.id != id);
                if ai
                    .default_model
                    .as_ref()
                    .is_some_and(|model| model.connection_id == id)
                {
                    ai.default_model = None;
                }
                Ok(())
            })?;
            this.state_now()
        })
        .await
    }

    pub async fn set_default_model(
        self: &Arc<Self>,
        model: Option<AiModelRef>,
    ) -> Result<AiState, AppError> {
        let model = model
            .map(|model| {
                model
                    .normalized()
                    .ok_or_else(|| AppError::validation("model"))
            })
            .transpose()?;
        self.blocking(move |this| {
            this.settings.update_ai(|ai| {
                if let Some(model) = &model
                    && !ai.connections.iter().any(|c| c.id == model.connection_id)
                {
                    return Err(not_configured());
                }
                ai.default_model = model;
                Ok(())
            })?;
            this.state_now()
        })
        .await
    }

    pub async fn set_route(self: &Arc<Self>, route: ServiceRoute) -> Result<AiState, AppError> {
        route.validate("route")?;
        self.blocking(move |this| {
            this.settings.update_ai(|ai| {
                ai.route = route;
                Ok(())
            })?;
            this.state_now()
        })
        .await
    }

    pub async fn set_proxy_password(
        self: &Arc<Self>,
        password: SecretValue,
    ) -> Result<AiState, AppError> {
        if !super::translation::valid_proxy_password(&password) {
            return Err(AppError::validation("route.password"));
        }
        self.blocking(move |this| {
            this.secrets.set(PROXY_PASSWORD_SECRET, &password)?;
            this.state_now()
        })
        .await
    }

    pub async fn remove_proxy_password(self: &Arc<Self>) -> Result<AiState, AppError> {
        self.blocking(|this| {
            this.secrets.delete(PROXY_PASSWORD_SECRET)?;
            this.state_now()
        })
        .await
    }

    // ----------------------------------------------------------- requests

    async fn stored_key(self: &Arc<Self>, connection_id: &str) -> Result<Option<String>, AppError> {
        let name = key_secret(connection_id);
        Ok(self
            .blocking(move |this| Ok(this.secrets.get(&name)?))
            .await?
            .map(|key| key.expose_secret().to_owned()))
    }

    /// The saved connection, its key and the AI proxy settings.
    async fn load(
        self: &Arc<Self>,
        connection_id: &str,
    ) -> Result<(AiConnection, String, ServiceRoute), AppError> {
        let id = Self::parse_id(connection_id).map_err(|_| not_configured())?;
        self.blocking(move |this| {
            let ai = this.ai_settings()?;
            let connection = ai
                .connections
                .into_iter()
                .find(|connection| connection.id == id)
                .ok_or_else(not_configured)?;
            let key = this
                .secrets
                .get(&key_secret(&id))?
                .map(|key| key.expose_secret().to_owned())
                .unwrap_or_default();
            Ok((connection, key, ai.route))
        })
        .await
    }

    fn catalog_data(&self) -> Option<Arc<catalog::CatalogData>> {
        let mut slot = self
            .catalog_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = catalog::read_cache(&self.cache_directory).map(Arc::new);
        }
        slot.clone()
    }

    /// The provider's model list, labelled from the catalog; the catalog's own
    /// models when the provider has no list.
    async fn list_models_for(
        self: &Arc<Self>,
        target: &Target,
        key: &str,
        ai_route: &ServiceRoute,
    ) -> Result<Vec<AiModel>, AppError> {
        let route = self.route(ai_route, target.use_proxy).await?;
        let endpoint = Endpoint {
            protocol: target.protocol,
            base_url: &target.base_url,
            key: Some(key),
            route: &route,
        };
        let listed = client::list_models(&endpoint)
            .await
            .map_err(AppError::new)?;
        let provider_id = target.provider_id.clone();
        self.blocking(move |this| {
            let data = this.catalog_data();
            let provider = data
                .as_deref()
                .and_then(|data| data.provider(&provider_id))
                .filter(|_| provider_id != CUSTOM_PROVIDER_ID);
            Ok(match listed {
                Some(listed) => models::enrich(listed, provider),
                None => models::from_catalog(provider),
            })
        })
        .await
    }

    pub async fn list_models(
        self: &Arc<Self>,
        connection_id: &str,
    ) -> Result<Vec<AiModel>, AppError> {
        let (connection, key, route) = self.load(connection_id).await?;
        self.list_models_for(&Target::of(&connection), &key, &route)
            .await
    }

    /// A fixed English prompt to one model; no mail text.
    pub async fn test(
        self: &Arc<Self>,
        connection_id: &str,
        model_id: &str,
    ) -> Result<AiTestResult, AppError> {
        if !valid_model_id(model_id) {
            return Err(AppError::validation("modelId"));
        }
        let (connection, key, ai_route) = self.load(connection_id).await?;
        let route = self.route(&ai_route, connection.use_proxy).await?;
        let endpoint = Endpoint {
            protocol: connection.protocol,
            base_url: &connection.base_url,
            key: Some(&key),
            route: &route,
        };
        let latency_ms = client::test(&endpoint, model_id)
            .await
            .map_err(AppError::new)?;
        Ok(AiTestResult { latency_ms })
    }

    /// One non-streaming completion on a saved connection: an optional system
    /// prompt, one user message, the reply text. What later features (mail
    /// translation) build on.
    pub async fn complete(
        self: &Arc<Self>,
        connection_id: &str,
        model_id: &str,
        system: Option<&str>,
        user: &str,
        max_output: Option<u32>,
    ) -> Result<String, AppError> {
        if !valid_model_id(model_id) {
            return Err(AppError::validation("modelId"));
        }
        let (connection, key, ai_route) = self.load(connection_id).await?;
        let route = self.route(&ai_route, connection.use_proxy).await?;
        let endpoint = Endpoint {
            protocol: connection.protocol,
            base_url: &connection.base_url,
            key: Some(&key),
            route: &route,
        };
        client::complete(&endpoint, model_id, system, user, max_output)
            .await
            .map_err(AppError::new)
    }

    // ------------------------------------------------------------ catalog

    fn catalog_view(
        data: Option<&catalog::CatalogData>,
        refresh_error: Option<ErrorCode>,
    ) -> AiCatalog {
        AiCatalog {
            providers: catalog::providers(data),
            updated_at: data.and_then(|data| {
                chrono::DateTime::from_timestamp(data.fetched_at, 0).map(|time| time.to_rfc3339())
            }),
            stale: data.is_none_or(|data| data.stale(now())),
            refresh_error,
        }
    }

    /// The cached catalog (built-ins only without one). With `refresh`, asks
    /// models.dev first; a failed refresh keeps the old cache and says so in
    /// `refresh_error` instead of failing.
    pub async fn catalog(self: &Arc<Self>, refresh: bool) -> Result<AiCatalog, AppError> {
        let cached = self.blocking(|this| Ok(this.catalog_data())).await?;
        if !refresh {
            return Ok(Self::catalog_view(cached.as_deref(), None));
        }
        let _one_at_a_time = self.refresh.lock().await;
        // Another refresh may have finished while this one waited.
        let cached = self.blocking(|this| Ok(this.catalog_data())).await?;
        match self.refresh_catalog(cached.clone()).await {
            Ok(data) => Ok(Self::catalog_view(Some(&data), None)),
            Err(error) => Ok(Self::catalog_view(cached.as_deref(), Some(error.code))),
        }
    }

    async fn refresh_catalog(
        self: &Arc<Self>,
        cached: Option<Arc<catalog::CatalogData>>,
    ) -> Result<Arc<catalog::CatalogData>, AppError> {
        let ai_route = self.blocking(|this| this.ai_settings()).await?.route;
        let route = self.route(&ai_route, true).await?;
        let etag = cached.as_ref().and_then(|data| data.etag.clone());
        let fetched = catalog::fetch(&route, &self.catalog_url, etag.as_deref())
            .await
            .map_err(AppError::new)?;
        let data = match (fetched, cached) {
            (catalog::Fetched::Fresh { etag, providers }, _) => {
                catalog::CatalogData::new(etag, now(), providers)
            }
            (catalog::Fetched::NotModified, Some(old)) => {
                catalog::CatalogData::new(old.etag.clone(), now(), old.providers.clone())
            }
            (catalog::Fetched::NotModified, None) => {
                return Err(AppError::new(ErrorCode::AiInvalidResponse));
            }
        };
        let data = Arc::new(data);
        let stored = data.clone();
        self.blocking(move |this| {
            // A cache that cannot be written still serves this run.
            let _ = catalog::write_cache(&this.cache_directory, &stored);
            *this
                .catalog_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(stored);
            Ok(())
        })
        .await?;
        Ok(data)
    }

    // -------------------------------------------------------------- icons

    /// The provider's icon, or `None` when there is none. A failure is
    /// remembered until the app closes, so a missing icon is asked for once.
    pub async fn provider_icon(
        self: &Arc<Self>,
        provider_id: &str,
    ) -> Result<Option<AiProviderIcon>, AppError> {
        if !valid_provider_id(provider_id) {
            return Err(AppError::validation("providerId"));
        }
        let id = provider_id.to_owned();
        let cached = self
            .blocking(move |this| {
                if this
                    .failed_icons
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains(&id)
                {
                    return Ok(Some(None));
                }
                let directory = this.cache_directory.join(ICON_DIRECTORY);
                Ok(icons::read_cached(&directory, &id)
                    .map(|svg| Some(AiProviderIcon::from_svg(&svg))))
            })
            .await?;
        if let Some(found) = cached {
            return Ok(found);
        }
        let ai_route = self.blocking(|this| this.ai_settings()).await?.route;
        let route = self.route(&ai_route, true).await?;
        match icons::fetch(&route, &self.icon_base_url, provider_id).await {
            Ok(svg) => {
                let (id, stored) = (provider_id.to_owned(), svg.clone());
                self.blocking(move |this| {
                    icons::write_cached(&this.cache_directory.join(ICON_DIRECTORY), &id, &stored);
                    Ok(())
                })
                .await?;
                Ok(Some(AiProviderIcon::from_svg(&svg)))
            }
            Err(_) => {
                self.failed_icons
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(provider_id.to_owned());
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests;
