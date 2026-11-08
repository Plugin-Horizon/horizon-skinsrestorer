//! Backend side of SkinsRestorer in proxy mode for Horizon servers. Derived from SkinsRestorer
//! (<https://github.com/SkinsRestorer/SkinsRestorer>, GPL-3.0).
//!
//! On a proxy network the proxy owns the skin commands and the selections, and forwards each
//! player's skin in the profile it logs them in with; Horizon already shows a forwarded skin.
//! This crate adds what the backend has to do on top:
//! - a join whose forwarded profile has no skin (or a server without a proxy) gets the stored
//!   selection from the shared SQL storage, read on the connection's thread before the player
//!   is sent their profile ([`profile`]);
//! - a skin the proxy sends on `sr:messagechannel` ([`payload`]) is applied and shown again: to
//!   the other players (player list entry and entity sent anew) and to the player (a respawn
//!   into the same level), with the original's dismount, remount and sound options
//!   ([`Action::Apply`]); a request for an ack is answered ([`Action::SendPayload`]);
//! - `/sr`, `/sr help`, `/sr status` and `/sr dump`, the commands a proxy-mode backend has.
//!
//! The engine glue (the bundle's helpers) turns [`Action`]s into packets.
pub mod config;
pub mod jobs;
#[cfg(feature = "mysql")]
pub mod mariadb;
#[cfg(feature = "net")]
pub mod net;
pub mod payload;
pub mod property;
pub mod sound;
pub mod storage;

use config::{Config, JoinLookup};
use jobs::{Answer, Done, Job, Jobs, Reason};
use property::Property;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use storage::{Lookup, Storage};

/// `ServerboundResourcePackPacket.Action.SUCCESSFULLY_LOADED`.
const PACK_LOADED: i32 = 0;

/// How a changed skin is shown again (the original's `server.*OnSkinUpdate` and `sound`).
#[derive(Clone, Debug, PartialEq)]
pub struct Refresh {
    pub dismount: bool,
    pub remount: bool,
    pub dismount_passengers: bool,
    pub sound: Option<sound::Sound>,
}

/// What the engine does for the mod.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Put `property` (None: no textures) into the player's profile and show it again.
    Apply {
        player: u128,
        property: Option<Property>,
        refresh: Refresh,
    },
    /// A plugin message to one player's connection (the ack the proxy asked for).
    SendPayload {
        player: u128,
        channel: String,
        data: Vec<u8>,
    },
    /// A line of a command's answer that came after the command returned (the service checks and
    /// the dump take a while): to a player, or to the console when `player` is None. Legacy
    /// colour codes.
    Tell { player: Option<u128>, line: String },
}

/// What the mod knows of an online player.
#[derive(Clone, Debug, Default)]
struct PlayerState {
    name: String,
    /// The textures the player's profile has now.
    shown: Option<Property>,
    /// The storage held a selection for them at the last look (a vanished one means /skin clear).
    had_selection: bool,
    /// A lookup is on its way.
    pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub lookups: u64,
    pub applied: u64,
    pub messages: u64,
    pub messages_with_skin: u64,
    pub last_error: Option<String>,
    pub last_ok: Option<Instant>,
}

/// A join's lookup done on the connection's thread.
#[derive(Clone, Debug)]
struct Prefetch {
    result: Result<Answer, String>,
}

pub struct Core {
    cfg: Config,
    behind_proxy: Option<bool>,
    storage: Option<Arc<Storage>>,
    jobs: Option<Box<dyn Jobs>>,
    players: HashMap<u128, PlayerState>,
    prefetched: HashMap<u128, Prefetch>,
    /// Lookups to start later: (when, player, why).
    later: Vec<(Instant, u128, Reason)>,
    next_poll: Option<Instant>,
    queued: Vec<Action>,
    /// Lines from command threads, taken by the next tick.
    outbox: Arc<Mutex<Vec<Action>>>,
    pub stats: Stats,
}

