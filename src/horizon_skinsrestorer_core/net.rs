//! The network calls of `/sr status` and `/sr dump`: the original's `ServiceCheckerService` and
//! `DumpService`. Blocking (ureq): they run on their own thread, never on the tick.
use crate::horizon_skinsrestorer_core::config::Config;
use crate::horizon_skinsrestorer_core::property::{parse_uuid, uuid_dashed, uuid_plain};
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

/// `MojangAPI.getSkin`: an account's name or UUID to its UUID, last name and signed skin (None: no such
/// account or no skin). Mojang's lookup by name, then the session server's profile.
pub fn mojang_profile(
    input: &str,
    cfg: &Config,
) -> Result<Option<(u128, String, crate::horizon_skinsrestorer_core::property::Property)>, String> {
    let uuid = match parse_uuid(input) {
        Some(u) => u,
        None => {
            if input.is_empty() || !input.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Ok(None);
            }
            let r = get(&format!("https://api.minecraftservices.com/minecraft/profile/lookup/name/{input}"), cfg)?;
            match r.code {
                200 => match json(&r).and_then(|v| parse_uuid(v.get("id")?.as_str()?)) {
                    Some(u) => u,
                    None => return Ok(None),
                },
                204 | 404 => return Ok(None),
                c => return Err(format!("Mojang answered {c}")),
            }
        }
    };
    let r = get(&format!("{PROFILE_MOJANG}{}?unsigned=false", uuid_plain(uuid)), cfg)?;
    match r.code {
        200 => {
            let v = json(&r).ok_or("unreadable profile")?;
            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_owned();
            Ok(first_property(&v, true).map(|(value, sig)| {
                (uuid, name, crate::horizon_skinsrestorer_core::property::Property::new(value, Some(sig)))
            }))
        }
        204 | 404 => Ok(None),
        c => Err(format!("Mojang answered {c}")),
    }
}

fn short(value: &str) -> String {
    crate::horizon_skinsrestorer_core::property::Property::new(value, None).short_hash()
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

// ---- MineSkin (`MineSkinAPIImpl`): a skin generated from an image URL ----

pub const MINESKIN_ENDPOINT: &str = "https://api.mineskin.org/v2/generate";
pub const AXOLOTL_ENDPOINT: &str = "https://axolotl.skinsrestorer.net/mineskin/decrypt-url";
const MINESKIN_AGENT: &str = "SkinsRestorer/MineSkinAPI";
const AXOLOTL_PREFIX: &str = "skinsrestorer-axolotl://";
const MINESKIN_RETRIES: u32 = 5;

/// The English texts of the original's `Message` constants a MineSkin failure ends in
/// (`locales/locale.json`).
pub const MS_FULL: &str = "MineSkin timed out while it uploaded your skin. Try again later.";
pub const MS_API_FAILED: &str = "MineSkin is overloaded. Try again later.";
pub const MS_API_KEY_INVALID: &str = "The MineSkin API key is invalid. Contact the server owner.";
pub const INVALID_URLSKIN: &str = "The skin URL or file format is invalid.";

/// `SRHelpers.sanitizeImageURL`: a NameMC skin page becomes its image.
pub fn sanitize_image_url(url: &str) -> String {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    if let Some(rest) = rest {
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.split(':').next().unwrap_or("");
        if host == "namemc.com" || host.ends_with(".namemc.com") {
            if let Some(id) = path.strip_prefix("skin/") {
                return format!("https://s.namemc.com/i/{id}.png");
            }
        }
    }
    url.to_owned()
}

/// What a generated skin is: the signed property, MineSkin's id and the variant it came out as.
#[derive(Clone, Debug)]
pub struct Generated {
    pub property: crate::horizon_skinsrestorer_core::property::Property,
    pub mine_skin_id: String,
    pub variant: &'static str,
}

/// Where the requests go and how waiting is done (a test points them at a local server and skips the sleeps).
pub struct MineSkinEnv<'a> {
    pub endpoint: &'a str,
    pub axolotl: &'a str,
    pub sleep: &'a dyn Fn(Duration),
}

