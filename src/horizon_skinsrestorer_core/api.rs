//! The Java API `net.skinsrestorer.api` (`SkinsRestorerProvider.get()` with `SkinStorage`, `PlayerStorage`,
//! `SkinApplier`, `SkinProperty`, `SkinIdentifier`, `InputDataResult`, `PropertyUtils`) for other Horizon mods.
//! A mod calls the service `skinsrestorer:api` (`host.commands().call_mod_service`) with one tab-separated line,
//! `<op>` then its arguments, and gets one text line back.
//!
//! Notation: a UUID is dashed or plain; `<type>` is `PLAYER|URL|CUSTOM|LEGACY`; `<variant>` is `CLASSIC|SLIM`, or
//! `-`/empty for none; an identifier is `<id>\t<type>\t<variant>` (`SkinIdentifier`); a property is
//! `<value>\t<signature>` (`SkinProperty`, an empty signature stays empty). Answers: `none` for an empty
//! `Optional`; `true`/`false`; `ok` for a `void` call; `error\t<message>` for a `DataRequestException`, a failed
//! storage, a bad line or a missing storage. An unknown op answers `error\tunknown op <op>` (a service call
//! that is not for this mod does not exist: the service name is ours).
//!
//! Operations (Java method in brackets):
//! - `version`, `available` (`SkinsRestorerProvider.get()` works: always `true`).
//! - `PropertyUtils`: `property_texture_url <value>`, `property_texture_hash <value>`, `property_texture_url_stripped
//!   <value>`, `property_variant <value>` (`CLASSIC`/`SLIM`), `property_profile_name <value>`; each `none` when the
//!   value has no skin texture.
//! - `PlayerStorage`: `get_skin_id_of_player <uuid>` (identifier), `set_skin_id_of_player <uuid> <id> <type>
//!   <variant>`, `remove_skin_id_of_player <uuid>`, `get_skin_of_player <uuid>` (property),
//!   `get_skin_id_for_player <uuid> <name>` / `get_skin_for_player <uuid> <name>` (the selection, else the
//!   account's own skin: stored, else fetched from Mojang and stored).
//! - `SkinStorage`: `get_skin_data_by_identifier <id> <type> <variant>` (property), `set_player_skin_data <uuid>
//!   <name> <value> <signature> <timestamp>`, `set_url_skin_data <url> <mineSkinId> <value> <signature> <variant>`,
//!   `set_url_skin_index <url> <variant>`, `set_custom_skin_data <name> <value> <signature>`, `remove_skin_data <id>
//!   <type> <variant>`, `find_skin_data <input> [variant]` and `find_or_create_skin_data <input> [variant]` (an
//!   `InputDataResult`: `<id>\t<type>\t<variant>\t<value>\t<signature>`), `get_player_skin <nameOrUuid>
//!   [allowExpired]` (`MojangSkinDataResult`: `<uuid>\t<value>\t<signature>`), `update_player_skin_data <uuid>`
//!   (property), `get_uuid <name>` (`CacheStorage.getUUID`).
//! - `SkinApplier`: `apply_skin <uuid> <value> <signature>` (a raw property), `apply_skin_id <uuid> <id> <type>
//!   <variant>`, `apply_skin_of <uuid>` (the skin that would be set on join), `reset_skin <uuid>` (no textures);
//!   `true` when the player is online and the skin is queued for the next tick, else `false`.
//!
//! - `SkinApplyEvent`: `subscribe <mod id>` / `unsubscribe <mod id>` (`true`). From then on the mod's own service
//!   `<mod id>:skinsrestorer_event` is called before every skin is shown, with `skin_apply\t<uuid>\t<value>\t<signature>`
//!   ([`apply_event`]); the answer `cancel` cancels the apply, `set\t<value>\t<signature>` replaces the property
//!   ([`parse_verdict`]), anything else leaves it. A subscriber whose mod is gone is dropped.
//!
//! A URL that is not stored is generated through MineSkin by `find_or_create_skin_data` (`net::mineskin_generate`,
//! key from `api.mineskinAPIKey`), stored with its index, and answered; a failure answers `error\t<the original's
//! message>`.
use crate::horizon_skinsrestorer_core::property::{parse_uuid, uuid_dashed, Property};
use crate::horizon_skinsrestorer_core::storage::{Lookup, Selection, SkinType, Storage};
use std::collections::HashMap;

