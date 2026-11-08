# Status

Written without an engine build or a server: the core crate's logic has unit tests on synthetic
data; the engine helpers, the MySQL driver path and the HTTP calls have not been compiled or run.
Compared line by line against the original's source (SkinsRestorer 15.12.x: `SRServerPlugin`,
`SRServerMessageAdapter`, `SRServerPluginMessage`, `SRProxyCommand`, `SRCommandService`,
`ServiceCheckerService`, `DumpService`, `SoundParser`, `MySQLAdapter`, the `ServerConfig` /
`DatabaseConfig` / `AdvancedConfig` / `APIConfig` / `DevConfig` holders, `locale.json`).

## Inventory of the proxy-mode backend and how it matches

| Part | Original | Port |
|---|---|---|
| Commands | `/sr`, `/skinsrestorer`: root, `help [query]`, `status`, `dump` | same four, both names; tab completion for each |
| Permissions | `skinsrestorer.admincommand` (root, help), `.status`, `.dump`; groups `skinsrestorer.admin`, `skinsrestorer.admincommand.*` (not `dump`) | same nodes; group nodes asked as plain nodes; op level fallback |
| Messages | prefix, status (checking, UUID/profile counts, working/degraded/broken, firewall, version/server/proxy mode/commit), dump (uploading, success, error), divider | same texts and colours, English only |
| Plugin channel | `sr:messagechannel`: `skinUpdateV3`, `SkinUpdateV2`, `openGUI`, `giveSkull`, ack reply `ack` | skin updates and the ack exact; GUI and skull logged, not done |
| Skin apply | event, eject passengers, refresh others, refresh self, sound | dismount / eject passengers / profile / refresh / sound |
| Config keys | `database.*` (9), `server.*` (8), `advanced.*` (5), `api.elyByEnabled`, `dev.debug` | all read from `config.yml` or TOML, defaults identical; `proxyMode.api`, `enablePaperJoinListener`, `teleportRefresh`, `playerHeadChatObjects` accepted and unused |
| Storage (read) | MySQL tables above | same SQL; PostgreSQL and FILE not read |

Counts: 4 command forms, 3 permission nodes (+2 group nodes), about 19 message texts, 5 message types
(2 exact + ack, 2 not done), 24 config keys read (19 used, 5 accepted), 6 tables read.

## Differences, and why

- **GUI / skull messages** (`/skins`, `/skull` on the proxy): not done; no Horizon inventory-menu
  hook shaped like the original's `SRInventory` yet. Logged as a warning.
- **`resourcePackFix`**: done through `native_resource_pack_status_v2` (a raw target, `helpers/resource_pack.rs`); on a loaded pack the
  textures are applied again. Unit tested in the core; the injection is not built.
- **`teleportRefresh`, `enablePaperJoinListener`**: Paper-specific workarounds, no meaning here.
- **`/sr help`**: the original uses cloud's `MinecraftHelp` (paged, clickable); this prints a plain
  list with the original's descriptions. Cloud's own wording is not in the original's repository.
- **No-permission / invalid-syntax lines**: from cloud-minecraft-extras' defaults as remembered, not
  from the repository (guess).
- **`/sr dump`**: uploads like the original but leaves out the Java and OS-user sections (no JVM;
  user name and home folder are not sent). `plugins` is empty.
- **`/sr status`**: the Mojang UUID check is one bulk lookup without the original's batching
  window; the profile lines print the texture hash instead of the property's `toString()`.
- **Hardcoded skins** (`steve`, `alex`, ...: the fallback of a `CUSTOM` selection whose row is
  missing) and **recommendation skins** (`sr-recommendation-*`, fetched over HTTP): not done.
- **Join lookup and `pollSeconds`**: port extras (the original's backend does neither in proxy
  mode); the join lookup only fills a profile without textures, polling is off.
- **`/sr applyskin`, `reload`, `drop`, `info`, `createcustom`, `purgeolddata`, `docs permissions`**:
  not commands of a proxy-mode backend in the original either (`SRProxyCommand` has only the four above).
- Tab completion shows `/sr` to everyone (the tree has level 0 because the nodes differ per
  subcommand); a player without the node gets the no-permission line instead of a hidden command.

## Test first, once a build is possible

1. **A real proxy.** Velocity with SkinsRestorer, one Horizon backend with this mod: `/skin set`,
   `/skin clear`, `/skin url`, `/skin update` on the proxy; the backend should log nothing and the
   player's skin should change for them and for a second client. The proxy should not warn
   "does likely not have SkinsRestorer installed" (the ack).
2. **The refresh on Horizon:** position, rotation, inventory, effects, game mode and flight kept;
   `run_level_teleports` with source == target level; riding a boat/horse with remount on and off.
3. **MySQL.** With `database.type: MYSQL` against the proxy's database: a join with no forwarded
   skin shows the stored selection; PLAYER, URL (both variants), CUSTOM (mixed-case name) and
   LEGACY selections; a fresh database (no tables) must not log an error per join.
4. **`/sr status` and `/sr dump`** from a player and from the console; the late lines arrive on the
   tick; `advanced.noConnections: true` makes every check fail with "Connections are disabled.".
5. The sound by ear against a Paper backend (volume 0.7, pitch 1.0 for the default value).
