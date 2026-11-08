//! The proxy's plugin message on `sr:messagechannel`, as the original's `SRServerPluginMessage`
//! and `SRProxyPluginMessage` codecs write it.
//!
//! Java's `DataOutput`: big-endian, a string is `writeUTF` (a `u16` byte length and modified
//! UTF-8), a boolean one byte, a UUID two `long`s (most significant first). A message is the
//! channel type's name as a string, then that type's fields:
//!
//! | type | fields |
//! |---|---|
//! | `skinUpdateV3` | value, signature, `bool` ack present, then the ack: id (UUID), proxy version |
//! | `SkinUpdateV2` | value, signature |
//! | `openGUI` | the inventory (not read here) |
//! | `giveSkull` | display name (JSON string), texture hash |
//!
//! The backend answers an ack with a `SRProxyPluginMessage`: `ack`, id (UUID), version.
use crate::property::Property;

/// The channel (`SRHelpers.MESSAGE_CHANNEL`).
pub const CHANNEL: &str = "sr:messagechannel";

/// What a V3 update asks the backend to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ack {
    pub id: u128,
    pub proxy_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    /// Apply this skin; None is the original's empty property (no textures).
    SkinUpdate { property: Option<Property>, ack: Option<Ack> },
    /// `/skins` on the proxy: the inventory menu (not supported, see GAPS.md).
    OpenGui,
    /// `/skull` on the proxy.
    GiveSkull { texture_hash: String },
    /// A type this version does not know (the original logs a warning).
    Unknown(String),
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.data.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
    fn boolean(&mut self) -> Option<bool> {
        Some(self.take(1)?[0] != 0)
    }
    /// `DataInput.readUTF`.
    fn string(&mut self) -> Option<String> {
        let n = self.u16()? as usize;
        decode_modified_utf8(self.take(n)?)
    }
    /// `UUID_CODEC`: most significant long, then least.
    fn uuid(&mut self) -> Option<u128> {
        let msb = self.i64()? as u64;
        let lsb = self.i64()? as u64;
        Some(((msb as u128) << 64) | lsb as u128)
    }
}

/// Java's modified UTF-8: `0xC0 0x80` is NUL, supplementary characters are surrogate pairs of
/// three bytes each.
pub fn decode_modified_utf8(b: &[u8]) -> Option<String> {
    let mut units: Vec<u16> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c >> 4 {
            0..=7 => {
                units.push(c as u16);
                i += 1;
            }
            12 | 13 => {
                let d = *b.get(i + 1)?;
                if d & 0xc0 != 0x80 {
                    return None;
                }
                units.push((((c & 0x1f) as u16) << 6) | (d & 0x3f) as u16);
                i += 2;
            }
            14 => {
                let (d, e) = (*b.get(i + 1)?, *b.get(i + 2)?);
                if d & 0xc0 != 0x80 || e & 0xc0 != 0x80 {
                    return None;
                }
                units.push((((c & 0x0f) as u16) << 12) | (((d & 0x3f) as u16) << 6) | (e & 0x3f) as u16);
                i += 3;
            }
            _ => return None,
        }
    }
    String::from_utf16(&units).ok()
}

/// `DataOutput.writeUTF` (None when the text is past 65535 bytes).
pub fn encode_modified_utf8(s: &str) -> Option<Vec<u8>> {
    let mut body = Vec::with_capacity(s.len());
    for u in s.encode_utf16() {
        match u {
            1..=0x7f => body.push(u as u8),
            0 | 0x80..=0x7ff => {
                body.push(0xc0 | (u >> 6) as u8);
                body.push(0x80 | (u & 0x3f) as u8);
            }
            _ => {
                body.push(0xe0 | (u >> 12) as u8);
                body.push(0x80 | ((u >> 6) & 0x3f) as u8);
                body.push(0x80 | (u & 0x3f) as u8);
            }
        }
    }
    let n = u16::try_from(body.len()).ok()?;
    let mut out = n.to_be_bytes().to_vec();
    out.extend(body);
    Some(out)
}

fn property(value: String, signature: String) -> Option<Property> {
    // SkinProperty.of("", ""): the original's empty skin, which clears the textures.
    if value.is_empty() {
        None
    } else {
        Some(Property::new(value, Some(signature)))
    }
}

