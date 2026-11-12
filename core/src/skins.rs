//! Skins the original keeps outside the database: the hardcoded default skins
//! (`hardcoded_skins.json`, bundled in `data/` from SkinsRestorer's resources, GPL-3.0) that a
//! CUSTOM selection falls back to, the recommendation cache (`plugins/SkinsRestorer/recommendations.json`,
//! read for the `sr-recommendation-<id>` selections), and the textures value of a player head.
use crate::property::Property;
use base64::Engine;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// `SkinStorageImpl.RECOMMENDATION_PREFIX`.
pub const RECOMMENDATION_PREFIX: &str = "sr-recommendation-";

const HARDCODED: &str = include_str!("../data/hardcoded_skins.json");

#[derive(serde::Deserialize)]
struct HardcodedEntry {
    value: String,
    signature: String,
}

fn hardcoded_map() -> &'static HashMap<String, Property> {
    static MAP: OnceLock<HashMap<String, Property>> = OnceLock::new();
    MAP.get_or_init(|| {
        let raw: HashMap<String, HardcodedEntry> =
            serde_json::from_str(HARDCODED).expect("data/hardcoded_skins.json parses");
        raw.into_iter()
            .map(|(name, e)| (name.to_lowercase(), Property::new(e.value, Some(e.signature))))
            .collect()
    })
}

/// `HardcodedSkins.getHardcodedSkin`: the bundled skin of that name, names in lower case.
pub fn hardcoded(name: &str) -> Option<Property> {
    hardcoded_map().get(&name.to_lowercase()).cloned()
}

/// The `textures` value of a player head showing `hash` (`SkullUtil.setSkull`): base64 of
/// `{"textures":{"SKIN":{"url":"https://textures.minecraft.net/texture/<hash>"}}}`, unsigned.
pub fn texture_value(hash: &str) -> String {
    let json = serde_json::json!({
        "textures": { "SKIN": { "url": format!("https://textures.minecraft.net/texture/{hash}") } }
    });
    base64::engine::general_purpose::STANDARD.encode(json.to_string())
}

/// The recommended skins by id (`RecommenationResponse.SkinInfo`).
#[derive(Clone, Debug, Default)]
pub struct Recommendations {
    skins: HashMap<String, Property>,
}

impl Recommendations {
    /// `{"version": n, "skins": [{"skinId", "skinName", "value", "signature"}, ...]}`. An entry
    /// without a value is skipped; the others are kept. Err when the document is not that shape.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let list = doc
            .get("skins")
            .and_then(|s| s.as_array())
            .ok_or("no skins list")?;
        let mut skins = HashMap::new();
        for s in list {
            let id = s.get("skinId").and_then(|v| v.as_str());
            let value = s.get("value").and_then(|v| v.as_str()).filter(|v| !v.is_empty());
            if let (Some(id), Some(value)) = (id, value) {
                let signature = s.get("signature").and_then(|v| v.as_str()).map(str::to_owned);
                skins.insert(id.to_owned(), Property::new(value, signature));
            }
        }
        Ok(Self { skins })
    }

    /// The file in the server folder; a missing or broken file gives an empty list (logged).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|e| {
                log::warn!("[SkinsRestorer] could not read the recommended skins: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn get(&self, id: &str) -> Option<Property> {
        self.skins.get(id).cloned()
    }

    pub fn len(&self) -> usize {
        self.skins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_defaults_are_the_originals() {
        let steve = hardcoded("steve").expect("steve is bundled");
        assert!(steve.signature.is_some());
        assert!(hardcoded("Alex").is_some(), "names are looked up in lower case");
        assert!(hardcoded("technoblade").is_some());
        assert!(hardcoded("nobody_by_that_name").is_none());
    }

    #[test]
    fn the_texture_value_is_the_skull_utils_url() {
        let v = texture_value("abc123");
        let json = base64::engine::general_purpose::STANDARD.decode(v).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            doc["textures"]["SKIN"]["url"],
            "https://textures.minecraft.net/texture/abc123"
        );
    }

    const LIST: &str = r#"{"version": 1, "skins": [
        {"skinId": "7f", "skinName": "Knight", "value": "dmFsdWU=", "signature": "c2ln"},
        {"skinId": "bad", "skinName": "No value", "value": "", "signature": "x"},
        {"skinId": "nosig", "skinName": "Unsigned", "value": "dg==", "signature": ""}
    ]}"#;

    #[test]
    fn recommendations_are_read_by_skin_id() {
        let r = Recommendations::parse(LIST).unwrap();
        assert_eq!(r.len(), 2);
        let k = r.get("7f").unwrap();
        assert_eq!(k.value, "dmFsdWU=");
        assert_eq!(k.signature.as_deref(), Some("c2ln"));
        assert!(r.get("bad").is_none());
        assert_eq!(r.get("nosig").unwrap().signature, None);
        assert!(Recommendations::parse("{\"skins\": 3}").is_err());
    }

    #[test]
    fn a_missing_recommendation_file_is_empty() {
        let r = Recommendations::load(Path::new("/nonexistent/skinsrestorer/recommendations.json"));
        assert!(r.is_empty());
    }
}