impl MineSkinEnv<'static> {
    pub fn live() -> Self {
        Self { endpoint: MINESKIN_ENDPOINT, axolotl: AXOLOTL_ENDPOINT, sleep: &std::thread::sleep }
    }
}

fn axolotl_url(url: &str, env: &MineSkinEnv, cfg: &Config) -> Result<String, String> {
    if !url.starts_with(AXOLOTL_PREFIX) {
        return Ok(url.to_owned());
    }
    let enc: String = url
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    if !cfg.connections_allowed() {
        return Err("Connections are disabled.".into());
    }
    let r = finish(
        agent(30_000)
            .get(&format!("{}?encryptedUrl={enc}", env.axolotl))
            .set("User-Agent", MINESKIN_AGENT)
            .set("Accept", "application/json")
            .call(),
    )?;
    let err_of = |r: &Response, fallback: &str| {
        json(r).and_then(|v| v.get("error")?.as_str().map(str::to_owned)).unwrap_or_else(|| fallback.to_owned())
    };
    match r.code {
        200 => json(&r)
            .and_then(|v| v.get("url")?.as_str().map(str::to_owned))
            .filter(|u| !u.is_empty())
            .ok_or_else(|| "Invalid decrypt response: missing URL".into()),
        400 => Err(format!("Failed to decrypt axolotl URL: {}", err_of(&r, "Malformed ciphertext"))),
        500 => Err(format!("Axolotl decrypt service error: {}", err_of(&r, "Server configuration error"))),
        c => Err(format!("Unexpected decrypt response: {c}")),
    }
}

