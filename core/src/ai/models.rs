//! The models a provider reports, enriched from the cached catalog, and the
//! catalog's own list of presets.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{
    catalog::{CatalogModel, CatalogProvider},
    client::ProviderModel,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AiModelSource {
    /// The provider's own model list.
    Provider,
    /// Preset from the catalog, not a listing from the connection.
    Catalog,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiModel {
    pub id: String,
    pub name: String,
    pub context_length: Option<u32>,
    pub output_length: Option<u32>,
    /// `None` when nobody says.
    pub tool_call: Option<bool>,
    /// Display-only attribution reported by the list endpoint, never a route.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub owned_by: Option<String>,
    pub source: AiModelSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiTestResult {
    pub latency_ms: u32,
}

fn known<'a>(catalog: Option<&'a CatalogProvider>, id: &str) -> Option<&'a CatalogModel> {
    catalog?.models.iter().find(|model| model.id == id)
}

/// The provider's list, with the catalog's names and limits filled in. Models
/// the catalog says do not output text are dropped. The provider's own name
/// wins when it gives one.
pub(crate) fn enrich(
    listed: Vec<ProviderModel>,
    catalog: Option<&CatalogProvider>,
) -> Vec<AiModel> {
    listed
        .into_iter()
        .filter_map(|model| {
            let extra = known(catalog, &model.id);
            if extra.is_some_and(|extra| !extra.text) {
                return None;
            }
            Some(AiModel {
                name: model
                    .name
                    .or_else(|| extra.map(|extra| extra.name.clone()))
                    .unwrap_or_else(|| model.id.clone()),
                context_length: extra.and_then(|extra| extra.context),
                output_length: extra.and_then(|extra| extra.output),
                tool_call: extra.and_then(|extra| extra.tool_call),
                owned_by: model.owned_by,
                source: AiModelSource::Provider,
                id: model.id,
            })
        })
        .collect()
}

/// Text-capable presets for this provider, if there are any in the catalog.
pub(crate) fn from_catalog(catalog: Option<&CatalogProvider>) -> Vec<AiModel> {
    catalog
        .into_iter()
        .flat_map(|provider| &provider.models)
        .filter(|model| model.text)
        .map(|model| AiModel {
            id: model.id.clone(),
            name: model.name.clone(),
            context_length: model.context,
            output_length: model.output,
            tool_call: model.tool_call,
            owned_by: None,
            source: AiModelSource::Catalog,
        })
        .collect()
}
