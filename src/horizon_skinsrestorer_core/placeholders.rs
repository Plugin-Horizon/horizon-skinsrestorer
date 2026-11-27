//! The `%skinsrestorer_...%` placeholders (`SRPlaceholderAPIExpansion.onRequest`): `skin_name[_or_empty|_or_player_name]`,
//! `texture_url[_or_empty|_or_steve|_or_alex]`, `texture_id[_or_empty|_or_steve|_or_alex]`.

use super::property::Property;
use super::skins;

const ERROR_MESSAGE: &str = "Error";

fn url_of(p: &Property) -> Option<String> {
    p.textures().and_then(|t| t.skin_url)
}

fn hash_of(p: &Property) -> Option<String> {
    url_of(p).map(|u| u.rsplit('/').next().unwrap_or("").to_owned())
}

/// The value for `params` (case-insensitive), or None when the original answers null (an unknown placeholder).
/// `skin_id` is the player's selected skin identifier, `current` the textures of their profile, `name` their name.
pub fn request(params: &str, skin_id: Option<&str>, current: Option<&Property>, name: Option<&str>) -> Option<String> {
    let params = params.to_lowercase();
    if params.starts_with("skin_name") {
        if let Some(id) = skin_id {
            return Some(id.to_owned());
        }
        return Some(match params.strip_prefix("skin_name_") {
            Some("or_empty") => String::new(),
            Some("or_player_name") => name.unwrap_or("").to_owned(),
            _ => ERROR_MESSAGE.to_owned(),
        });
    }
    let (rest, extract): (&str, fn(&Property) -> Option<String>) = if params.starts_with("texture_url") {
        (params.strip_prefix("texture_url_").unwrap_or(""), url_of)
    } else if params.starts_with("texture_id") {
        (params.strip_prefix("texture_id_").unwrap_or(""), hash_of)
    } else {
        return None;
    };
    if let Some(v) = current.and_then(extract) {
        return Some(v);
    }
    let fallback = match rest {
        "or_empty" => Some(String::new()),
        "or_steve" => skins::hardcoded("steve").as_ref().and_then(extract),
        "or_alex" => skins::hardcoded("alex").as_ref().and_then(extract),
        _ => None,
    };
    Some(fallback.unwrap_or_else(|| ERROR_MESSAGE.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop() -> Property {
        Property::new(skins::texture_value("abc123"), None)
    }

    #[test]
    fn skin_name_variants() {
        assert_eq!(request("skin_name", Some("xknat"), None, Some("Bob")).unwrap(), "xknat");
        assert_eq!(request("skin_name", None, None, Some("Bob")).unwrap(), "Error");
        assert_eq!(request("skin_name_or_empty", None, None, Some("Bob")).unwrap(), "");
        assert_eq!(request("SKIN_NAME_or_player_name", None, None, Some("Bob")).unwrap(), "Bob");
    }

    #[test]
    fn texture_url_and_id() {
        let p = prop();
        assert_eq!(
            request("texture_url", None, Some(&p), None).unwrap(),
            "https://textures.minecraft.net/texture/abc123"
        );
        assert_eq!(request("texture_id", None, Some(&p), None).unwrap(), "abc123");
        assert_eq!(request("texture_url", None, None, None).unwrap(), "Error");
        assert_eq!(request("texture_id_or_empty", None, None, None).unwrap(), "");
        assert!(request("texture_url_or_steve", None, None, None).unwrap().starts_with("http"));
        assert!(!request("texture_id_or_alex", None, None, None).unwrap().is_empty());
    }

    #[test]
    fn unknown_placeholder_is_none() {
        assert_eq!(request("other", None, None, None), None);
    }
}
