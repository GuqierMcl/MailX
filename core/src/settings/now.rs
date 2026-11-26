//! The Now page's module preferences (`docs/architecture/now-page.md` 4.3).
//!
//! Module ids are opaque to core: the interface owns the list of modules, and
//! ids it does not know (written by a newer version) are kept so a downgrade
//! followed by an upgrade loses nothing.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

const MAX_IDS: usize = 32;
const MAX_ID_CHARS: usize = 32;

/// How the Now page orders its modules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NowArrangement {
    /// By zone and by what is going on right now.
    #[default]
    Auto,
    /// In the order the user chose.
    Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct NowSettings {
    pub arrangement: NowArrangement,
    /// Module ids in the user's order; empty means the default order.
    pub order: Vec<String>,
    /// Module ids that are never shown.
    pub hidden: Vec<String>,
}

impl NowSettings {
    /// The canonical form: malformed ids dropped, duplicates removed (the
    /// first one stays), at most [`MAX_IDS`] per list, order kept.
    pub(super) fn normalized(self) -> Self {
        Self {
            arrangement: self.arrangement,
            order: normalize_ids(&self.order),
            hidden: normalize_ids(&self.hidden),
        }
    }
}

/// A module id: ASCII letters, the first one lowercase, at most 32 characters.
fn valid_id(id: &str) -> bool {
    id.len() <= MAX_ID_CHARS
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_alphabetic())
}

fn normalize_ids(ids: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in ids {
        if kept.len() == MAX_IDS {
            break;
        }
        if valid_id(id) && !kept.contains(id) {
            kept.push(id.clone());
        }
    }
    kept
}

/// Drops `now` fields of the wrong type, so a hand-edited file loses only
/// that field.
pub(super) fn repair_now(value: &mut serde_json::Value) {
    let Some(section) = value.as_object_mut() else {
        return;
    };
    section.retain(|key, v| match key.as_str() {
        "arrangement" => serde_json::from_value::<NowArrangement>(v.clone()).is_ok(),
        "order" | "hidden" => v
            .as_array()
            .is_some_and(|a| a.iter().all(serde_json::Value::is_string)),
        _ => true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn defaults_are_automatic_and_empty() {
        let settings = NowSettings::default();
        assert_eq!(settings.arrangement, NowArrangement::Auto);
        assert!(settings.order.is_empty() && settings.hidden.is_empty());
        let parsed: NowSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(parsed, settings);
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            json!({ "arrangement": "auto", "order": [], "hidden": [] })
        );
        assert_eq!(
            serde_json::to_value(NowArrangement::Fixed).unwrap(),
            json!("fixed")
        );
    }

    #[test]
    fn normalizing_drops_malformed_ids_and_duplicates_but_keeps_order_and_unknown_ids() {
        let settings = NowSettings {
            arrangement: NowArrangement::Fixed,
            order: ids(&[
                "inbox",
                "Codes",
                "",
                "a1",
                "in-box",
                "inbox",
                "futureModule",
                "drafts",
                "ünï",
            ]),
            hidden: ids(&["activity", " activity", "activity", "x"]),
        }
        .normalized();
        assert_eq!(settings.arrangement, NowArrangement::Fixed);
        assert_eq!(settings.order, ids(&["inbox", "futureModule", "drafts"]));
        assert_eq!(settings.hidden, ids(&["activity", "x"]));
    }

    #[test]
    fn normalizing_bounds_id_length_and_list_size() {
        let long = "a".repeat(MAX_ID_CHARS);
        let too_long = "a".repeat(MAX_ID_CHARS + 1);
        let many: Vec<String> = (0..40).map(|i| "b".repeat(i + 1)).collect();
        let settings = NowSettings {
            order: [vec![long.clone(), too_long], many].concat(),
            ..NowSettings::default()
        }
        .normalized();
        assert_eq!(settings.order.len(), MAX_IDS);
        assert_eq!(settings.order[0], long);
        assert!(settings.order.iter().all(|id| valid_id(id)));
    }

    #[test]
    fn repair_drops_only_wrong_typed_fields() {
        for (input, expected) in [
            (
                json!({ "arrangement": 3, "order": ["a"], "hidden": ["b"] }),
                json!({ "order": ["a"], "hidden": ["b"] }),
            ),
            (json!({ "arrangement": "grid", "order": "a" }), json!({})),
            (
                json!({ "arrangement": "fixed", "order": ["a", 1], "hidden": ["b"] }),
                json!({ "arrangement": "fixed", "hidden": ["b"] }),
            ),
            (
                json!({ "arrangement": "fixed", "hidden": null, "extra": 1 }),
                json!({ "arrangement": "fixed", "extra": 1 }),
            ),
        ] {
            let mut value = input.clone();
            repair_now(&mut value);
            assert_eq!(value, expected, "{input}");
        }
    }
}
