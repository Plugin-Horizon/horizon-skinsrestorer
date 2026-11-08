//! A profile's `textures` property (Mojang's format: base64 JSON, signed by the session server)
//! and UUID text forms.
use base64::Engine;

pub const TEXTURES: &str = "textures";

/// A signed `textures` property: what a profile carries and the player list sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub value: String,
    pub signature: Option<String>,
}

/// What the value says, for diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Textures {
    pub profile_name: Option<String>,
    pub skin_url: Option<String>,
    pub slim: bool,
    pub cape_url: Option<String>,
}

impl Property {
    pub fn new(value: impl Into<String>, signature: Option<String>) -> Self {
        Self {
            value: value.into(),
            signature: signature.filter(|s| !s.is_empty()),
        }
    }

    /// The decoded value, if it is base64 JSON with a `textures` object.
    pub fn textures(&self) -> Option<Textures> {
        let json = decode_b64(&self.value)?;
        let v: serde_json::Value = serde_json::from_slice(&json).ok()?;
        let t = v.get("textures")?;
        let skin = t.get("SKIN");
        Some(Textures {
            profile_name: v.get("profileName").and_then(|x| x.as_str()).map(str::to_owned),
            skin_url: skin
                .and_then(|s| s.get("url"))
                .and_then(|x| x.as_str())
                .map(str::to_owned),
            slim: skin
                .and_then(|s| s.get("metadata"))
                .and_then(|m| m.get("model"))
                .and_then(|x| x.as_str())
                == Some("slim"),
            cape_url: t
                .get("CAPE")
                .and_then(|c| c.get("url"))
                .and_then(|x| x.as_str())
                .map(str::to_owned),
        })
    }

    /// The texture hash (the skin URL's last segment), short, for status lines.
    pub fn short_hash(&self) -> String {
        match self.textures().and_then(|t| t.skin_url) {
            Some(url) => {
                let h = url.rsplit('/').next().unwrap_or("");
                h.chars().take(12).collect()
            }
            None => "(no skin)".into(),
        }
    }

    pub fn looks_valid(&self) -> bool {
        self.textures().is_some()
    }
}

/// Standard or URL-safe base64, padded or not.
pub fn decode_b64(s: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let s = s.trim();
    [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD]
        .iter()
        .find_map(|e| e.decode(s).ok())
}

/// `8-4-4-4-12` lowercase hex.
pub fn uuid_dashed(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn uuid_plain(u: u128) -> String {
    format!("{u:032x}")
}

pub fn parse_uuid(s: &str) -> Option<u128> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
}

/// A profile's properties with `textures` replaced (or removed for `None`), the others kept in
/// their order. Generic over the engine's property type: `is_textures` tells its name.
pub fn replace_textures<P>(
    props: &mut Vec<P>,
    new: Option<&Property>,
    is_textures: impl Fn(&P) -> bool,
    make: impl Fn(&Property) -> P,
) {
    let at = props.iter().position(is_textures);
    match (at, new) {
        (Some(i), Some(n)) => props[i] = make(n),
        (Some(i), None) => {
            props.remove(i);
        }
        (None, Some(n)) => props.push(make(n)),
        (None, None) => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic textures value (not a real player's).
    pub fn sample(hash: &str, slim: bool) -> Property {
        let model = if slim { r#","metadata":{"model":"slim"}"# } else { "" };
        let json = format!(
            r#"{{"timestamp":1,"profileId":"00000000000000000000000000000001","profileName":"Sample","textures":{{"SKIN":{{"url":"http://textures.minecraft.net/texture/{hash}"{model}}}}}}}"#
        );
        Property::new(
            base64::engine::general_purpose::STANDARD.encode(json),
            Some(base64::engine::general_purpose::STANDARD.encode([7u8; 512])),
        )
    }

    #[test]
    fn decodes_textures() {
        let p = sample("abcdef0123456789", true);
        let t = p.textures().unwrap();
        assert_eq!(t.profile_name.as_deref(), Some("Sample"));
        assert!(t.slim);
        assert_eq!(p.short_hash(), "abcdef012345");
        assert!(!Property::new("not base64 json", None).looks_valid());
    }

    #[test]
    fn uuids_round_trip() {
        let u = 0x0123456789abcdef0123456789abcdefu128;
        assert_eq!(uuid_dashed(u), "01234567-89ab-cdef-0123-456789abcdef");
        assert_eq!(parse_uuid(&uuid_dashed(u)), Some(u));
        assert_eq!(parse_uuid(&uuid_plain(u)), Some(u));
        assert_eq!(parse_uuid("xyz"), None);
    }

    #[test]
    fn replaces_only_textures() {
        let mut props = vec![("other".to_string(), "1".to_string()), ("textures".into(), "old".into())];
        let n = Property::new("new", None);
        let name = |p: &(String, String)| p.0 == TEXTURES;
        let make = |p: &Property| ("textures".to_string(), p.value.clone());
        replace_textures(&mut props, Some(&n), name, make);
        assert_eq!(props[1].1, "new");
        replace_textures(&mut props, None, name, make);
        assert_eq!(props.len(), 1);
        replace_textures(&mut props, Some(&n), name, make);
        assert_eq!(props.last().unwrap().1, "new");
    }
}