/// `MojangAPI`: an account's name or UUID to its UUID, last name and signed skin.
pub trait Mojang {
    fn profile(&self, input: &str) -> Result<Option<(u128, String, Property)>, String>;
    /// `MineSkinAPI.genSkin`: (property, MineSkin id, variant `CLASSIC`/`SLIM`); Err is the original's message.
    fn generate(&self, url: &str, variant: Option<&str>) -> Result<(Property, String, String), String>;
}

/// What an apply call leaves for the core to do.
#[derive(Debug, PartialEq)]
pub enum Effect {
    Apply(u128, Option<Property>),
    Subscribe(String),
    Unsubscribe(String),
}

/// The line a subscriber's service `<id>:skinsrestorer_event` gets before a skin is applied
/// (`SkinApplyEvent`): `skin_apply\t<uuid>\t<value>\t<signature>` (empty value for no textures).
pub fn apply_event(player: u128, property: Option<&Property>) -> String {
    let p = property.map(|p| (p.value.as_str(), p.signature.as_deref().unwrap_or(""))).unwrap_or(("", ""));
    format!("skin_apply\t{}\t{}\t{}", uuid_dashed(player), p.0, p.1)
}

/// A subscriber's answer to [`apply_event`]: `cancel` (`setCancelled(true)`) or `set\t<value>\t<signature>`
/// (`setProperty`; an empty value removes the textures); anything else leaves the event as it is.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Keep,
    Cancel,
    Set(Option<Property>),
}

pub fn parse_verdict(reply: &str) -> Verdict {
    let parts: Vec<&str> = reply.trim_end_matches(['\r', '\n']).split('\t').collect();
    match parts.as_slice() {
        ["cancel"] => Verdict::Cancel,
        ["set", value, sig] => Verdict::Set(if value.is_empty() { None } else { Some(make_prop(value, sig)) }),
        _ => Verdict::Keep,
    }
}

/// What the call can see: the storage, the online players (lower case name to UUID) and the Mojang lookup.
pub struct Ctx<'a> {
    pub storage: Option<&'a Storage>,
    pub online: &'a HashMap<String, u128>,
    pub mojang: &'a dyn Mojang,
    pub now_secs: i64,
}

fn err(m: impl std::fmt::Display) -> String {
    format!("error\t{m}")
}

fn kind(s: &str) -> Option<SkinType> {
    match s.to_ascii_uppercase().as_str() {
        "PLAYER" => Some(SkinType::Player),
        "URL" => Some(SkinType::Url),
        "CUSTOM" => Some(SkinType::Custom),
        "LEGACY" => Some(SkinType::Legacy),
        _ => None,
    }
}

fn kind_name(k: SkinType) -> &'static str {
    match k {
        SkinType::Player => "PLAYER",
        SkinType::Url => "URL",
        SkinType::Custom => "CUSTOM",
        SkinType::Legacy => "LEGACY",
    }
}

fn variant(s: &str) -> Result<Option<String>, String> {
    match s.to_ascii_uppercase().as_str() {
        "" | "-" | "NONE" => Ok(None),
        "CLASSIC" => Ok(Some("CLASSIC".into())),
        "SLIM" => Ok(Some("SLIM".into())),
        _ => Err(format!("unknown skin variant {s}")),
    }
}

fn ident(s: &Selection) -> String {
    format!("{}\t{}\t{}", s.identifier, kind_name(s.kind), s.variant.as_deref().unwrap_or(""))
}

fn prop(p: &Property) -> String {
    format!("{}\t{}", p.value, p.signature.as_deref().unwrap_or(""))
}

fn make_prop(value: &str, sig: &str) -> Property {
    Property::new(value, Some(sig.to_owned()))
}

fn uuid(s: &str) -> Result<u128, String> {
    parse_uuid(s).ok_or_else(|| format!("not a UUID: {s}"))
}

fn selection(id: &str, ty: &str, var: &str) -> Result<Selection, String> {
    let kind = kind(ty).ok_or_else(|| format!("unknown skin type {ty}"))?;
    let variant = variant(var)?;
    // A player identifier is the UUID, written dashed (`SkinIdentifier.ofPlayer`).
    let identifier = match (kind, parse_uuid(id)) {
        (SkinType::Player, Some(u)) => uuid_dashed(u),
        _ => id.to_owned(),
    };
    Ok(Selection {
        identifier,
        kind,
        variant: if kind == SkinType::Url { variant } else { None },
    })
}

