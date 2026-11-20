# hmod port: skinsrestorer
Parity: 51% now / 68% with our PRs (36/70 items; method: backend commands, nodes, config keys, messages and behaviours of the proxy-mode backend; skin application behind a missing API counts as not done; AdminInfoListener counted as closed because it has no reachable message on this backend). Round 100 changed no item count: profile write and refresh (#17923) are merged upstream but not in the pinned SDK, so they stay behind `horizon_profile_api`.

Round 100 (blind, not compiled)
- Checked #17923 on arismoko/horizon upstream/main: host methods `player_set_profile_property(player, name, value, signature) -> bool` and `player_refresh(player) -> bool` (the mod's `PlayerRef::set_profile_property` / `PlayerRef::refresh` match the generated wrapper naming). Not in the pinned SDK (`.horizon/sdk/api` = perm-sdk HEAD, and the pin in horizon-mod.toml) so the feature stays off. `profile_by_uuid` / `profile_by_name` came with it; no backend path here needs them (the join uses the login profile). #17923 has no `player_remove_profile_property` and no remount, so those gaps stay.
- Changed (behind `horizon_profile_api`): `player_logged_in` now writes the stored skin that `core::joining` returns into the profile, before the player is sent to others. Without the feature the replacement is still dropped, as before.
- Doc comments and README/Cargo.toml/GAPS_UPSTREAM notes: #17923 is merged upstream, not in the pin.
- Checklist (backend, proxy mode):
  - [x] `/sr`, `/sr help [query]`, `/sr status`, `/sr dump` registered (`SRPlugin.initProxyCommands` registers only `SRProxyCommand`; skin, skull and GUI commands are not on backends, matching the port)
  - [x] status texts and order vs `SRCommandService.executeStatus` and `locale.json` (`admincommand_status_*`, `dump_*`); the prefix is on the same messages as in `Message.java`, the divider is not
  - [x] verdict rule (`minOneServiceUnavailable` = a type with 0 successes) and the firewall line
  - [x] permission nodes: the 35 `PermissionRegistry` nodes and the two groups are all declared (wildcards cannot be node names)
  - [x] proxy message channel: skin update V2/V3, ack, GUI, give skull, unknown (`SRProxyMessageAdapter`, `SRServerMessageAdapter`)
  - [ ] commit short hash in `/sr status` (`BuildData.COMMIT_SHORT`): port prints the fixed text `horizon-port`
  - [ ] locales (`plugins/SkinsRestorer/locales/*.json`), `messages.disablePrefix`, `perIssuerLocale` (no locale API, `player_locale` not in #17923)
  - [ ] cloud's paginated `/sr help` layout
  - [ ] `messages.*`, `login.*`, `commands.*` keys used on a backend (the proxy does the logins and commands)
  - [ ] remount after a skin refresh (no entity remount API in #17923)
  - [ ] reset to the default skin: needs `player_remove_profile_property` (not in #17923); the feature writes an empty value
  - [ ] AdminInfoListener notices (update checker, Java 17 notice: not applicable on Horizon)
- Not run: no cargo, no tests. Feature build (`--features horizon_profile_api`) still needs a compile round against a pin that has #17923.
Round 1
- Fixed: src/lib.rs `ProxyInbox::handle`: returns `PayloadHandleError` (import added), context error wrapped in `PayloadHandleError::Failed`.
- Fixed: src/lib.rs `set_profile_textures`: no `set_profile_property` on main, call replaced by a refusal (gap below).
- Fixed: src/lib.rs `refresh_player`: no `PlayerRef::refresh` on main, call removed (gap below).
- Fixed: src/lib.rs `menu_item`: no `ItemStack::set_display` on main, call removed, name and lore computed but not set (gap below).
- Fixed: src/lib.rs `open_menu`: `host.open_menu` takes a fifth argument `MenuReplace`; passed `MenuReplace::CloseFirst`.
- Fixed: src/lib.rs `player_commands_sent` and `run_mod_command`: `PlayerRef::has_permission` replaced by typed `permission_bool` queries (see nodes). In `run_mod_command` the admin nodes are answered once before the core's `Fn` check, because typed queries need `&mut` host.
- Fixed: src/lib.rs new `permission_gather_nodes` hook declares the admin nodes.
- Permission nodes:
  - `skinsrestorer.admincommand` -> Level 2, declared in `permission_gather_nodes` (from `ADMIN_NODES`)
  - `skinsrestorer.admin` -> Level 2, same
  - `skinsrestorer.admincommand.status` -> Level 2, same
  - `skinsrestorer.admincommand.dump` -> Level 2, same
  - `skinsrestorer.admincommand.*` (wildcard) -> dropped: a node name cannot contain `*`, so it is not queried any more (LuckPerms wildcard grants are no longer seen by this mod).
- Gaps:
  - HMOD-GAP(profile-write): main has no setter for player profile properties (`textures`). No skin is applied at all: every skin change is refused. This guts the mod's main function (skin changes); the mod is effectively inert for skins. Needs a decision by the main session.
  - HMOD-GAP(player-refresh): no per-player tracker refresh; a changed skin is not re-shown to other players.
  - HMOD-GAP(item-display): no ItemStack setter for custom name or lore; skin menu entries show bare items without names or lore.
- Unsure:
  - `MenuReplace::CloseFirst` chosen for the new fifth argument of `open_menu` (vanilla default).
  - `PayloadHandleError`, `MenuReplace`, `PermissionLevel::from_level(2)` (assumed i32 argument), `NodeDefault` imported from the `mod_api` root.
  - The wildcard drop changes who can use `/sr` admin subcommands when a permission provider grants only the wildcard.
  - `run_mod_command` admin answers are precomputed for the four declared nodes; any other `skinsrestorer.admin*` name the core asks is denied (fail-closed).

- Audit round: `player_logged_in` now calls `core::joining`, so the core knows the player: the proxy ack, `openGUI` and `giveSkull` messages and resource-pack status are handled. A skin found by the join lookup is not applied (HMOD-GAP(profile-write)). README/config comments corrected (`adminLevel` unused, level 2 default).

Round 3 (100 % round)
- Menu entries now carry name, lore, head texture (`minecraft:profile`) and glint, built from SNBT with `item_from_snbt` (JSON text to compounds); `/skull` gives the head with `give_stack`.
- `/sr help` is filtered by the sender's permissions; `/sr help <TAB>` suggests the syntaxes of the commands the sender may run.
- Sound constants whose vanilla id has an underscore (`BLOCK_NOTE_BLOCK_PLING`, `ITEM_ARMOR_EQUIP_IRON`) resolve (table of compound words); an invalid `server.sound.value` logs `Invalid sound value in config: <value>`.
- Profile write and refresh are written behind the cargo feature `horizon_profile_api` (off): `PlayerRef::set_profile_property(host, name, value, signature) -> bool` and `PlayerRef::refresh(host) -> bool` of the `gap-profiles` branch (#17923). Not compiled by default.
- Gaps: skin application on Horizon main, locale files, cloud's help layout, `messages.*`/`login.*`/`commands.*` keys, AdminInfoListener, extra permission nodes, PlaceholderAPI expansions.
- Unsure (blind): the names `PlayerRef::give_stack`, `set_profile_property`, `refresh`; SNBT shape of `minecraft:profile`; `Suggestion::new((start, end), text)` as used by the invsee port.

Round 4 (blind, permission nodes, config keys, AdminInfoListener)
- Added: the nodes of the original's `PermissionRegistry` and its two groups are declared at gathering (`DECLARED_NODES` in src/lib.rs, 37 nodes) with Bukkit's defaults: the children of `skinsrestorer.player` true for everyone (level 0), all others operator (level 2). The wildcards (`skinsrestorer.command.*`, `skinsrestorer.admincommand.*`) stay out: a node name cannot contain `*` and Horizon has no node groups. The `/sr` checks still use `ADMIN_NODES`.
- Added: config keys the original has and the port did not read, with the original's defaults in data/skinsrestorer/default-config.toml and the core's `Config`: `login.*` (3), `messages.*` (3), `commands.forceDefaultPermissions`, `skinChangeCooldown`, `skinErrorCooldown`, `skullErrorCooldown`, `disableSkinCommand`, `disableSkullCommand`, `disableGUICommand`, `perSkinPermissions`, `perSkinPermissionsConsent`, `gui.notUnlockedSkin`, `gui.*.onlyShowList`, `storage.disallowAutoUpdateSkin`, `storage.skinExpiresAfter`, `storage.defaultSkins.applyForPremium`, `proxy.notAllowedCommandServers.allowList` and `ifNoServerBlockCommand`, `api.mineskinSecretSkins`, `api.mojangBatchWindowSeconds`. Read, not used on the backend: the proxy shows the menus, runs the commands and does the logins. `api.mineskinAPIKey` is still not kept (a secret; the Debug output must not show it).
- AdminInfoListener: not ported, counted as closed. In the original the adapter sends the outdated-version notice (needs the update checker, not ported), the Java-17 notice (no Java on Horizon) and the offline-mode launcher warning, which returns at once in proxy mode. On this backend nothing is sent.
- Not done: locale files (`plugins/SkinsRestorer/locales/*.json`), cloud's help layout, `messages.disablePrefix` and `perIssuerLocale` (no locale system yet).
- Tests (blind, not run): the new defaults and the camelCase keys (`disableGUICommand`, `ifNoServerBlockCommand`) in src/horizon_skinsrestorer_core/config.rs.