/// `MineSkinAPIImpl.genSkin`: POST the image URL to MineSkin (with `Authorization: Bearer <key>` when
/// `api.mineskinAPIKey` is set to something other than `""`/`"key"`), retry as the original does and map
/// failures to its messages. `variant` is `CLASSIC`, `SLIM` or None (MineSkin decides). The key is never
/// logged. Err holds the text the original's exception ends in.
pub fn mineskin_generate(
    image_url: &str,
    variant: Option<&str>,
    cfg: &Config,
    env: &MineSkinEnv,
) -> Result<Generated, String> {
    use crate::horizon_skinsrestorer_core::property::Property;
    if !cfg.connections_allowed() {
        return Err("Connections are disabled.".into());
    }
    let url = sanitize_image_url(&axolotl_url(image_url, env, cfg)?);
    let key = cfg.api.mineskin_api_key.get();
    let body = serde_json::json!({
        "variant": match variant { Some("SLIM") => "slim", Some("CLASSIC") => "classic", _ => "unknown" },
        "visibility": if cfg.api.mineskin_secret_skins { "unlisted" } else { "public" },
        "url": url,
    })
    .to_string();
    let mut next_at = std::time::Instant::now();
    for _ in 0..MINESKIN_RETRIES {
        let wait = next_at.saturating_duration_since(std::time::Instant::now());
        if !wait.is_zero() {
            (env.sleep)(wait);
        }
        let mut last = Err(String::new());
        for _ in 0..3 {
            let mut req = agent(90_000)
                .post(env.endpoint)
                .set("User-Agent", MINESKIN_AGENT)
                .set("Content-Type", "application/json")
                .set("Accept", "application/json");
            if !key.is_empty() && key != "key" {
                req = req.set("Authorization", &format!("Bearer {key}"));
            }
            last = finish(req.send_string(&body));
            if last.is_ok() {
                break;
            }
        }
        let r = last?;
        if !matches!(r.code, 200 | 400 | 403 | 429 | 500) {
            return Err(format!("MineSkin error: {}", r.code));
        }
        let v = json(&r).ok_or_else(|| format!("Empty MineSkin response: {}", r.code))?;
        if let Some(ms) = v.pointer("/rateLimit/next/relative").and_then(|x| x.as_u64()) {
            next_at = next_at.max(std::time::Instant::now() + Duration::from_millis(ms));
        }
        if r.code == 200 && v.get("success").and_then(|s| s.as_bool()) == Some(true) {
            let skin = v.get("skin").ok_or("Missing MineSkin skin")?;
            let data = skin.pointer("/texture/data").ok_or("Missing MineSkin texture")?;
            let property = Property::new(
                data.get("value").and_then(|x| x.as_str()).unwrap_or(""),
                data.get("signature").and_then(|x| x.as_str()).map(str::to_owned),
            );
            let slim = property.textures().is_some_and(|t| t.slim);
            return Ok(Generated {
                property,
                mine_skin_id: skin.get("uuid").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
                variant: if slim { "SLIM" } else { "CLASSIC" },
            });
        }
        let Some(errors) = v.get("errors").and_then(|e| e.as_array()) else {
            return Err(format!("Missing MineSkin errors: {}", r.code));
        };
        let Some(first) = errors.first() else {
            return Err(MS_API_FAILED.into());
        };
        match first.get("code").and_then(|c| c.as_str()).unwrap_or("") {
            "rate_limit" => {}
            "failed_to_create_id" | "skin_change_failed" => {
                next_at = next_at.max(std::time::Instant::now() + Duration::from_secs(6));
            }
            "no_account_available" => return Err(MS_FULL.into()),
            "invalid_api_key" => {
                match first.get("message").and_then(|m| m.as_str()).unwrap_or("") {
                    "Invalid API Key" => log::error!("[SkinsRestorer] The API Key provided is not registered on MineSkin! Please empty \"api.mineskinAPIKey\" in plugins/SkinsRestorer/config.yml and run /sr reload"),
                    "Client not allowed" => log::error!("[SkinsRestorer] This server ip is not on the api key allowed IPs list!"),
                    "Origin not allowed" => log::error!("[SkinsRestorer] This server Origin is not on the api key allowed Origins list!"),
                    "Agent not allowed" => log::error!("[SkinsRestorer] SkinsRestorer's agent \"{MINESKIN_AGENT}\" is not on the api key allowed agents list!"),
                    _ => log::error!("[SkinsRestorer] MineSkin API key is invalid, unknown error, please report this to SkinsRestorer's Discord!"),
                }
                return Err(MS_API_KEY_INVALID.into());
            }
            _ => return Err(INVALID_URLSKIN.into()),
        }
    }
    Err(MS_API_FAILED.into())
}