/// A message from the proxy; None when the bytes are not a message of this layout.
pub fn decode(data: &[u8]) -> Option<ServerMessage> {
    let mut r = Reader { data, at: 0 };
    let kind = r.string()?;
    Some(match kind.as_str() {
        "skinUpdateV3" => {
            let value = r.string()?;
            let signature = r.string()?;
            let ack = if r.boolean()? {
                Some(Ack {
                    id: r.uuid()?,
                    proxy_version: r.string()?,
                })
            } else {
                None
            };
            ServerMessage::SkinUpdate {
                property: property(value, signature),
                ack,
            }
        }
        "SkinUpdateV2" => {
            let value = r.string()?;
            let signature = r.string()?;
            ServerMessage::SkinUpdate {
                property: property(value, signature),
                ack: None,
            }
        }
        "openGUI" => ServerMessage::OpenGui,
        "giveSkull" => {
            let _display_name = r.string()?;
            ServerMessage::GiveSkull {
                texture_hash: r.string()?,
            }
        }
        _ => ServerMessage::Unknown(kind),
    })
}

/// The backend's answer to an ack request (`SRProxyPluginMessage` type `ack`).
pub fn encode_ack(ack: &Ack) -> Option<Vec<u8>> {
    let mut out = encode_modified_utf8("ack")?;
    out.extend(((ack.id >> 64) as u64).to_be_bytes());
    out.extend((ack.id as u64).to_be_bytes());
    out.extend(encode_modified_utf8(&ack.proxy_version)?);
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::property::tests::sample;

    fn utf(s: &str) -> Vec<u8> {
        encode_modified_utf8(s).unwrap()
    }

    pub fn v3(p: &Property, ack: Option<&Ack>) -> Vec<u8> {
        let mut d = utf("skinUpdateV3");
        d.extend(utf(&p.value));
        d.extend(utf(p.signature.as_deref().unwrap_or("")));
        match ack {
            None => d.push(0),
            Some(a) => {
                d.push(1);
                d.extend(((a.id >> 64) as u64).to_be_bytes());
                d.extend((a.id as u64).to_be_bytes());
                d.extend(utf(&a.proxy_version));
            }
        }
        d
    }

    #[test]
    fn a_v3_update_with_an_ack() {
        let p = sample("feedbeef", false);
        let a = Ack {
            id: 0x0123456789abcdef_fedcba9876543210,
            proxy_version: "15.12.7".into(),
        };
        let d = v3(&p, Some(&a));
        assert_eq!(
            decode(&d),
            Some(ServerMessage::SkinUpdate {
                property: Some(p),
                ack: Some(a.clone())
            })
        );
        // The answer: "ack", the same id, the version.
        let mut want = vec![0, 3, b'a', b'c', b'k'];
        want.extend(0x0123456789abcdefu64.to_be_bytes());
        want.extend(0xfedcba9876543210u64.to_be_bytes());
        want.extend(utf("15.12.7"));
        assert_eq!(encode_ack(&a).unwrap(), want);
    }

    #[test]
    fn a_v3_update_without_an_ack_and_the_empty_skin() {
        let p = sample("cafe", true);
        assert_eq!(
            decode(&v3(&p, None)),
            Some(ServerMessage::SkinUpdate { property: Some(p), ack: None })
        );
        let mut d = utf("skinUpdateV3");
        d.extend(utf(""));
        d.extend(utf(""));
        d.push(0);
        assert_eq!(decode(&d), Some(ServerMessage::SkinUpdate { property: None, ack: None }));
    }

    #[test]
    fn a_v2_update() {
        let p = sample("0a0b", false);
        let mut d = utf("SkinUpdateV2");
        d.extend(utf(&p.value));
        d.extend(utf(p.signature.as_deref().unwrap()));
        assert_eq!(decode(&d), Some(ServerMessage::SkinUpdate { property: Some(p), ack: None }));
    }

    #[test]
    fn other_types_and_bad_bytes() {
        assert_eq!(decode(&utf("openGUI")), Some(ServerMessage::OpenGui));
        let mut d = utf("giveSkull");
        d.extend(utf("{\"text\":\"x\"}"));
        d.extend(utf("abc123"));
        assert_eq!(decode(&d), Some(ServerMessage::GiveSkull { texture_hash: "abc123".into() }));
        assert_eq!(decode(&utf("brandNew")), Some(ServerMessage::Unknown("brandNew".into())));
        assert_eq!(decode(b""), None);
        assert_eq!(decode(&[0, 9, b'a']), None);
        // A V3 update cut short.
        let mut d = utf("skinUpdateV3");
        d.extend(utf("v"));
        assert_eq!(decode(&d), None);
    }

    #[test]
    fn modified_utf8_round_trips() {
        for s in ["", "abc", "caf\u{e9}", "nul\0nul", "\u{1f600}", "\u{20ac}"] {
            assert_eq!(decode_modified_utf8(&utf(s)[2..]).as_deref(), Some(s));
        }
        // NUL is C0 80 and a supplementary character is two 3-byte surrogates.
        assert_eq!(utf("\0"), vec![0, 2, 0xc0, 0x80]);
        assert_eq!(utf("\u{1f600}").len(), 2 + 6);
    }
}
