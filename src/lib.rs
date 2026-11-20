//! SkinsRestorer (the backend side in proxy mode) as an arismoko native mod (trait mod on `mod_api`).
//! The logic is the unchanged `horizon-skinsrestorer-core` crate; this file only wires it to hooks
//! and host methods. Every gap on arismoko is a `TODO(arismoko-api: <name>)` and PORT_NOTES.md lists them.
//!
//! - Config: the core reads `plugins/SkinsRestorer/config.yml` when it exists, else
//!   `config/skinsrestorer.toml` (the core writes the defaults there on first start). Its defaults use
//!   `database.type = FILE`, so no MariaDB is opened.
//! - `/sr` and `/skinsrestorer` (`help`, `status`, `dump`): `register_commands` + `run_mod_command`, for
//!   players and the console. The nodes of the original's permission registry are declared at gathering
//!   (`DECLARED_NODES`, with Bukkit's defaults); `/sr` checks its admin nodes (`ADMIN_NODES`) with `permission_bool`.
//! - The skins menu: `menu_clicked`, `menu_closed` and `host.open_menu` / `host.close_menu`. Items have
//!   no names or lore (HMOD-GAP(item-display)); head textures and the glint are still missing.
//! - Skin changes: `server_tick` drains the core's actions. Writing the profile textures and refreshing
//!   the player have no API on main (HMOD-GAP(profile-write), HMOD-GAP(player-refresh)), so no skin is
//!   applied. A reset to the default skin is still missing.
//! - The proxy's plugin messages: `sr:messagechannel` is a raw play channel (`play_raw`, arismoko #3209);
//!   skin updates arrive through its handler and the ack goes out with `player.send_payload`.

pub mod horizon_skinsrestorer_core;
use crate::horizon_skinsrestorer_core::gui::MenuItem;
use crate::horizon_skinsrestorer_core::property::Property;
use crate::horizon_skinsrestorer_core::sound::Sound;
use crate::horizon_skinsrestorer_core::{self as core, Action, Refresh};
use mod_api::{
    Command, CommandArgumentValue, CommandSource, Flow, Hooks, Identifier, ItemStack,
    IPayloadContext, IPayloadHandler, MenuClickedEvent, MenuClosedEvent, MenuReplace, NodeDefault,
    PayloadBytes, PayloadHandleError, PayloadId, PermissionGatherNodesEvent, PermissionLevel,
    PermissionNode, PermissionValueType, PlayerCommandsSentEvent, PlayerLoggedInEvent, PlayerLoggedOutEvent,
    RegisterCommandsEvent, Registrar, RegistrationContext, ResourcePackStatusEvent,
    RunModCommandEvent, ServerAboutToStartEvent, ServerHost, ServerTickEvent, ServerTickPhase,
    SoundSource, Suggestion, TabCompleteEvent, Text,
};

/// The proxy's channel (`sr:messagechannel`): skin updates in, the ack out.
const PROXY_CHANNEL: &str = "sr:messagechannel";

/// The admin nodes of `/sr` (the core's `Sub::nodes`, and the Tab filter). Each is a typed Boolean node declared
/// at gathering with the operator level 2 as its default. The old `skinsrestorer.admincommand.*` wildcard is not
/// queried: a node name cannot contain `*`, so only the exact nodes are asked.
const ADMIN_NODES: [&str; 4] = [
    "skinsrestorer.admincommand",
    "skinsrestorer.admin",
    "skinsrestorer.admincommand.status",
    "skinsrestorer.admincommand.dump",
];