fn opt(s: Option<String>) -> String {
    s.unwrap_or_else(|| "none".into())
}

fn texture_url(value: &str) -> Option<String> {
    Property::new(value, None).textures()?.skin_url
}

/// `MojangSkinDataResult`'s source: the cached skin of an account, else Mojang's, stored.
fn player_skin(ctx: &Ctx, input: &str) -> Result<Option<(u128, String, Property)>, String> {
    let st = ctx.storage;
    let known = parse_uuid(input).or_else(|| ctx.online.get(&input.to_ascii_lowercase()).copied());
    if let (Some(u), Some(st)) = (known, st) {
        let sel = Selection { identifier: uuid_dashed(u), kind: SkinType::Player, variant: None };
        if let Some(p) = st.skin(&sel)? {
            let name = if parse_uuid(input).is_some() { String::new() } else { input.to_owned() };
            return Ok(Some((u, name, p)));
        }
    }
    let Some((u, name, p)) = ctx.mojang.profile(input)? else { return Ok(None) };
    if let Some(st) = st {
        st.set_player_skin(u, &name, &p, ctx.now_secs)?;
    }
    Ok(Some((u, name, p)))
}

/// The skin `getSkinForPlayer` finds: (identifier, property).
fn skin_for(ctx: &Ctx, player: u128, name: &str) -> Result<Option<(Selection, Property)>, String> {
    if let Some(st) = ctx.storage {
        match st.skin_of(player)? {
            Lookup::Skin(sel, p) => return Ok(Some((sel, p))),
            Lookup::Dangling(_) => return Ok(None),
            Lookup::NoSelection => {}
        }
    }
    Ok(player_skin(ctx, &uuid_dashed(player)).or_else(|_| player_skin(ctx, name))?.map(|(u, _, p)| {
        (Selection { identifier: uuid_dashed(u), kind: SkinType::Player, variant: None }, p)
    }))
}

fn find(ctx: &Ctx, input: &str, hint: Option<String>, create: bool) -> Result<String, String> {
    let st = ctx.storage.ok_or("no skin storage is configured")?;
    let found = |sel: Selection, p: Property| format!("{}\t{}", ident(&sel), prop(&p));
    if input.starts_with("http://") || input.starts_with("https://") {
        let sel = Selection { identifier: input.into(), kind: SkinType::Url, variant: hint };
        return Ok(match st.skin(&sel)? {
            Some(p) => found(sel, p),
            None if create => {
                // `findOrCreateSkinData`: generate through MineSkin, store the skin and (no variant asked) the index.
                let (p, id, var) = ctx.mojang.generate(input, sel.variant.as_deref())?;
                st.set_url_skin(input, Some(&id), &p, &var)?;
                if sel.variant.is_none() {
                    st.set_url_index(input, &var)?;
                }
                found(Selection { identifier: input.into(), kind: SkinType::Url, variant: Some(var) }, p)
            }
            None => "none".into(),
        });
    }
    let custom = Selection { identifier: input.to_lowercase(), kind: SkinType::Custom, variant: None };
    if let Some(p) = st.skin(&custom)? {
        return Ok(found(custom, p));
    }
    let legacy = Selection { identifier: input.to_owned(), kind: SkinType::Legacy, variant: None };
    if let Some(p) = st.skin(&legacy)? {
        return Ok(found(legacy, p));
    }
    let known = parse_uuid(input).or_else(|| ctx.online.get(&input.to_ascii_lowercase()).copied());
    if let Some(u) = known {
        let sel = Selection { identifier: uuid_dashed(u), kind: SkinType::Player, variant: None };
        if let Some(p) = st.skin(&sel)? {
            return Ok(found(sel, p));
        }
    }
    if !create {
        return Ok("none".into());
    }
    Ok(match player_skin(ctx, input)? {
        Some((u, _, p)) => found(Selection { identifier: uuid_dashed(u), kind: SkinType::Player, variant: None }, p),
        None => "none".into(),
    })
}

/// One call. The answer line, and what to apply.
pub fn handle(ctx: &Ctx, line: &str) -> (String, Vec<Effect>) {
    let mut effects = Vec::new();
    let r = run(ctx, line, &mut effects).unwrap_or_else(err);
    (r, effects)
}