#[cfg(test)]
mod mineskin_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A local server answering the canned `(status, body)` list in order; it records the raw requests.
    fn serve(replies: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for (code, body) in replies {
                let Ok((mut c, _)) = l.accept() else { return };
                let mut buf = vec![0u8; 8192];
                let mut got = Vec::new();
                loop {
                    let n = c.read(&mut buf).unwrap_or(0);
                    got.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&got).to_string();
                    if let Some(h) = t.find("\r\n\r\n") {
                        let len = t[..h]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if got.len() >= h + 4 + len || n == 0 {
                            break;
                        }
                    } else if n == 0 {
                        break;
                    }
                }
                s2.lock().unwrap().push(String::from_utf8_lossy(&got).to_string());
                let _ = write!(c, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (format!("http://{addr}/v2/generate"), seen)
    }

    fn ok_body() -> String {
        let p = crate::horizon_skinsrestorer_core::property::tests::sample("g1", true);
        serde_json::json!({"success": true, "skin": {"uuid": "ms-id", "texture": {"data": {"value": p.value, "signature": p.signature}}}}).to_string()
    }

    fn err_body(code: &str, msg: &str) -> String {
        serde_json::json!({"success": false, "errors": [{"code": code, "message": msg}]}).to_string()
    }

    fn run(replies: Vec<(u16, String)>, cfg: &Config) -> (Result<Generated, String>, Vec<String>, usize) {
        let (url, seen) = serve(replies);
        let slept = Mutex::new(0usize);
        let sleep = |_: Duration| *slept.lock().unwrap() += 1;
        let env = MineSkinEnv { endpoint: &url, axolotl: "http://127.0.0.1:1/x", sleep: &sleep };
        let r = mineskin_generate("https://example.test/a.png", Some("SLIM"), cfg, &env);
        let n = *slept.lock().unwrap();
        let s = seen.lock().unwrap().clone();
        (r, s, n)
    }

    #[test]
    fn a_generated_skin_is_returned_and_the_request_has_the_originals_shape() {
        let (r, seen, _) = run(vec![(200, ok_body())], &Config::default());
        let g = r.unwrap();
        assert_eq!((g.mine_skin_id.as_str(), g.variant), ("ms-id", "SLIM"));
        let req = &seen[0];
        assert!(req.starts_with("POST /v2/generate"));
        assert!(req.to_ascii_lowercase().contains("user-agent: skinsrestorer/mineskinapi"));
        assert!(!req.to_ascii_lowercase().contains("authorization"));
        assert!(req.contains(r#""variant":"slim""#) && req.contains(r#""visibility":"public""#));
        assert!(req.contains(r#""url":"https://example.test/a.png""#));
    }

    #[test]
    fn a_key_goes_as_bearer_and_the_placeholder_key_does_not() {
        let cfg = Config::parse_layers(Some("api:\n  mineskinAPIKey: fake-test-key\n  mineskinSecretSkins: true\n"), "").unwrap();
        let (_, seen, _) = run(vec![(200, ok_body())], &cfg);
        assert!(seen[0].contains("Bearer fake-test-key"));
        assert!(seen[0].contains(r#""visibility":"unlisted""#));
        let cfg = Config::parse_layers(Some("api:\n  mineskinAPIKey: key\n"), "").unwrap();
        let (_, seen, _) = run(vec![(200, ok_body())], &cfg);
        assert!(!seen[0].to_ascii_lowercase().contains("authorization"));
    }

    #[test]
    fn rate_limits_are_retried_up_to_five_times_then_overloaded() {
        let rl = err_body("rate_limit", "slow down");
        let (r, seen, slept) = run(vec![(429, rl.clone()), (429, rl.clone()), (200, ok_body())], &Config::default());
        assert!(r.is_ok());
        assert_eq!(seen.len(), 3);
        let _ = slept;
        let (r, seen, _) = run(vec![(429, rl.clone()); 5], &Config::default());
        assert_eq!(r.unwrap_err(), MS_API_FAILED);
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn the_error_codes_map_to_the_originals_messages() {
        let cfg = Config::default();
        assert_eq!(run(vec![(400, err_body("no_account_available", ""))], &cfg).0.unwrap_err(), MS_FULL);
        assert_eq!(run(vec![(403, err_body("invalid_api_key", "Invalid API Key"))], &cfg).0.unwrap_err(), MS_API_KEY_INVALID);
        assert_eq!(run(vec![(400, err_body("invalid_image", ""))], &cfg).0.unwrap_err(), INVALID_URLSKIN);
        assert_eq!(run(vec![(418, "{}".into())], &cfg).0.unwrap_err(), "MineSkin error: 418");
        assert_eq!(run(vec![(500, "{}".into())], &cfg).0.unwrap_err(), "Missing MineSkin errors: 500");
        let (r, _, slept) = run(vec![(400, err_body("failed_to_create_id", "")), (200, ok_body())], &cfg);
        assert!(r.is_ok());
        assert_eq!(slept, 1);
    }

    #[test]
    fn namemc_pages_become_their_image() {
        assert_eq!(sanitize_image_url("https://namemc.com/skin/abc123"), "https://s.namemc.com/i/abc123.png");
        assert_eq!(sanitize_image_url("https://example.test/skin/abc"), "https://example.test/skin/abc");
        assert_eq!(sanitize_image_url("not a url"), "not a url");
    }
}