/// Every node of the original's `PermissionRegistry` plus its two groups (`skinsrestorer.player` and `skinsrestorer.admin`),
/// with the default Bukkit gives them: the children of the player group are true for everyone (level 0), all others
/// are operator-only (level 2). The groups' wildcards (`skinsrestorer.command.*`, `skinsrestorer.admincommand.*`) cannot be
/// node names here; a permission plugin that grants a wildcard still needs to grant the nodes below.
const DECLARED_NODES: &[(&str, u8)] = &[
    ("skinsrestorer.player", 0),
    ("skinsrestorer.admin", 2),
    ("skinsrestorer.command", 0),
    ("skinsrestorer.command.skull", 2),
    ("skinsrestorer.command.gui", 0),
    ("skinsrestorer.command.set", 0),
    ("skinsrestorer.command.set.other", 2),
    ("skinsrestorer.command.set.url", 0),
    ("skinsrestorer.command.clear", 0),
    ("skinsrestorer.command.clear.other", 2),
    ("skinsrestorer.command.random", 0),
    ("skinsrestorer.command.random.other", 2),
    ("skinsrestorer.command.search", 0),
    ("skinsrestorer.command.edit", 0),
    ("skinsrestorer.command.update", 0),
    ("skinsrestorer.command.update.other", 2),
    ("skinsrestorer.command.undo", 0),
    ("skinsrestorer.command.undo.other", 2),
    ("skinsrestorer.command.favourite", 0),
    ("skinsrestorer.command.favourite.other", 2),
    ("skinsrestorer.admincommand", 2),
    ("skinsrestorer.admincommand.skull.get", 2),
    ("skinsrestorer.admincommand.skull.get.other", 2),
    ("skinsrestorer.admincommand.skull.get.url", 2),
    ("skinsrestorer.admincommand.skull.random", 2),
    ("skinsrestorer.admincommand.skull.random.other", 2),
    ("skinsrestorer.admincommand.reload", 2),
    ("skinsrestorer.admincommand.status", 2),
    ("skinsrestorer.admincommand.drop", 2),
    ("skinsrestorer.admincommand.info", 2),
    ("skinsrestorer.admincommand.applyskin", 2),
    ("skinsrestorer.admincommand.createcustom", 2),
    ("skinsrestorer.admincommand.purgeolddata", 2),
    ("skinsrestorer.admincommand.dump", 2),
    ("skinsrestorer.bypasscooldown", 2),
    ("skinsrestorer.bypassdisabled", 2),
    ("skinsrestorer.ownskin", 2),
];

/// The typed node for one admin node name.
fn admin_node(name: &str) -> PermissionNode {
    PermissionNode::new(name, PermissionValueType::Boolean)
}

#[derive(Default)]
struct Mod;

/// The proxy's messages on `sr:messagechannel` (skin updates), for the core.
struct ProxyInbox;

impl IPayloadHandler for ProxyInbox {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), PayloadHandleError> {
        let player = context
            .player()
            .map_err(|e| PayloadHandleError::Failed(format!("{e:?}")))?;
        let _ = core::message(player.uuid(), PROXY_CHANNEL, &payload.0);
        Ok(())
    }
}

fn log(line: &str) {
    eprintln!("[SkinsRestorer] {line}");
}

/// A chat line in legacy colour codes (`host.text_from_legacy`). A line the host refuses keeps its words.
fn plain(host: &dyn ServerHost, line: &str) -> Text {
    host.text_from_legacy(line)
        .unwrap_or_else(|_| Text::literal(core::strip_codes(line)))
}

/// A JSON text component (the menu's title, item names and lore), as `host.text_from_json` reads it.
/// A value the host refuses falls back to its plain words.
fn json_text(host: &dyn ServerHost, json: &str) -> Text {
    match host.text_from_json(json) {
        Ok(Some(text)) => text,
        _ => Text::literal(json_plain(json)),
    }
}

