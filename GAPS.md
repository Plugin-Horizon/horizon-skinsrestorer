# What Horizon lacks for this mod

Each line is a hook, helper or crate this mod needs, the feature it serves, and what the mod does
today. Signatures are proposals in the style of `native_hooks.rs` (revision `a589a3dd9`).

- [x] (hooks-all, used) `native_player_profile(player: u128, name: &str, properties: &mut Vec<login::Property>)` on the
  connection's thread, after authentication or the proxy's forwarding and before `LoginFinished` —
  needed for: replacing a join's textures with the stored selection before the client and anyone
  else sees the profile, and doing the blocking database lookup off the tick —
  today: two raw injections, HEAD of `lan::conn::login_and_configure` (lookup only, the
  properties are not `mut` there) and HEAD of `game::lan::Game::lan_guest_joined` (writes
  `Guest.properties`). The client's own `LoginFinished` profile keeps the forwarded textures.
- [x] (hooks-all, used) `native_set_profile_property(player: u128, name: &str, value: Option<(&str, Option<&str>)>) -> bool`
  (server helper) — needed for: a skin changed while the player is online — today: the raw
  `refresh` injection edits `host.hub.guests[i].properties` itself.
- [x] (hooks-all, used) `native_refresh_player(player: u128) -> bool` (server helper, Paper's `hidePlayer`/`showPlayer`
  plus a same-level respawn for the player) — needed for: showing a changed skin to the player
  and to everyone else — today: raw HEAD injection in `game::native_net::Game::flush_native_net`
  that sends `PlayerInfoRemove` + `PlayerInfoUpdate` (all actions) to every placed guest, calls
  `Hub::respawned(entity_id)` so trackers send the entity again, and pushes a same-dimension
  `LevelTeleport` into `Game.level_teleports` before calling `Game::run_level_teleports` (a
  `Respawn` keeping all data). Whether `run_level_teleports` is fine with source == target level
  is untested.
- [x] (hooks-all, used) `native_send_custom_payload_to_player(player, channel, data)` — needed for: the ack
  the proxy asks for on `sr:messagechannel` (otherwise the proxy logs that the backend has no
  SkinsRestorer) — today: used from `native_server_tick`.
- [x] (hooks-all, used) `native_custom_payload_from_client(player: u128, channel: &str, data: &[u8])` (server hook for
  every plugin channel, not only `horizon:mod_net`) — needed for: the proxy's change message —
  today: raw HEAD injection in `game::lan_actions::Game::lan_guest_packet` (same as horizon-voicechat).
- [x] (used: `native_server_tick`, the profile helpers work from any server hook) `native_server_tick_with_host` or a stable once-a-tick server hook that may call the hub
  helpers above — needed for: applying the core's actions — today: `flush_native_net` HEAD,
  filtered on `level_pass.last`.
- [x] (hooks-all, used for dismount and passengers; remount relies on `native_refresh_player` keeping the vehicle, untested) `native_dismount(player: u128)` / `native_mount(player: u128, vehicle: u32)` and
  `native_eject_passengers(entity: i32)` — needed for: `dismountPlayerOnSkinUpdate`,
  `remountPlayerOnSkinUpdate`, `dismountPassengersOnSkinUpdate` — today: `Guest::stop_riding()`
  for a dismount without remount; remount keeps `vehicle` through the respawn teleport (the
  engine's passenger teleport path, untested); passengers of the player are not dismounted.
- [x] (hooks-b-all, used) `native_play_sound_to(player: u128, id: &str, source: SoundSource, pos, volume, pitch)` —
  needed for: `server.sound` — today: a `Sound` packet with an inline event sent from the
  `refresh` injection.
- [x] (not needed: `detect_behind_proxy` checks for `forwarding.secret` and `online-mode` at start, as the engine does) `native_behind_proxy() -> bool` (whether `forwarding.secret` is set) — needed for:
  `proxyMode.detection: AUTO` — today: read from `Shared.config.forwarding_secret` in the
  `login-prefetch` injection, so it is unknown until the first login.
- [x] (hooks-all, used for status/dump/applyskin) `native_has_permission(player: u128, node: &str) -> Option<bool>` (a permissions provider such
  as a LuckPerms port answers) — needed for: `skinsrestorer.admincommand.status` / `.dump` —
  today: operator level (`adminLevel`, default 3).
- [x] (used, never run) `native_resource_pack_status_v2(player: u128, name: &str, pack: u128, status: i32)` — needed for:
  `server.resourcePackFix` (skin shown again after a pack loads). It also hears the answer to the server's own pack, given in the
  configuration and replayed once the player has logged in. `helpers/resource_pack.rs` -> `Core::resource_pack_status`: on status 0
  (successfully loaded) the profile's current textures are applied again, without a dismount or a sound. Heard for any pack, not only
  the server's: a reload by another mod's pack also shows the skin again, which is harmless.
- [ ] A menu hook shaped for the original's `openGUI` message (`SRInventory`: slots with items, names,
  lore, textures and click actions that go back to the proxy as `guiActionList`) — needed for: `/skins`
  — today: the message is logged and ignored. `giveSkull` (`/skull`) needs a native "give item with
  profile texture" helper; today: logged and ignored.
