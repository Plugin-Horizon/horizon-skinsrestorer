//! The network calls of `/sr status` and `/sr dump`: the original's `ServiceCheckerService` and
//! `DumpService`. Blocking (ureq): they run on their own thread, never on the tick.
use crate::config::Config;
use crate::property::{parse_uuid, uuid_dashed, uuid_plain};
use std::time::Duration;

const UUID_ECLIPSE: &str = "https://eclipse.skinsrestorer.net/mojang/uuid/";
const PROFILE_ECLIPSE: &str = "https://eclipse.skinsrestorer.net/mojang/skin/";
const PROFILE_MOJANG: &str = "https://sessionserver.mojang.com/session/minecraft/profile/";
const PROFILE_ELYBY_SIGNED: &str = "https://skinsystem.ely.by/textures/signed/";
const BATCH_UUID_NEW: &str = "https://api.minecraftservices.com/minecraft/profile/lookup/bulk/byname";
const BYTEBIN_POST: &str = "https://bytebin.lucko.me/post";

/// The four accounts the original picks one of for the checks.
const PLAYERS: [(&str, &str); 4] = [
    ("xknat", "7dcfc130-344a-4719-9fbe-3176bc2075c6"),
    ("jeb_", "853c80ef-3c37-49fd-aa49-938b674adae6"),
    ("Dinnerbone", "61699b2e-d327-4a01-9f1e-0ea8c3f06bc6"),
    ("Grumm", "e6b5c088-0680-44df-9e1b-9bf11792291b"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Uuid,
    Profile,
}

/// One service check: a line (legacy colour codes) and whether it worked.
#[derive(Clone, Debug)]
pub struct Check {
    pub text: String,
    pub ok: bool,
    pub kind: Kind,
}

pub fn user_agent() -> String {
    format!("SkinsRestorer/{} (Horizon)", env!("CARGO_PKG_VERSION"))
}

struct Response {
    code: u16,
    body: String,
}

fn agent(timeout_ms: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(timeout_ms))
        .timeout_read(Duration::from_millis(timeout_ms))
        .user_agent(&user_agent())
        .build()
}