fn run(ctx: &Ctx, line: &str, fx: &mut Vec<Effect>) -> Result<String, String> {
    let parts: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    let a = |i: usize| -> Result<&str, String> {
        parts.get(i).copied().ok_or_else(|| format!("{} needs argument {i}", parts[0]))
    };
    let st = || ctx.storage.ok_or_else(|| "no skin storage is configured".to_owned());
    Ok(match parts[0] {
        "version" => env!("CARGO_PKG_VERSION").to_owned(),
        "available" => "true".into(),
        "property_texture_url" => opt(texture_url(a(1)?)),
        "property_texture_hash" => opt(texture_url(a(1)?).map(|u| u.rsplit('/').next().unwrap_or("").to_owned())),
        "property_texture_url_stripped" => opt(texture_url(a(1)?).map(|u| {
            u.trim_start_matches("http://textures.minecraft.net/texture/")
                .trim_start_matches("https://textures.minecraft.net/texture/")
                .to_owned()
        })),
        "property_variant" => match Property::new(a(1)?, None).textures() {
            Some(t) if t.skin_url.is_some() => if t.slim { "SLIM" } else { "CLASSIC" }.into(),
            _ => "none".into(),
        },
        "property_profile_name" => opt(Property::new(a(1)?, None).textures().and_then(|t| t.profile_name)),
        "get_skin_id_of_player" => opt(st()?.selection(uuid(a(1)?)?)?.map(|s| ident(&s))),
        "set_skin_id_of_player" => {
            st()?.set_selection(uuid(a(1)?)?, &selection(a(2)?, a(3)?, parts.get(4).copied().unwrap_or(""))?)?;
            "ok".into()
        }
        "remove_skin_id_of_player" => {
            st()?.clear_selection(uuid(a(1)?)?)?;
            "ok".into()
        }
        "get_skin_of_player" => match st()?.skin_of(uuid(a(1)?)?)? {
            Lookup::Skin(_, p) => prop(&p),
            _ => "none".into(),
        },
        "get_skin_id_for_player" => opt(skin_for(ctx, uuid(a(1)?)?, a(2)?)?.map(|(s, _)| ident(&s))),
        "get_skin_for_player" => opt(skin_for(ctx, uuid(a(1)?)?, a(2)?)?.map(|(_, p)| prop(&p))),
        "get_skin_data_by_identifier" => {
            opt(st()?.skin(&selection(a(1)?, a(2)?, parts.get(3).copied().unwrap_or(""))?)?.map(|p| prop(&p)))
        }
        "set_player_skin_data" => {
            let ts = a(5)?.parse::<i64>().map_err(|_| "the timestamp is not a number".to_owned())?;
            st()?.set_player_skin(uuid(a(1)?)?, a(2)?, &make_prop(a(3)?, a(4)?), ts)?;
            "ok".into()
        }
        "set_url_skin_data" => {
            let v = variant(a(5)?)?.ok_or("a URL skin needs a variant")?;
            let mid = Some(a(2)?).filter(|m| !m.is_empty() && *m != "-");
            st()?.set_url_skin(a(1)?, mid, &make_prop(a(3)?, a(4)?), &v)?;
            "ok".into()
        }
        "set_url_skin_index" => {
            let v = variant(a(2)?)?.ok_or("a URL index needs a variant")?;
            st()?.set_url_index(a(1)?, &v)?;
            "ok".into()
        }
        "set_custom_skin_data" => {
            st()?.set_custom_skin(a(1)?, &make_prop(a(2)?, a(3)?))?;
            "ok".into()
        }
        "remove_skin_data" => {
            st()?.remove_skin(&selection(a(1)?, a(2)?, parts.get(3).copied().unwrap_or(""))?)?;
            "ok".into()
        }
        "find_skin_data" => find(ctx, a(1)?, variant(parts.get(2).copied().unwrap_or(""))?, false)?,
        "find_or_create_skin_data" => find(ctx, a(1)?, variant(parts.get(2).copied().unwrap_or(""))?, true)?,
        "get_player_skin" => opt(player_skin(ctx, a(1)?)?.map(|(u, _, p)| format!("{}\t{}", uuid_dashed(u), prop(&p)))),
        "update_player_skin_data" => {
            let u = uuid(a(1)?)?;
            match ctx.mojang.profile(&uuid_dashed(u))? {
                Some((_, name, p)) => {
                    if let Some(s) = ctx.storage {
                        s.set_player_skin(u, &name, &p, ctx.now_secs)?;
                    }
                    prop(&p)
                }
                None => "none".into(),
            }
        }
        "get_uuid" => {
            let name = a(1)?;
            match ctx.online.get(&name.to_ascii_lowercase()) {
                Some(u) => uuid_dashed(*u),
                None => opt(ctx.mojang.profile(name)?.map(|(u, _, _)| uuid_dashed(u))),
            }
        }
        "apply_skin" | "apply_skin_id" | "apply_skin_of" | "reset_skin" => {
            let u = uuid(a(1)?)?;
            let property = match parts[0] {
                "apply_skin" => Some(make_prop(a(2)?, a(3)?)),
                "apply_skin_id" => {
                    let sel = selection(a(2)?, a(3)?, parts.get(4).copied().unwrap_or(""))?;
                    Some(st()?.skin(&sel)?.ok_or("no such skin")?)
                }
                "apply_skin_of" => {
                    let name = ctx.online.iter().find(|(_, v)| **v == u).map(|(k, _)| k.as_str()).unwrap_or("");
                    Some(skin_for(ctx, u, name)?.ok_or("no skin to apply")?.1)
                }
                _ => None,
            };
            if ctx.online.values().any(|v| *v == u) {
                fx.push(Effect::Apply(u, property));
                "true".into()
            } else {
                "false".into()
            }
        }
        "subscribe" => {
            fx.push(Effect::Subscribe(a(1)?.to_owned()));
            "true".into()
        }
        "unsubscribe" => {
            fx.push(Effect::Unsubscribe(a(1)?.to_owned()));
            "true".into()
        }
        other => return Err(format!("unknown op {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_skinsrestorer_core::property::tests::sample;
    use crate::horizon_skinsrestorer_core::storage::tests::{storage, CUSTOM, PLAYER, SEL, URL};
    use crate::horizon_skinsrestorer_core::storage::MemorySql;
    use std::sync::Arc;

    const U: u128 = 0x1111;

    struct Fake(Option<(u128, String, Property)>, Option<Result<(Property, String), String>>);
    impl Mojang for Fake {
        fn profile(&self, _: &str) -> Result<Option<(u128, String, Property)>, String> {
            Ok(self.0.clone())
        }
        fn generate(&self, _: &str, v: Option<&str>) -> Result<(Property, String, String), String> {
            match &self.1 {
                Some(r) => r.clone().map(|(p, id)| (p, id, v.unwrap_or("CLASSIC").to_owned())),
                None => Err("MineSkin is overloaded. Try again later.".into()),
            }
        }
    }

    fn call(st: &Storage, online: &[(&str, u128)], mojang: &Fake, line: &str) -> (String, Vec<Effect>) {
        let online: HashMap<String, u128> = online.iter().map(|(n, u)| (n.to_ascii_lowercase(), *u)).collect();
        handle(&Ctx { storage: Some(st), online: &online, mojang, now_secs: 1000 }, line)
    }

    fn plain(st: &Storage, line: &str) -> String {
        call(st, &[], &Fake(None, None), line).0
    }

    fn setup() -> (Storage, Arc<MemorySql>) {
        storage()
    }

    #[test]
    fn version_and_unknown_ops_answer_as_documented() {
        let (st, _) = setup();
        assert_eq!(plain(&st, "available"), "true");
        assert_eq!(plain(&st, "version"), env!("CARGO_PKG_VERSION"));
        assert_eq!(plain(&st, "nope"), "error\tunknown op nope");
        assert_eq!(plain(&st, "get_skin_of_player"), "error\tget_skin_of_player needs argument 1");
        assert!(plain(&st, "get_skin_of_player\tzzz").starts_with("error\tnot a UUID"));
    }

    #[test]
    fn property_utils_read_the_texture_value() {
        let (st, _) = setup();
        let p = sample("abc123", true);
        assert_eq!(plain(&st, &format!("property_texture_url\t{}", p.value)), "http://textures.minecraft.net/texture/abc123");
        assert_eq!(plain(&st, &format!("property_texture_hash\t{}", p.value)), "abc123");
        assert_eq!(plain(&st, &format!("property_texture_url_stripped\t{}", p.value)), "abc123");
        assert_eq!(plain(&st, &format!("property_variant\t{}", p.value)), "SLIM");
        assert_eq!(plain(&st, &format!("property_variant\t{}", sample("abc123", false).value)), "CLASSIC");
        assert_eq!(plain(&st, "property_variant\tgarbage"), "none");
    }

    #[test]
    fn player_storage_reads_the_selection_and_its_skin() {
        let (st, mem) = setup();
        assert_eq!(plain(&st, &format!("get_skin_id_of_player\t{}", uuid_dashed(U))), "none");
        let p = crate::horizon_skinsrestorer_core::storage::tests::select_custom(&mem, U, "knight", "h1");
        let id = plain(&st, &format!("get_skin_id_of_player\t{}", uuid_dashed(U)));
        assert_eq!(id, "knight\tCUSTOM\t");
        assert_eq!(
            plain(&st, &format!("get_skin_of_player\t{}", uuid_dashed(U))),
            format!("{}\t{}", p.value, p.signature.unwrap())
        );
        assert_eq!(
            plain(&st, &format!("get_skin_id_for_player\t{}\tbob", uuid_dashed(U))),
            "knight\tCUSTOM\t"
        );
    }

    #[test]
    fn get_skin_for_player_falls_back_to_the_stored_then_the_fetched_account_skin() {
        let (st, mem) = setup();
        let own = sample("own", false);
        mem.put(PLAYER, &[&uuid_dashed(U)], vec![Some(own.value.clone()), own.signature.clone()]);
        let (r, _) = call(&st, &[], &Fake(None, None), &format!("get_skin_for_player\t{}\tbob", uuid_dashed(U)));
        assert_eq!(r, format!("{}\t{}", own.value, own.signature.clone().unwrap()));
        // Nothing stored: Mojang's answer is used and written.
        let other = 0x2222u128;
        let fetched = sample("fetched", false);
        let fake = Fake(Some((other, "carol".into(), fetched.clone())), None);
        let (r, _) = call(&st, &[], &fake, &format!("get_skin_id_for_player\t{}\tcarol", uuid_dashed(other)));
        assert_eq!(r, format!("{}\tPLAYER\t", uuid_dashed(other)));
        let ex = mem.executed.lock().unwrap();
        assert!(ex[0].0.contains("INSERT INTO sr_player_skins"));
        assert_eq!(ex[0].1[0].as_deref(), Some(uuid_dashed(other).as_str()));
        assert_eq!(ex[0].1[1].as_deref(), Some("carol"));
        assert_eq!(ex[0].1[4].as_deref(), Some("1000"));
    }

    #[test]
    fn the_writes_use_the_originals_statements() {
        let (st, mem) = setup();
        let u = uuid_dashed(U);
        assert_eq!(plain(&st, &format!("set_skin_id_of_player\t{u}\tKnight\tcustom")), "ok");
        assert_eq!(plain(&st, &format!("set_skin_id_of_player\t{u}\thttps://x/y.png\turl\tslim")), "ok");
        assert_eq!(plain(&st, &format!("remove_skin_id_of_player\t{u}")), "ok");
        assert_eq!(plain(&st, "set_custom_skin_data\tKnight\tVAL\tSIG"), "ok");
        assert_eq!(plain(&st, "set_url_skin_data\thttps://x/y.png\tmid\tVAL\tSIG\tCLASSIC"), "ok");
        assert_eq!(plain(&st, "set_url_skin_index\thttps://x/y.png\tCLASSIC"), "ok");
        assert_eq!(plain(&st, &format!("set_player_skin_data\t{u}\tbob\tVAL\tSIG\t77")), "ok");
        assert_eq!(plain(&st, "remove_skin_data\tknight\tCUSTOM"), "ok");
        assert_eq!(plain(&st, "remove_skin_data\thttps://x/y.png\tURL\tSLIM"), "ok");
        assert_eq!(plain(&st, &format!("remove_skin_data\t{u}\tPLAYER")), "ok");
        let ex = mem.executed.lock().unwrap();
        assert_eq!(ex.len(), 10);
        // The type name is the schema's, the custom name goes in lower case with the display name beside it.
        assert_eq!(ex[0].1[..4], [Some(u.clone()), Some("Knight".into()), Some("CUSTOM".into()), None]);
        assert_eq!(ex[1].1[3].as_deref(), Some("SLIM"));
        assert!(ex[2].0.contains("skin_identifier=NULL"));
        assert_eq!(ex[3].1[..3], [Some("knight".into()), Some("Knight".into()), Some("VAL".into())]);
        assert!(ex[4].0.starts_with("INSERT INTO sr_url_skins"));
        assert!(ex[5].0.starts_with("INSERT INTO sr_url_index"));
        assert_eq!(ex[7].0, "DELETE FROM sr_custom_skins WHERE name=?");
        assert_eq!(ex[8].1, [Some("https://x/y.png".into()), Some("SLIM".into())]);
        assert_eq!(ex[9].1, [Some(u)]);
    }

    #[test]
    fn bad_input_and_a_failing_database_are_errors() {
        let (st, mem) = setup();
        assert!(plain(&st, "set_skin_id_of_player\tx\ty\tz").starts_with("error"));
        assert_eq!(
            plain(&st, &format!("set_skin_id_of_player\t{}\ta\tbogus", uuid_dashed(U))),
            "error\tunknown skin type bogus"
        );
        assert_eq!(plain(&st, "set_url_skin_index\tu\tweird"), "error\tunknown skin variant weird");
        mem.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(plain(&st, "set_custom_skin_data\ta\tb\tc"), "error\tdatabase down");
        let online = HashMap::new();
        let none = Ctx { storage: None, online: &online, mojang: &Fake(None, None), now_secs: 0 };
        assert_eq!(handle(&none, "get_skin_id_of_player\t0000").0, "error\tnot a UUID: 0000");
        assert_eq!(
            handle(&none, &format!("get_skin_id_of_player\t{}", uuid_dashed(U))).0,
            "error\tno skin storage is configured"
        );
    }

    #[test]
    fn get_skin_data_and_find_skin_data_find_custom_url_and_player_skins() {
        let (st, mem) = setup();
        let c = sample("c1", false);
        mem.put(CUSTOM, &["knight"], vec![Some(c.value.clone()), c.signature.clone()]);
        let cp = format!("{}\t{}", c.value, c.signature.clone().unwrap());
        assert_eq!(plain(&st, "get_skin_data_by_identifier\tKnight\tCUSTOM"), cp);
        assert_eq!(plain(&st, "find_skin_data\tKnight"), format!("knight\tCUSTOM\t\t{cp}"));
        assert_eq!(plain(&st, "find_skin_data\tnobody"), "none");
        let u = sample("u1", true);
        mem.put(URL, &["https://x/y.png", "SLIM"], vec![Some(u.value.clone()), u.signature.clone()]);
        assert_eq!(
            plain(&st, "find_skin_data\thttps://x/y.png\tslim"),
            format!("https://x/y.png\tURL\tSLIM\t{}\t{}", u.value, u.signature.unwrap())
        );
        assert_eq!(plain(&st, "find_skin_data\thttps://x/none.png\tslim"), "none");
        assert_eq!(
            plain(&st, "find_or_create_skin_data\thttps://x/none.png\tslim"),
            "error\tMineSkin is overloaded. Try again later."
        );
        // Generated: stored with its id; without a variant the index is stored too.
        let g = sample("gen", false);
        let fake = Fake(None, Some(Ok((g.clone(), "ms1".into()))));
        let (r, _) = call(&st, &[], &fake, "find_or_create_skin_data\thttps://x/new.png");
        assert_eq!(r, format!("https://x/new.png\tURL\tCLASSIC\t{}\t{}", g.value, g.signature.clone().unwrap()));
        let ex = mem.executed.lock().unwrap();
        assert!(ex[0].0.starts_with("INSERT INTO sr_url_skins"));
        assert_eq!(ex[0].1[1].as_deref(), Some("ms1"));
        assert!(ex[1].0.starts_with("INSERT INTO sr_url_index"));
        drop(ex);
        let (r, _) = call(&st, &[], &Fake(None, Some(Err("x".into()))), "find_or_create_skin_data\thttps://x/n2.png\tslim");
        assert_eq!(r, "error\tx");
        // An online player's cached skin is found by name.
        let own = sample("own", false);
        mem.put(PLAYER, &[&uuid_dashed(U)], vec![Some(own.value.clone()), own.signature.clone()]);
        let (r, _) = call(&st, &[("Bob", U)], &Fake(None, None), "find_skin_data\tbob");
        assert_eq!(r, format!("{}\tPLAYER\t\t{}\t{}", uuid_dashed(U), own.value, own.signature.unwrap()));
    }

    #[test]
    fn find_or_create_and_get_player_skin_fetch_from_mojang_and_store() {
        let (st, mem) = setup();
        let p = sample("m1", false);
        let fake = Fake(Some((U, "bob".into(), p.clone())), None);
        let want = format!("{}\t{}\t{}", uuid_dashed(U), p.value, p.signature.clone().unwrap());
        assert_eq!(call(&st, &[], &fake, "get_player_skin\tbob").0, want);
        assert_eq!(
            call(&st, &[], &fake, "find_or_create_skin_data\tbob").0,
            format!("{}\tPLAYER\t\t{}\t{}", uuid_dashed(U), p.value, p.signature.clone().unwrap())
        );
        assert_eq!(mem.executed.lock().unwrap().len(), 2);
        assert_eq!(call(&st, &[], &fake, &format!("update_player_skin_data\t{}", uuid_dashed(U))).0, format!("{}\t{}", p.value, p.signature.unwrap()));
        assert_eq!(call(&st, &[], &fake, "get_uuid\tbob").0, uuid_dashed(U));
        assert_eq!(call(&st, &[("Bob", 5)], &Fake(None, None), "get_uuid\tbob").0, uuid_dashed(5));
        assert_eq!(call(&st, &[], &Fake(None, None), "get_player_skin\tghost").0, "none");
    }

    #[test]
    fn the_skin_apply_event_lines_and_verdicts() {
        let p = make_prop("VAL", "SIG");
        assert_eq!(apply_event(U, Some(&p)), format!("skin_apply\t{}\tVAL\tSIG", uuid_dashed(U)));
        assert_eq!(apply_event(U, None), format!("skin_apply\t{}\t\t", uuid_dashed(U)));
        assert_eq!(parse_verdict("cancel"), Verdict::Cancel);
        assert_eq!(parse_verdict("set\tA\tB"), Verdict::Set(Some(make_prop("A", "B"))));
        assert_eq!(parse_verdict("set\t\t"), Verdict::Set(None));
        assert_eq!(parse_verdict("whatever"), Verdict::Keep);
        let (st, _) = setup();
        let (r, fx) = call(&st, &[], &Fake(None, None), "subscribe\tpvpbot");
        assert_eq!((r.as_str(), fx), ("true", vec![Effect::Subscribe("pvpbot".into())]));
        assert_eq!(call(&st, &[], &Fake(None, None), "unsubscribe\tpvpbot").1, vec![Effect::Unsubscribe("pvpbot".into())]);
    }

    #[test]
    fn the_applier_queues_for_online_players_only() {
        let (st, mem) = setup();
        let u = uuid_dashed(U);
        let (r, fx) = call(&st, &[("Bob", U)], &Fake(None, None), &format!("apply_skin\t{u}\tVAL\tSIG"));
        assert_eq!(r, "true");
        assert_eq!(fx, vec![Effect::Apply(U, Some(make_prop("VAL", "SIG")))]);
        let (r, fx) = call(&st, &[], &Fake(None, None), &format!("apply_skin\t{u}\tVAL\tSIG"));
        assert_eq!((r.as_str(), fx.len()), ("false", 0));
        let (r, fx) = call(&st, &[("Bob", U)], &Fake(None, None), &format!("reset_skin\t{u}"));
        assert_eq!((r.as_str(), fx), ("true", vec![Effect::Apply(U, None)]));
        let c = crate::horizon_skinsrestorer_core::storage::tests::select_custom(&mem, U, "knight", "h9");
        let (r, fx) = call(&st, &[("Bob", U)], &Fake(None, None), &format!("apply_skin_of\t{u}"));
        assert_eq!((r.as_str(), fx), ("true", vec![Effect::Apply(U, Some(c.clone()))]));
        mem.put(CUSTOM, &["castle"], vec![Some(c.value.clone()), c.signature.clone()]);
        let (_, fx) = call(&st, &[("Bob", U)], &Fake(None, None), &format!("apply_skin_id\t{u}\tcastle\tCUSTOM"));
        assert_eq!(fx, vec![Effect::Apply(U, Some(c))]);
        assert_eq!(call(&st, &[("Bob", U)], &Fake(None, None), &format!("apply_skin_id\t{u}\tmissing\tCUSTOM")).0, "error\tno such skin");
    }
}