/// The plain text of a JSON text component (a bare string, or an object with `text` and `extra`).
fn json_plain(json: &str) -> String {
    fn walk(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            serde_json::Value::Object(map) => {
                if let Some(text) = map.get("text") {
                    walk(text, out);
                }
                if let Some(extra) = map.get("extra") {
                    walk(extra, out);
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => walk(&value, &mut out),
        Err(_) => out.push_str(json),
    }
    core::strip_codes(&out)
}

/// The engine item an id names (`minecraft:player_head`), as one stack. None for an unknown id.
fn item_stack(host: &mut dyn ServerHost, id: &str) -> Option<ItemStack> {
    let ident = Identifier::parse(id).ok()?;
    let key = host.item_key(&ident)?;
    ItemStack::new(key, 1)
}

/// The sound category of the core (`Sound.category`, the engine's spelling) as arismoko's enum.
fn sound_source(category: &str) -> Option<SoundSource> {
    Some(match category {
        "master" => SoundSource::Master,
        "music" => SoundSource::Music,
        "record" => SoundSource::Records,
        "weather" => SoundSource::Weather,
        "block" => SoundSource::Blocks,
        "hostile" => SoundSource::Hostile,
        "neutral" => SoundSource::Neutral,
        "player" => SoundSource::Players,
        "ambient" => SoundSource::Ambient,
        "voice" => SoundSource::Voice,
        "ui" => SoundSource::Ui,
        _ => return None,
    })
}

/// Puts `property` into the player's profile, so that every list entry and the player's own client carry
/// it. A reset to the default skin (None) has no API on arismoko: it answers false and the action stops
/// here, so the skin stays as it was.
// TODO(arismoko-api: player_clear_profile_property): a None property (no textures) cannot be written;
// the reset to the default skin is skipped.
// HMOD-GAP(profile-write): main has no setter for a player's profile properties (`textures`). Every skin
// change is refused here, so no skin is applied at all; the core's actions still run without it.
fn set_profile_textures(host: &mut dyn ServerHost, player: u128, property: Option<&Property>) -> bool {
    let Some(p) = host.player(player) else {
        return false;
    };
    // Needs `player_set_profile_property` (arismoko/horizon branch gap-profiles, not on main yet).
    #[cfg(feature = "horizon_profile_api")]
    {
        return match property {
            Some(prop) => p.set_profile_property(host, "textures", &prop.value, prop.signature.as_deref()),
            // A reset to the default skin has no clear call: an empty value shows the default skin.
            None => p.set_profile_property(host, "textures", "", None),
        };
    }
    #[cfg(not(feature = "horizon_profile_api"))]
    {
        let _ = (p, property);
        false
    }
}

/// Shows the player's changed skin to everyone who sees them (`player_refresh`: hidden and shown again
/// over two ticks). Their own view keeps the old skin until they change level or join again.
// HMOD-GAP(player-refresh): main has no per-player tracker refresh; the new skin is not shown to others.
fn refresh_player(host: &mut dyn ServerHost, player: u128) {
    #[cfg(feature = "horizon_profile_api")]
    if let Some(p) = host.player(player) {
        let _ = p.refresh(host);
    }
    #[cfg(not(feature = "horizon_profile_api"))]
    let _ = (host, player);
}

/// One skin change, as the core decided it (`server.*OnSkinUpdate`, `server.sound`). Same order as the
/// original: profile first, then dismount and passengers, then the refresh, then the sound.
fn apply(host: &mut dyn ServerHost, player: u128, property: Option<&Property>, refresh: &Refresh) {
    let Some(p) = host.player(player) else {
        return;
    };
    if !set_profile_textures(host, player, property) {
        return;
    }
    let entity = p.entity();
    // TODO(arismoko-api: dismount): entity_stop_riding covers the dismount without a remount; the
    // remount (keeping the vehicle through the refresh) has no API yet.
    if refresh.dismount && !refresh.remount {
        let _ = entity.stop_riding(host);
    }
    if refresh.dismount_passengers {
        let _ = entity.eject_passengers(host);
    }
    refresh_player(host, player);
    if let Some(sound) = &refresh.sound {
        play_sound(host, player, sound);
    }
}

/// `server.sound`, to the player alone.
fn play_sound(host: &mut dyn ServerHost, player: u128, sound: &Sound) {
    let Some(source) = sound_source(&sound.category) else {
        return;
    };
    let Some(p) = host.player(player) else {
        return;
    };
    let _ = p.play_sound(host, &sound.id, source, sound.volume, sound.pitch);
}

/// A JSON value as SNBT, the form an item's text components take in NBT: objects as compounds, strings quoted,
/// booleans as bytes, and the entries of an array all compounds (a bare string becomes `{text: ...}`).
fn json_to_snbt(value: &serde_json::Value) -> String {
    use serde_json::Value;
    fn quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
    match value {
        Value::Null => "\"\"".into(),
        Value::Bool(b) => if *b { "1b" } else { "0b" }.into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => format!("{}d", n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => quote(s),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    Value::Object(_) => json_to_snbt(item),
                    other => format!("{{\"text\":{}}}", json_to_snbt(&Value::String(match other {
                        Value::String(s) => s.clone(),
                        o => o.to_string(),
                    }))),
                })
                .collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let parts: Vec<String> = map.iter().map(|(k, v)| format!("{}:{}", quote(k), json_to_snbt(v))).collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

/// A text of the core (JSON text) as the SNBT of a component; a string that is no JSON is literal text.
fn text_snbt(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v @ serde_json::Value::Object(_)) => json_to_snbt(&v),
        Ok(serde_json::Value::Array(items)) => json_to_snbt(&serde_json::Value::Array(items)),
        _ => json_to_snbt(&serde_json::json!({ "text": json })),
    }
}

/// The SNBT of an item with a name, lore, the head's texture and the glint of a menu entry.
fn item_snbt(item: &MenuItem) -> String {
    let mut components: Vec<String> = Vec::new();
    if !item.name.is_empty() {
        components.push(format!("\"minecraft:custom_name\":{}", text_snbt(&item.name)));
    }
    if !item.lore.is_empty() {
        let lines: Vec<String> = item.lore.iter().map(|l| text_snbt(l)).collect();
        components.push(format!("\"minecraft:lore\":[{}]", lines.join(",")));
    }
    if let Some(texture) = &item.texture_value {
        components.push(format!(
            "\"minecraft:profile\":{{\"properties\":[{{\"name\":\"textures\",\"value\":{}}}]}}",
            json_to_snbt(&serde_json::Value::String(texture.clone()))
        ));
    }
    if item.glint {
        components.push("\"minecraft:enchantment_glint_override\":1b".into());
    }
    format!(
        "{{\"id\":\"{}\",\"count\":1,\"components\":{{{}}}}}",
        item.material,
        components.join(",")
    )
}

/// One menu entry: the engine item with its name, lore, head texture and glint (built from SNBT; the bare
/// item when the engine refuses it).
fn menu_item(host: &mut dyn ServerHost, item: &MenuItem) -> Option<ItemStack> {
    host.item_from_snbt(&item_snbt(item)).or_else(|| item_stack(host, item.material))
}

/// `/skins`: a chest menu of `rows` rows, its slots as the core gave them, and the window reported to
/// the core for its clicks.
fn open_menu(
    host: &mut dyn ServerHost,
    player: u128,
    rows: u8,
    title: &str,
    slots: &[(usize, MenuItem)],
) {
    let size = usize::from(rows) * 9;
    let mut items: Vec<Option<ItemStack>> = (0..size).map(|_| None).collect();
    for (slot, item) in slots {
        if let Some(cell) = items.get_mut(*slot) {
            *cell = menu_item(host, item);
        }
    }
    let title = json_text(host, title);
    // MenuReplace::CloseFirst is vanilla's default (the old call had no such argument; see HMOD_PORT.md).
    match host.open_menu(player, rows, title, items, MenuReplace::CloseFirst) {
        Ok(Some(window)) => core::menu_opened(player, window),
        Ok(None) => log("the skins menu did not open (no window)"),
        Err(_) => log("the skins menu did not open (stack write refused)"),
    }
}

/// What the core asks of the engine on the server tick.
fn apply_actions(host: &mut dyn ServerHost, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Apply {
                player,
                property,
                refresh,
            } => apply(host, player, property.as_ref(), &refresh),
            Action::OpenMenu {
                player,
                rows,
                title,
                slots,
            } => open_menu(host, player, rows, &title, &slots),
            Action::CloseMenu { player } => host.close_menu(player),
            // `/skull`: the head with its texture and name, into the inventory (what does not fit is dropped).
            Action::GiveSkull { player, display_name, texture_value } => {
                let head = MenuItem {
                    material: "minecraft:player_head",
                    name: display_name,
                    lore: Vec::new(),
                    texture_value: Some(texture_value),
                    glint: false,
                };
                if let (Some(p), Some(stack)) = (host.player(player), menu_item(host, &head)) {
                    let _ = p.give_stack(host, stack);
                }
            }
            // The proxy's ack, on the raw channel declared in `register`.
            Action::SendPayload {
                player,
                channel,
                data,
            } => {
                if let Some(p) = host.player(player) {
                    if let Err(e) = p.send_payload(host, &PayloadId(channel.clone()), PayloadBytes(data)) {
                        log(&format!("the ack for {channel} is not sent: {e:?}"));
                    }
                }
            }
            Action::Tell { player, line } => match player {
                Some(uuid) => {
                    if let Some(p) = host.player(uuid) {
                        for l in line.lines() {
                            let text = plain(host, l);
                            let _ = p.tell(host, text);
                        }
                    }
                }
                None => {
                    for l in line.lines() {
                        log(&core::strip_codes(l));
                    }
                }
            },
        }
    }
}

