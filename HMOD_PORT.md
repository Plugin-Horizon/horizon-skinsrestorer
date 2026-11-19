# hmod port: skinsrestorer
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