impl Core {
    pub fn new(cfg: Config, storage: Option<Arc<Storage>>, jobs: Option<Box<dyn Jobs>>) -> Self {
        Self {
            cfg,
            behind_proxy: None,
            storage,
            jobs,
            players: HashMap::new(),
            prefetched: HashMap::new(),
            later: Vec::new(),
            next_poll: None,
            queued: Vec::new(),
            outbox: Arc::new(Mutex::new(Vec::new())),
            stats: Stats::default(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    fn proxy_mode(&self) -> bool {
        self.cfg.proxy_mode(self.behind_proxy.unwrap_or(false))
    }

    fn refresh(&self) -> Refresh {
        let s = &self.cfg.server;
        Refresh {
            dismount: s.dismount_player_on_skin_update,
            remount: s.dismount_player_on_skin_update && s.remount_player_on_skin_update,
            dismount_passengers: s.dismount_passengers_on_skin_update,
            sound: s.sound.enabled.then(|| sound::parse(&s.sound.value)).flatten(),
        }
    }

    /// Whether a join with this forwarded skin needs the storage.
    pub fn wants_join_lookup(&self, forwarded: Option<&Property>) -> bool {
        if self.storage.is_none() {
            return false;
        }
        match self.cfg.join_lookup(self.proxy_mode()) {
            JoinLookup::Never => false,
            JoinLookup::Always => true,
            JoinLookup::Missing => forwarded.is_none(),
        }
    }

    /// The skin a lookup asks for, given what the player shows: None leaves it.
    fn wanted(answer: &Answer, had_selection: bool) -> Option<Option<Property>> {
        match &answer.lookup {
            Lookup::Skin(_, p) => Some(Some(p.clone())),
            // A selection that went away: back to the account's own skin, when it is known.
            Lookup::NoSelection if had_selection => answer.own.clone().map(Some),
            Lookup::NoSelection | Lookup::Dangling(_) => None,
        }
    }

    fn note_result(&mut self, r: &Result<Answer, String>) {
        self.stats.lookups += 1;
        match r {
            Ok(_) => self.stats.last_ok = Some(Instant::now()),
            Err(e) => {
                if self.stats.last_error.as_deref() != Some(e.as_str()) {
                    log::warn!("[SkinsRestorer] storage: {e}");
                }
                self.stats.last_error = Some(e.clone());
            }
        }
    }

    /// The player is admitted, not placed yet: the textures their profile should carry. Some
    /// replaces the forwarded ones before anyone is sent the profile; None keeps them.
    pub fn joining(
        &mut self,
        player: u128,
        name: &str,
        forwarded: Option<Property>,
        now: Instant,
    ) -> Option<Option<Property>> {
        let mut state = PlayerState {
            name: name.to_owned(),
            shown: forwarded.clone(),
            ..PlayerState::default()
        };
        let mut replace = None;
        if self.wants_join_lookup(forwarded.as_ref()) {
            match self.prefetched.remove(&player) {
                Some(Prefetch { result }) => {
                    self.note_result(&result);
                    if let Ok(answer) = &result {
                        state.had_selection = matches!(answer.lookup, Lookup::Skin(..));
                        if let Lookup::Skin(_, p) = &answer.lookup {
                            if forwarded.as_ref().map(|f| &f.value) != Some(&p.value) {
                                replace = Some(Some(p.clone()));
                                state.shown = Some(p.clone());
                            }
                        }
                    }
                }
                // The connection's lookup did not finish: read it now, show it when it comes.
                None => self.later.push((now, player, Reason::Join)),
            }
        }
        self.players.insert(player, state);
        replace
    }

    /// The connection's thread, before the player is admitted: what [`joining`] will need.
    /// Runs the lookup here (blocking, bounded by the driver's timeouts).
    pub fn prefetch_with(storage: &Storage, player: u128) -> Result<Answer, String> {
        jobs::answer(storage, player)
    }

    pub fn left(&mut self, player: u128) {
        self.players.remove(&player);
        self.prefetched.remove(&player);
        self.later.retain(|(_, p, _)| *p != player);
    }

    /// A player's client answered a resource pack (`status` as the game numbers it: 0 successfully
    /// loaded). `server.resourcePackFix`: a pack that finished loading hides the skin on the
    /// client, so the profile's current textures are shown again, without dismounting or a sound.
    pub fn resource_pack_status(&mut self, player: u128, status: i32) {
        if !self.cfg.server.resource_pack_fix || status != PACK_LOADED {
            return;
        }
        let Some(shown) = self.players.get(&player).and_then(|st| st.shown.clone()) else {
            return;
        };
        self.queued.push(Action::Apply {
            player,
            property: Some(shown),
            refresh: Refresh {
                dismount: false,
                remount: false,
                dismount_passengers: false,
                sound: None,
            },
        });
    }

    /// A plugin channel message from the proxy over `player`'s connection. True when it was on
    /// `sr:messagechannel` (the engine still lets other code see it).
    pub fn message(&mut self, player: u128, channel: &str, data: &[u8]) -> bool {
        if channel != payload::CHANNEL {
            return false;
        }
        self.stats.messages += 1;
        // Like the original: the work is for a player who is on.
        if !self.players.contains_key(&player) {
            return true;
        }
        match payload::decode(data) {
            Some(payload::ServerMessage::SkinUpdate { property, ack }) => {
                if property.is_some() {
                    self.stats.messages_with_skin += 1;
                }
                // SharedSkinApplier.applySkin: always applied (event, refresh, sound), even when
                // the player already shows it.
                self.apply(player, property);
                if let Some(ack) = ack {
                    // The proxy compares versions and warns on a difference; answering with its
                    // own keeps both logs quiet.
                    match payload::encode_ack(&ack) {
                        Some(data) => self.queued.push(Action::SendPayload {
                            player,
                            channel: payload::CHANNEL.to_owned(),
                            data,
                        }),
                        None => log::warn!("[SkinsRestorer] the proxy's version text is too long to answer"),
                    }
                }
            }
            Some(payload::ServerMessage::OpenGui) => {
                log::warn!("[SkinsRestorer] the proxy sent the skins menu; menus are not supported here (GAPS.md)")
            }
            Some(payload::ServerMessage::GiveSkull { .. }) => {
                log::warn!("[SkinsRestorer] the proxy sent a skull; giving skulls is not supported here (GAPS.md)")
            }
            Some(payload::ServerMessage::Unknown(kind)) => log::warn!(
                "[SkinsRestorer] Received unknown channel payload {kind:?} from proxy (Make sure the server and proxy are running the same version of SkinsRestorer)"
            ),
            None => log::warn!("[SkinsRestorer] could not read a message from the proxy on {}", payload::CHANNEL),
        }
        true
    }

    /// Queue an Apply whatever the player shows now.
    fn apply(&mut self, player: u128, p: Option<Property>) {
        let refresh = self.refresh();
        let Some(st) = self.players.get_mut(&player) else {
            return;
        };
        log::debug!("[SkinsRestorer] applying a skin to {}", st.name);
        st.shown = p.clone();
        self.stats.applied += 1;
        self.queued.push(Action::Apply {
            player,
            property: p,
            refresh,
        });
    }

    /// Queue an Apply when `p` differs from what the player shows.
    fn show(&mut self, player: u128, p: Option<Property>) {
        let refresh = self.refresh();
        let Some(st) = self.players.get_mut(&player) else {
            return;
        };
        if st.shown.as_ref().map(|x| &x.value) == p.as_ref().map(|x| &x.value) {
            return;
        }
        st.shown = p.clone();
        self.stats.applied += 1;
        self.queued.push(Action::Apply {
            player,
            property: p,
            refresh,
        });
    }

    fn handle(&mut self, d: Done) {
        self.note_result(&d.result);
        let Some(st) = self.players.get_mut(&d.player) else {
            return;
        };
        st.pending = false;
        let Ok(answer) = d.result else {
            return;
        };
        let wanted = Self::wanted(&answer, st.had_selection);
        st.had_selection = matches!(answer.lookup, Lookup::Skin(..));
        if let Some(p) = wanted {
            self.show(d.player, p);
        }
    }

    fn submit(&mut self, player: u128, reason: Reason) {
        let Some(jobs) = self.jobs.as_mut() else {
            return;
        };
        let Some(st) = self.players.get_mut(&player) else {
            return;
        };
        // One periodic lookup at a time; the others always go.
        if reason == Reason::Poll && st.pending {
            return;
        }
        st.pending = true;
        jobs.submit(Job { player, reason });
    }

    /// Once a server tick: finished lookups, due ones, the periodic check; what to apply.
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        let done = self.jobs.as_mut().map(|j| j.done()).unwrap_or_default();
        for d in done {
            self.handle(d);
        }
        let (due, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut self.later)
            .into_iter()
            .partition(|(at, _, _)| *at <= now);
        self.later = wait;
        for (_, p, r) in due {
            self.submit(p, r);
        }
        let every = self.cfg.horizon.poll_seconds;
        if every > 0 && self.storage.is_some() {
            match self.next_poll {
                Some(at) if at > now => {}
                Some(_) => {
                    let online: Vec<u128> = self.players.keys().copied().collect();
                    for p in online {
                        self.submit(p, Reason::Poll);
                    }
                    self.next_poll = Some(now + Duration::from_secs(every));
                }
                None => self.next_poll = Some(now + Duration::from_secs(every)),
            }
        }
        if let Ok(mut out) = self.outbox.lock() {
            self.queued.append(&mut out);
        }
        std::mem::take(&mut self.queued)
    }

    /// What `/sr status` prints after the service checks: version, server, proxy mode, commit.
    fn summary_lines(&self) -> Vec<String> {
        vec![
            DIVIDER.to_owned(),
            prefixed(&format!("\u{a7}7SkinsRestorer \u{a7}6v{}", env!("CARGO_PKG_VERSION"))),
            prefixed("\u{a7}7Server: \u{a7}6Horizon"),
            prefixed(&format!("\u{a7}7Proxy Mode: \u{a7}6{}", self.proxy_mode())),
            prefixed("\u{a7}7Commit: \u{a7}6horizon-port"),
            DIVIDER.to_owned(),
        ]
    }

    /// Runs a parsed `/sr` subcommand for `sender` (None: the console). The answer is the lines
    /// to print at once; `status` and `dump` answer later through [`Action::Tell`].
    pub fn run(&mut self, sender: Option<u128>, sub: &Sub) -> Vec<String> {
        match sub {
            Sub::Root => help_lines(""),
            Sub::Help(query) => help_lines(query),
            Sub::Status => {
                #[cfg(feature = "net")]
                {
                    let summary = self.summary_lines();
                    let cfg = self.cfg.clone();
                    let out = self.outbox.clone();
                    let spawned = std::thread::Builder::new()
                        .name("skinsrestorer-status".into())
                        .spawn(move || {
                            let lines = status_report(&cfg, summary);
                            if let Ok(mut o) = out.lock() {
                                o.extend(lines.into_iter().map(|line| Action::Tell { player: sender, line }));
                            }
                        });
                    if spawned.is_err() {
                        return vec![prefixed("\u{a7}cThe service checks could not be started.")];
                    }
                }
                #[cfg(not(feature = "net"))]
                {
                    let _ = sender;
                    return self.summary_lines();
                }
                vec![prefixed("\u{a7}7Running service checks...")]
            }
            Sub::Dump => {
                #[cfg(feature = "net")]
                {
                    let report = self.dump_report();
                    let cfg = self.cfg.clone();
                    let out = self.outbox.clone();
                    let spawned = std::thread::Builder::new()
                        .name("skinsrestorer-dump".into())
                        .spawn(move || {
                            let line = match net::upload_dump(&report, &cfg) {
                                Ok(Some(key)) => prefixed(&format!(
                                    "\u{a7}aSupport data uploaded: \u{a7}ehttps://bytebin.lucko.me/{key}"
                                )),
                                Ok(None) => prefixed(DUMP_ERROR),
                                Err(e) => {
                                    log::error!("[SkinsRestorer] Failed to dump data: {e}");
                                    prefixed(DUMP_ERROR)
                                }
                            };
                            if let Ok(mut o) = out.lock() {
                                o.push(Action::Tell { player: sender, line });
                            }
                        });
                    if spawned.is_err() {
                        return vec![prefixed(DUMP_ERROR)];
                    }
                }
                #[cfg(not(feature = "net"))]
                {
                    let _ = sender;
                    return vec![prefixed(DUMP_ERROR)];
                }
                vec![prefixed("\u{a7}aUploading data to bytebin.lucko.me...")]
            }
        }
    }

    /// The report `/sr dump` uploads: the original's `DumpInfo` without the Java and the
    /// OS-user parts (a Horizon server has no JVM, and the user's name and home folder are not
    /// something to send out).
    #[cfg(feature = "net")]
    fn dump_report(&self) -> serde_json::Value {
        serde_json::json!({
            "buildInfo": {
                "version": env!("CARGO_PKG_VERSION"),
                "commit": "horizon-port",
                "branch": null,
                "buildTime": null,
                "ciName": null,
                "ciBuildNumber": null,
            },
            "pluginInfo": {
                "proxyMode": self.proxy_mode(),
                // The original reports NONE for a proxy-mode server (no storage of its own).
                "storageType": if self.proxy_mode() { "NONE".to_owned() } else { self.cfg.database.kind.to_ascii_uppercase() },
                "configData": self.cfg.dump_json(),
            },
            "platformInfo": {
                "name": "Horizon",
                "vendor": "Horizon",
                "version": env!("CARGO_PKG_VERSION"),
                "plugins": [],
            },
            "osInfo": {
                "name": std::env::consts::OS,
                "version": null,
                "arch": std::env::consts::ARCH,
            },
        })
    }
}

const DUMP_ERROR: &str = "\u{a7}cSkinsRestorer failed to upload support data to bytebin.lucko.me.";

/// `Message.DIVIDER`: `<dark_aqua>` and 46 dashes, no prefix.
const DIVIDER: &str = "\u{a7}3----------------------------------------------";

/// `text` without legacy colour codes (for the console).
pub fn strip_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// The message prefix (`prefix_format`: `<yellow>[<dark_green>SkinsRestorer</dark_green>] <message>`)
/// in legacy codes; a message without a colour of its own is yellow.
pub fn prefixed(message: &str) -> String {
    format!("\u{a7}e[\u{a7}2SkinsRestorer\u{a7}e] \u{a7}e{message}")
}

/// `ServiceCheckerService` + `SRCommandService.executeStatus` after the first line: the failed
/// checks (all with `dev.debug`), the two counts, the verdict, the summary.
#[cfg(feature = "net")]
fn status_report(cfg: &Config, summary: Vec<String>) -> Vec<String> {
    use net::Kind;
    let checks = net::check_services(cfg);
    let mut out = vec![DIVIDER.to_owned()];
    for c in &checks {
        if !c.ok || cfg.dev.debug {
            out.push(c.text.clone());
        }
    }
    let ok = |k: Kind| checks.iter().filter(|c| c.kind == k && c.ok).count();
    let total = |k: Kind| checks.iter().filter(|c| c.kind == k).count();
    out.push(prefixed(&format!(
        "\u{a7}7Available UUID APIs: {}/{}",
        ok(Kind::Uuid),
        total(Kind::Uuid)
    )));
    out.push(prefixed(&format!(
        "\u{a7}7Available profile APIs: {}/{}",
        ok(Kind::Profile),
        total(Kind::Profile)
    )));
    if checks.iter().all(|c| c.ok) {
        out.push(prefixed("\u{a7}aAll required services are available."));
    } else if ok(Kind::Uuid) == 0 || ok(Kind::Profile) == 0 {
        out.push(prefixed("\u{a7}cAll required services are unavailable. You cannot request new skins."));
        out.push(prefixed(
            "\u{a7}cA firewall probably blocks the required connections.\nRead https://skinsrestorer.net/firewall for help.",
        ));
    } else {
        out.push(prefixed("\u{a7}eSome required services are unavailable. Some features will not work."));
    }
    out.extend(summary);
    out
}

/// A `/sr` subcommand of a proxy-mode backend (`SRProxyCommand`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sub {
    /// `/sr`
    Root,
    /// `/sr help [query]`
    Help(String),
    Status,
    Dump,
}

impl Sub {
    /// The permission nodes of which one lets a sender run this: the command's own, then the
    /// groups that contain it (`skinsrestorer.admin` and `skinsrestorer.admincommand.*`; `dump`
    /// is in neither group in the original, so it needs its own node).
    pub fn nodes(&self) -> &'static [&'static str] {
        match self {
            Sub::Root | Sub::Help(_) => &[
                "skinsrestorer.admincommand",
                "skinsrestorer.admin",
                "skinsrestorer.admincommand.*",
            ],
            Sub::Status => &[
                "skinsrestorer.admincommand.status",
                "skinsrestorer.admin",
                "skinsrestorer.admincommand.*",
            ],
            Sub::Dump => &["skinsrestorer.admincommand.dump"],
        }
    }
}