fn finish(r: Result<ureq::Response, ureq::Error>) -> Result<Response, String> {
    match r {
        Ok(r) => Ok(Response {
            code: r.status(),
            body: r.into_string().unwrap_or_default(),
        }),
        Err(ureq::Error::Status(code, r)) => Ok(Response {
            code,
            body: r.into_string().unwrap_or_default(),
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// `readURL`: GET, JSON accepted, 5 s.
fn get(url: &str, cfg: &Config) -> Result<Response, String> {
    if !cfg.connections_allowed() {
        return Err("Connections are disabled.".into());
    }
    finish(agent(5000).get(url).set("Accept", "application/json").call())
}

fn json(r: &Response) -> Option<serde_json::Value> {
    serde_json::from_str(&r.body).ok()
}

/// A property of a `MojangProfileResponse`: the first with a value and a signature.
fn first_property(v: &serde_json::Value, only_textures: bool) -> Option<(String, String)> {
    v.get("properties")?.as_array()?.iter().find_map(|p| {
        if only_textures && p.get("name")?.as_str()? != "textures" {
            return None;
        }
        let value = p.get("value")?.as_str()?;
        let signature = p.get("signature")?.as_str()?;
        (!value.is_empty() && !signature.is_empty()).then(|| (value.to_owned(), signature.to_owned()))
    })
}

/// The id the bulk lookup returns for `name` (an array of `{id, name}`).
fn bulk_id(r: &Response, name: &str) -> Option<u128> {
    let v = json(r)?;
    let entry = v
        .as_array()?
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.eq_ignore_ascii_case(name)))?;
    parse_uuid(entry.get("id")?.as_str()?)
}

fn short(value: &str) -> String {
    crate::property::Property::new(value, None).short_hash()
}

fn ok_uuid(service: &str, name: &str, uuid: &str) -> Check {
    Check {
        text: format!("{service} \u{a7}a\u{2714} {name} UUID: \u{a7}b{uuid}"),
        ok: true,
        kind: Kind::Uuid,
    }
}

fn ok_profile(service: &str, who: &str, value: &str) -> Check {
    Check {
        text: format!("{service} \u{a7}a\u{2714} {who} Profile: \u{a7}b{}", short(value)),
        ok: true,
        kind: Kind::Profile,
    }
}

fn failed(service: &str, what: &str, kind: Kind, why: Option<String>) -> Check {
    let text = match why {
        None => format!("{service} \u{a7}c\u{2718} Error getting {what}"),
        Some(w) => format!("{service} \u{a7}c\u{2718} Error getting {what}: {w}"),
    };
    Check {
        text,
        ok: false,
        kind,
    }
}

/// `ServiceCheckerService.checkServices`: Mojang and Eclipse UUID lookups, Mojang and Eclipse
/// profile lookups and (with `api.elyByEnabled`) Ely.by's profile, for one of four accounts.
pub fn check_services(cfg: &Config) -> Vec<Check> {
    let pick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize)
        % PLAYERS.len();
    let (name, uuid_text) = PLAYERS[pick];
    let uuid = parse_uuid(uuid_text).unwrap_or(0);
    let mut out = Vec::new();

    // Mojang UUID: the bulk name lookup.
    out.push(if !cfg.connections_allowed() {
        failed("Mojang", "UUID", Kind::Uuid, Some("Connections are disabled.".into()))
    } else {
        let body = serde_json::json!([name]).to_string();
        let r = finish(
            agent(5000)
                .post(BATCH_UUID_NEW)
                .set("Content-Type", "application/json")
                .set("Accept", "application/json")
                .send_string(&body),
        );
        match r {
            Err(e) => failed("Mojang", "UUID", Kind::Uuid, Some(e)),
            Ok(r) if r.code != 200 => failed("Mojang", "UUID", Kind::Uuid, Some(format!("Mojang error: {}", r.code))),
            Ok(r) => match bulk_id(&r, name) {
                Some(u) => ok_uuid("Mojang", name, &uuid_dashed(u)),
                None => failed("Mojang", "UUID", Kind::Uuid, None),
            },
        }
    });

    // Eclipse UUID.
    out.push(match get(&format!("{UUID_ECLIPSE}{name}"), cfg) {
        Err(e) => failed("Eclipse", "UUID", Kind::Uuid, Some(e)),
        Ok(r) if r.code != 200 => failed("Eclipse", "UUID", Kind::Uuid, Some(format!("Eclipse error: {}", r.code))),
        Ok(r) => match json(&r).and_then(|v| parse_uuid(v.get("uuid")?.as_str()?)) {
            Some(u) => ok_uuid("Eclipse", name, &uuid_dashed(u)),
            None => failed("Eclipse", "UUID", Kind::Uuid, None),
        },
    });

    // Mojang profile.
    out.push(
        match get(&format!("{PROFILE_MOJANG}{}?unsigned=false", uuid_plain(uuid)), cfg) {
            Err(e) => failed("Mojang", "Profile", Kind::Profile, Some(e)),
            Ok(r) if r.code == 204 || r.code == 404 => failed("Mojang", "Profile", Kind::Profile, None),
            Ok(r) if r.code != 200 => {
                failed("Mojang", "Profile", Kind::Profile, Some(format!("Mojang error: {}", r.code)))
            }
            Ok(r) => match json(&r).and_then(|v| first_property(&v, false)) {
                Some((value, _)) => ok_profile("Mojang", uuid_text, &value),
                None => failed("Mojang", "Profile", Kind::Profile, None),
            },
        },
    );

    // Eclipse profile.
    out.push(match get(&format!("{PROFILE_ECLIPSE}{uuid_text}"), cfg) {
        Err(e) => failed("Eclipse", "Profile", Kind::Profile, Some(e)),
        Ok(r) if r.code != 200 => failed("Eclipse", "Profile", Kind::Profile, Some(format!("Eclipse error: {}", r.code))),
        Ok(r) => match json(&r).and_then(|v| v.get("skinProperty")?.get("value")?.as_str().map(str::to_owned)) {
            Some(value) => ok_profile("Eclipse", uuid_text, &value),
            None => failed("Eclipse", "Profile", Kind::Profile, None),
        },
    });

    // Ely.by profile, by name (only when enabled).
    if cfg.api.ely_by_enabled {
        out.push(match get(&format!("{PROFILE_ELYBY_SIGNED}{name}"), cfg) {
            Err(e) => failed("Ely.by", "Profile", Kind::Profile, Some(e)),
            Ok(r) if r.code == 204 => failed("Ely.by", "Profile", Kind::Profile, None),
            Ok(r) if r.code != 200 => failed("Ely.by", "Profile", Kind::Profile, Some(format!("Ely.by error: {}", r.code))),
            Ok(r) => match json(&r).and_then(|v| first_property(&v, true)) {
                Some((value, _)) => ok_profile("Ely.by", name, &value),
                None => failed("Ely.by", "Profile", Kind::Profile, None),
            },
        });
    }
    out
}

/// `DumpService.dump`: posts the report to bytebin and returns its key; None when bytebin did
/// not answer 201. Err on a transport failure or with `advanced.noConnections`.
pub fn upload_dump(report: &serde_json::Value, cfg: &Config) -> Result<Option<String>, String> {
    if !cfg.connections_allowed() {
        return Err("Connections are disabled.".into());
    }
    let r = finish(
        agent(20_000)
            .post(BYTEBIN_POST)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_string(&report.to_string()),
    )?;
    if r.code != 201 {
        log::warn!("[SkinsRestorer] Failed to dump data to bytebin. Response code: {}", r.code);
        return Ok(None);
    }
    Ok(json(&r).and_then(|v| v.get("key")?.as_str().map(str::to_owned)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_list_is_valid() {
        for (_, u) in PLAYERS {
            assert!(parse_uuid(u).is_some());
        }
    }

    #[test]
    fn a_profile_property_needs_value_and_signature() {
        let v = serde_json::json!({"properties":[{"name":"x","value":"a","signature":"b"},{"name":"textures","value":"v","signature":"s"}]});
        assert_eq!(first_property(&v, false), Some(("a".into(), "b".into())));
        assert_eq!(first_property(&v, true), Some(("v".into(), "s".into())));
        let v = serde_json::json!({"properties":[{"name":"textures","value":"v","signature":""}]});
        assert_eq!(first_property(&v, false), None);
    }

    #[test]
    fn no_connections_fails_every_check_without_a_request() {
        let mut c = Config::default();
        c.advanced.no_connections = true;
        c.api.ely_by_enabled = true;
        let checks = check_services(&c);
        assert_eq!(checks.len(), 5);
        assert!(checks.iter().all(|c| !c.ok && c.text.contains("Connections are disabled.")));
    }
}
