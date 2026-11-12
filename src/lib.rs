//! SkinsRestorer (the backend side in proxy mode) as an arismoko native mod (trait mod on `mod_api`).
//! The logic is the unchanged `horizon-skinsrestorer-core` crate; this file only wires it to hooks
//! and host methods. Every gap on arismoko is a `TODO(arismoko-api: <name>)` and PORT_NOTES.md lists them.
//!
//! - Config: the core reads `plugins/SkinsRestorer/config.yml` when it exists, else
//!   `config/skinsrestorer.toml` (the core writes the defaults there on first start). Its defaults use
//!   `database.type = FILE`, so no MariaDB is opened.
//! - `/sr` and `/skinsrestorer` (`help`, `status`, `dump`): `register_commands` + `run_mod_command`, for
//!   players and the console. Admin nodes are answered by `player.has_permission` (provider, else op level 2).
//! - The skins menu: `menu_clicked`, `menu_closed` and `host.open_menu` / `host.close_menu`. Items have
//!   names and lore; head textures and the glint are still missing.
//! - Skin changes: `server_tick` drains the core's actions and applies them with `player.set_profile_property`
//!   and `player.refresh`. A reset to the default skin is still missing.
//! - The proxy's plugin messages: `sr:messagechannel` is a raw play channel (`play_raw`, arismoko #3209);
//!   skin updates arrive through its handler and the ack goes out with `player.send_payload`.
use horizon_skinsrestorer_core::gui::MenuItem;
use horizon_skinsrestorer_core::property::Property;
use horizon_skinsrestorer_core::sound::Sound;
use horizon_skinsrestorer_core::{self as core, Action, Refresh};
use mod_api::{
    Command, CommandArgumentValue, CommandSource, Flow, Hooks, Identifier, ItemStack,
    IPayloadContext, IPayloadHandler, MenuClickedEvent, MenuClosedEvent, PayloadBytes, PayloadId,
    PlayerLoggedOutEvent, RegisterCommandsEvent, Registrar, RegistrationContext, ResourcePackStatusEvent, RunModCommandEvent, ServerAboutToStartEvent,
    ServerHost, ServerTickEvent, ServerTickPhase, SoundSource, Text,
};

/// The proxy's channel (`sr:messagechannel`): skin updates in, the ack out.
const PROXY_CHANNEL: &str = "sr:messagechannel";

#[derive(Default)]
struct Mod;

/// The proxy's messages on `sr:messagechannel` (skin updates), for the core.
struct ProxyInbox;

impl IPayloadHandler for ProxyInbox {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let player = context.player().map_err(|e| format!("{e:?}"))?;
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
fn set_profile_textures(host: &mut dyn ServerHost, player: u128, property: Option<&Property>) -> bool {
    let Some(p) = host.player(player) else {
        return false;
    };
    match property {
        Some(prop) => p.set_profile_property(
            host,
            "textures",
            &prop.value,
            prop.signature.as_deref(),
        ),
        None => false,
    }
}

/// Shows the player's changed skin to everyone who sees them (`player_refresh`: hidden and shown again
/// over two ticks). Their own view keeps the old skin until they change level or join again.
fn refresh_player(host: &mut dyn ServerHost, player: u128) {
    if let Some(p) = host.player(player) {
        let _ = p.refresh(host);
    }
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

/// One menu entry: the engine item with its name and lore (`ItemStack::set_display`).
// TODO(arismoko-api: stack_components): the head's texture (`minecraft:profile`) and the glint are not
// set, so every head is a plain head and the entry does not glint.
fn menu_item(host: &mut dyn ServerHost, item: &MenuItem) -> Option<ItemStack> {
    let mut stack = item_stack(host, item.material)?;
    let name = (!item.name.is_empty()).then(|| json_text(host, &item.name));
    let lore: Vec<Text> = item.lore.iter().map(|line| json_text(host, line)).collect();
    let _ = stack.set_display(host, name.as_ref(), &lore);
    Some(stack)
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
    match host.open_menu(player, rows, title, items) {
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
            // TODO(arismoko-api: give_stack): /skull cannot give the head yet: `host.give` takes a bare item
            // key, and the head needs its texture component. The player is told so.
            Action::GiveSkull { player, .. } => {
                if let Some(p) = host.player(player) {
                    let _ = p.tell(
                        host,
                        Text::literal("/skull is not available on this server yet."),
                    );
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
        // without one (`has_permission`). Other nodes are open to everyone; the console has every node.
        let lines = {
            let host: &dyn ServerHost = &*host;
            let allowed = move |node: &str| {
                !node.starts_with("skinsrestorer.admin")
                    || player.map_or(true, |p| p.has_permission(host, node))
            };
            core::command(sender, &line, &allowed)
        };
        for l in lines {
            reply(host, &l);
        }
        Flow::Cancel
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

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        // TODO(arismoko-api: player_profile): the join lookup (core::profile) needs the profile hook on
        // login, so a joining player keeps the skin the proxy forwarded.
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