/// `args` is the line after `sr`. Err is the answer to a line that is not a command of this
/// backend (cloud's invalid-syntax message).
pub fn parse(args: &str) -> Result<Sub, String> {
    let mut words = args.split_whitespace();
    let Some(first) = words.next() else {
        return Ok(Sub::Root);
    };
    let rest: Vec<&str> = words.collect();
    match first.to_ascii_lowercase().as_str() {
        "help" => Ok(Sub::Help(rest.join(" "))),
        "status" if rest.is_empty() => Ok(Sub::Status),
        "dump" if rest.is_empty() => Ok(Sub::Dump),
        "status" | "dump" => Err(format!("/sr {}", first.to_ascii_lowercase())),
        _ => Err("/sr".into()),
    }
}

const NO_PERMISSION: &str = "\u{a7}cI'm sorry, but you do not have permission to perform this command. Please contact the server administrators if you believe that this is in error.";

fn invalid_syntax(correct: &str) -> String {
    format!("\u{a7}cInvalid Command Syntax. Correct command syntax is: \u{a7}7{correct}")
}

/// The commands `/sr help` lists: the ones with a description (`Message.HELP_*`).
const HELP: [(&str, &str); 3] = [
    ("/sr help [query]", "Admin commands for SkinsRestorer."),
    ("/sr status", "Checks the API services that SkinsRestorer uses."),
    ("/sr dump", "Uploads support data to bytebin.lucko.me."),
];