/// One line of a `/sr` answer, to the sender's feedback (the console's log for the console).
fn reply(host: &mut dyn ServerHost, line: &str) {
    let text = plain(host, line);
    let _ = host.commands().feedback(text);
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// `sr:messagechannel` as a raw play channel: the proxy's skin updates in, the ack out.
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let mut payloads = registrar.payload_handlers();
        payloads
            .registrar("1")
            .play_raw(PayloadId(PROXY_CHANNEL.into()), Some(std::sync::Arc::new(ProxyInbox)));
    }

    /// Declares the nodes of the original's permission registry, each with its Bukkit default (`DECLARED_NODES`).
    fn permission_gather_nodes(&self, event: &mut PermissionGatherNodesEvent<'_>) {
        for &(name, level) in DECLARED_NODES {
            let _ = event.nodes.add(
                admin_node(name),
                NodeDefault::Level(PermissionLevel::from_level(i32::from(level))),
            );
        }
    }

    fn server_about_to_start(&self, _: &mut dyn ServerHost, _: &mut ServerAboutToStartEvent) {
        // Builds the core now (config, recommended skins, storage), not on the first tick.
        let _ = core::admin_level();
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        // SRProxyCommand: `/sr` and `/skinsrestorer`, each with `help [query]`, `status`, `dump`. Level 0,
        // as the permission nodes differ per subcommand; run_mod_command checks them.
        for root in ["sr", "skinsrestorer"] {
            event.commands.push(Command::new(root));
            for tail in ["help [query:text]", "status", "dump"] {
                event.commands.push(Command::new(&format!("{root} {tail}")));
            }
        }
    }

    /// Tab: `/sr` and `/skinsrestorer` are listed only for a sender with one of the admin nodes of
    /// `core::Sub::nodes` (the same nodes `run_mod_command` checks; the console is not a player here).
    fn player_commands_sent(&self, host: &mut dyn ServerHost, event: &mut PlayerCommandsSentEvent<'_>) {
        let mut may = false;
        if let Some(p) = host.player(event.player) {
            for name in ADMIN_NODES {
                // Any error is a denial.
                if p.permission_bool(host, &admin_node(name)).unwrap_or(false) {
                    may = true;
                    break;
                }
            }
        }
        if !may {
            event.names.retain(|n| {
                let lower = n.to_ascii_lowercase();
                let bare = lower.rsplit(':').next().unwrap_or("");
                bare != "sr" && bare != "skinsrestorer"
            });
        }
    }

    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        let Some(root) = event.command.literals.first().map(String::as_str) else {
            return Flow::Continue;
        };
        if root != "sr" && root != "skinsrestorer" {
            return Flow::Continue;
        }
        let (sender, player) = match &event.source {
            CommandSource::Player { player, .. } => (Some(player.uuid()), Some(*player)),
            CommandSource::Console { .. } => (None, None),
            _ => {
                reply(host, "Only players and the console may run /sr.");
                return Flow::Cancel;
            }
        };
        let mut line = event.command.literals[1..].join(" ");
        if let Some(CommandArgumentValue::String(query)) = event.command.argument("query") {
            line.push(' ');
            line.push_str(query);
        }
        // Admin nodes (skinsrestorer.admin*) are the permission provider's answer, or the operator level 2
        // without one. Other nodes are open to everyone; the console has every node. The typed queries need
        // `&mut` host, so the admin nodes are answered once here, before the core's `Fn` check runs.
        let lines = {
            let mut granted: Vec<&str> = Vec::new();
            if let Some(p) = player {
                for name in ADMIN_NODES {
                    if p.permission_bool(host, &admin_node(name)).unwrap_or(false) {
                        granted.push(name);
                    }
                }
            }
            let allowed = move |node: &str| {
                !node.starts_with("skinsrestorer.admin")
                    || player.is_none()
                    || granted.iter().any(|g| *g == node)
            };
            core::command(sender, &line, &allowed)
        };
        for l in lines {
            reply(host, &l);
        }
        Flow::Cancel
    }

    /// `/sr help <query>`: the syntaxes of the commands the sender may run (`suggestHelpQueries`).
    fn tab_complete(&self, host: &mut dyn ServerHost, event: &mut TabCompleteEvent<'_>) {
        let line = event.command.trim_start_matches('/');
        let mut words = line.split(' ');
        let (Some(root), Some(sub)) = (words.next(), words.next()) else {
            return;
        };
        if (root != "sr" && root != "skinsrestorer") || !sub.eq_ignore_ascii_case("help") {
            return;
        }
        let mut granted: Vec<&str> = Vec::new();
        if let Some(p) = host.player(event.player) {
            for name in ADMIN_NODES {
                if p.permission_bool(host, &admin_node(name)).unwrap_or(false) {
                    granted.push(name);
                }
            }
        }
        let allowed = |node: &str| !node.starts_with("skinsrestorer.admin") || granted.iter().any(|g| *g == node);
        let start = event.command.rfind(' ').map_or(0, |i| i + 1);
        let end = event.command.len();
        for name in core::help_suggestions(&allowed) {
            event.suggestions.push(Suggestion::new((start, end), name));
        }
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(
            event.phase,
            ServerTickPhase::ServerTickStart {
                world_running: true,
                ..
            }
        ) {
            return;
        }
        apply_actions(host, core::tick());
    }

    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        // The core only answers messages (ack, menu, skull) of a player it knows. The forwarded textures
        // come from the profile; a replacement the join lookup might return cannot be written yet
        // (HMOD-GAP(profile-write)), so it is dropped.
        let forwarded = host
            .player(event.player)
            .and_then(|p| p.profile(&*host))
            .and_then(|profile| {
                profile
                    .properties
                    .into_iter()
                    .find(|prop| prop.name == "textures")
            })
            .map(|prop| Property::new(prop.value, prop.signature));
        let _ = core::joining(event.player, event.name, forwarded);
    }

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        core::left(event.player);
    }

    fn menu_clicked(&self, _: &mut dyn ServerHost, event: &mut MenuClickedEvent) {
        core::menu_clicked(
            event.uuid,
            event.window,
            event.slot,
            event.button,
            event.mode,
        );
    }

    fn menu_closed(&self, _: &mut dyn ServerHost, event: &mut MenuClosedEvent) {
        core::menu_closed(event.uuid, event.window);
    }

    fn resource_pack_status(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ResourcePackStatusEvent<'_>,
    ) {
        // server.resourcePackFix: a pack that finished loading shows the skin again (through the core).
        core::resource_pack_status(event.player, event.status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_become_snbt_compounds() {
        assert_eq!(text_snbt(r#"{"text":"A","color":"red","bold":true}"#), r#"{"bold":1b,"color":"red","text":"A"}"#);
        assert_eq!(text_snbt("hi"), r#"{"text":"hi"}"#);
        assert_eq!(text_snbt(r#"["a",{"text":"b"}]"#), r#"[{"text":"a"},{"text":"b"}]"#);
    }

    #[test]
    fn a_menu_head_carries_its_texture_name_and_glint() {
        let item = MenuItem {
            material: "minecraft:player_head",
            name: r#"{"text":"Skin"}"#.into(),
            lore: vec!["line".into()],
            texture_value: Some("abc".into()),
            glint: true,
        };
        assert_eq!(
            item_snbt(&item),
            r#"{"id":"minecraft:player_head","count":1,"components":{"minecraft:custom_name":{"text":"Skin"},"minecraft:lore":[{"text":"line"}],"minecraft:profile":{"properties":[{"name":"textures","value":"abc"}]},"minecraft:enchantment_glint_override":1b}}"#
        );
    }
}
