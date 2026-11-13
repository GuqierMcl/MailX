//! AI provider connections: settings, the provider catalog, model lists, the
//! protocol clients and provider icons. Nothing here reads mail. The app
//! service in `app::ai` ties it to settings, secrets and the network.

pub(crate) mod catalog;
pub(crate) mod client;
pub(crate) mod icons;
pub(crate) mod models;
pub mod settings;
pub mod target;
pub(crate) mod validation;

pub use catalog::{AiCatalog, AiCatalogProvider};
pub use icons::AiProviderIcon;
pub use models::{AiModel, AiModelSource, AiTestResult};
pub use settings::{AiConnection, AiModelRef, AiProtocol, AiSettings};
pub use target::AiModelTarget;