/// `MinecraftHelp` output of the original's cloud help, as a list: the guess is the layout, not
/// the text (cloud's own wording is not in the SkinsRestorer repository).
fn help_lines(query: &str) -> Vec<String> {
    let q = query.trim().to_ascii_lowercase();
    let mut out = vec![format!("\u{a7}6Help Menu: \u{a7}7/sr help {q}").trim_end().to_owned()];
    for (syntax, description) in HELP {
        let plain = syntax.trim_start_matches('/').to_ascii_lowercase();
        if q.is_empty() || plain.contains(&q) {
            out.push(format!("\u{a7}e{syntax} \u{a7}7- {description}"));
        }
    }
    out
}

// ---- the server's single instance -------------------------------------------------------

static CORE: OnceLock<Mutex<Core>> = OnceLock::new();

/// The config: the defaults, then the original's `plugins/SkinsRestorer/config.yml` when there is
/// one, then `config/skinsrestorer.toml` over both. Without either, the TOML file is written
/// with the defaults.
pub fn load_config(dir: &Path) -> Config {
    let yaml_path = dir.join("plugins").join("SkinsRestorer").join("config.yml");
    let yaml = std::fs::read_to_string(&yaml_path).ok();
    let path = dir.join("config").join("skinsrestorer.toml");
    let text = std::fs::read_to_string(&path).ok();
    if text.is_none() && yaml.is_none() {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(dir));
        if let Err(e) = std::fs::write(&path, config::DEFAULT) {
            log::warn!("[SkinsRestorer] could not write {}: {e}", path.display());
        }
    }
    match Config::parse_layers(yaml.as_deref(), text.as_deref().unwrap_or("")) {
        Ok(c) => c,
        Err(e) => {
            log::error!("[SkinsRestorer] the config: {e}; using the defaults");
            Config::default()
        }
    }
}

fn build(cfg: Config) -> Core {
    let storage = if cfg.database.kind.eq_ignore_ascii_case("MYSQL") {
        open_storage(&cfg)
    } else {
        // FILE is the original's default and what a proxy-mode backend normally has: the proxy
        // sends the skins, nothing is read from storage.
        if cfg.database.kind.eq_ignore_ascii_case("POSTGRESQL") {
            log::warn!("[SkinsRestorer] database.type POSTGRESQL is not readable here: only the proxy's skins are shown");
        }
        None
    };
    let jobs = storage
        .clone()
        .map(|s| Box::new(jobs::ThreadJobs::start(s)) as Box<dyn Jobs>);
    Core::new(cfg, storage, jobs)
}

#[cfg(feature = "mysql")]
fn open_storage(cfg: &Config) -> Option<Arc<Storage>> {
    let statements = match storage::Statements::new(cfg.horizon.schema.clone(), &cfg.database.table_prefix) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[SkinsRestorer] {e}");
            return None;
        }
    };
    let timeout = Duration::from_millis(cfg.horizon.join_timeout_ms.max(100));
    Some(Arc::new(Storage {
        statements,
        sql: Box::new(mariadb::MariaDb::new(&cfg.database, timeout)),
    }))
}

#[cfg(not(feature = "mysql"))]
fn open_storage(_: &Config) -> Option<Arc<Storage>> {
    log::warn!("[SkinsRestorer] built without the mysql feature");
    None
}

/// The instance, made on first use from the server folder's config.
pub fn with<R>(f: impl FnOnce(&mut Core) -> R) -> R {
    let core = CORE.get_or_init(|| {
        let dir = std::env::current_dir().unwrap_or_default();
        let mut core = build(load_config(&dir));
        core.behind_proxy = Some(detect_behind_proxy(&dir));
        Mutex::new(core)
    });
    let mut guard = core.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// Whether the server takes a proxy's forwarding, decided as the engine does at its start: a
/// `forwarding.secret` beside `server.properties` and `online-mode` not on (vanilla's default is
/// on when the line is missing). Only the secret file's presence is checked, never its content.
pub fn detect_behind_proxy(dir: &Path) -> bool {
    if !dir.join("forwarding.secret").exists() {
        return false;
    }
    let props = std::fs::read_to_string(dir.join("server.properties")).unwrap_or_default();
    let online = props
        .lines()
        .filter_map(|l| l.trim().strip_prefix("online-mode"))
        .filter_map(|rest| rest.trim_start().strip_prefix('='))
        .last()
        .map_or(true, |v| !v.trim().eq_ignore_ascii_case("false"));
    !online
}

/// Looks the player's skin up when the join will need it, so that [`joining`] has it without
/// waiting. Blocks the calling thread (a connection's, never the tick).
pub fn prefetch(player: u128, forwarded: Option<&Property>) {
    let storage = with(|c| {
        c.prefetched.remove(&player);
        c.wants_join_lookup(forwarded).then(|| c.storage.clone()).flatten()
    });
    // The lookup runs without the lock: the tick goes on meanwhile.
    if let Some(storage) = storage {
        let result = Core::prefetch_with(&storage, player);
        with(|c| c.prefetched.insert(player, Prefetch { result }));
    }
}

/// The connection's thread (`native_player_profile`): the player was let in, and nobody, their
/// own client included, has been sent the profile yet. Reads the stored skin when the join needs
/// it and gives the textures the profile should carry: Some replaces the forwarded ones (Some(None)
/// removes them), None keeps them.
pub fn profile(player: u128, name: &str, forwarded: Option<Property>) -> Option<Option<Property>> {
    prefetch(player, forwarded.as_ref());
    joining(player, name, forwarded)
}

/// The player admitted, the profile not sent yet: Some replaces the profile's textures.
pub fn joining(player: u128, name: &str, forwarded: Option<Property>) -> Option<Option<Property>> {
    with(|c| c.joining(player, name, forwarded, Instant::now()))
}

pub fn left(player: u128) {
    with(|c| c.left(player))
}

pub fn resource_pack_status(player: u128, status: i32) {
    with(|c| c.resource_pack_status(player, status))
}

pub fn message(player: u128, channel: &str, data: &[u8]) -> bool {
    with(|c| c.message(player, channel, data))
}

pub fn tick() -> Vec<Action> {
    with(|c| c.tick(Instant::now()))
}

/// `/sr ...` from `sender` (None: the console). `args` is the line after `sr`. `allowed(node)` answers a permission node for the sender; the console is
/// allowed everything. The lines are in legacy colour codes.
pub fn command(sender: Option<u128>, args: &str, allowed: &dyn Fn(&str) -> bool) -> Vec<String> {
    let sub = match parse(args) {
        Ok(s) => s,
        Err(correct) => return vec![invalid_syntax(&correct)],
    };
    if sender.is_some() && !sub.nodes().iter().any(|n| allowed(n)) {
        return vec![NO_PERMISSION.to_owned()];
    }
    with(|c| c.run(sender, &sub))
}

/// The permission nodes the `/sr` line needs (any one of them), for the command tree's own
/// check; empty for a line that is no command.
pub fn nodes(args: &str) -> &'static [&'static str] {
    parse(args).map(|s| s.nodes()).unwrap_or(&[])
}

pub fn admin_level() -> u8 {
    with(|c| c.config().horizon.admin_level)
}

#[cfg(test)]
mod tests;
