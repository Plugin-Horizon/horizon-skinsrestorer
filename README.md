# horizon-skinsrestorer

## Port status

Compared with SkinsRestorer 15.x (proxy-mode backend): item list `audit/rec100/skinsrestorer.md`, 19 of 28 items done (68 %); 6 need engine API that exists in engine main but not in the SDK pin (profile write, refresh, remount, client language, wildcard nodes), 1 exists nowhere. New: `%skinsrestorer_...%` placeholders (feature `placeholderapi`, off).

> **Unofficial Rust port of SkinsRestorer for the Horizon server (https://github.com/arismoko/horizon). Not affiliated with or endorsed by the SkinsRestorer authors.**

The backend side of [SkinsRestorer](https://skinsrestorer.net) for [Horizon](https://github.com/arismoko/horizon),
the Rust server/client for Minecraft 26.3, as a native mod. On a proxy network the proxy keeps
running SkinsRestorer (commands, selections, storage); a Horizon backend with this mod behaves as
a Paper backend with the plugin in **proxy mode** does.

Derived from SkinsRestorer (<https://github.com/SkinsRestorer/SkinsRestorer>, GPL-3.0).
Not affiliated with SkinsRestorer or its authors.

> **Built for and working only with [arismoko/horizon](https://github.com/arismoko/horizon)** (the Horizon fork that carries the mod API). [horizon-rust/horizon](https://github.com/horizon-rust/horizon) is experimental for mods and not supported: the earlier version for its native hooks / horizon-mixins is frozen on the `horizon-plain` branch (work stopped on 2026-11-12).

## Which Horizon

This is a native trait mod: `horizon-mod.toml`, a `Cargo.toml` that depends on `horizon-mod-api`, `src/lib.rs` with the hooks, and the plugin logic in the `core/` crate (plain Rust, no engine needed for its tests).

- Pin: `[native] horizon = "54090ee36c4e2c7b43b66db864220c577b94b7e0"` in `horizon-mod.toml` (game `26.3`). That is a development build of arismoko/horizon `main` plus mod API additions that are not all merged upstream yet, so a stock `main` may lack some host methods this mod calls until they land.
- It compiled and loaded on that build and was smoke-tested there; it has not seen a long run with real players. Treat it as experimental.
- Feature descriptions below were written for the earlier horizon-rust version; where they name engine calls (`native_*`) or old file paths, read them as descriptions only. The Known gaps list is current.

## What it does (the original's proxy-mode backend)

- **Skin updates.** The proxy sends `skinUpdateV3` / `SkinUpdateV2` on the plugin channel
  `sr:messagechannel` (value, signature, optional ack request), decoded exactly as the original's
  codec writes it ([core/src/payload.rs](core/src/payload.rs)). The skin goes into the player's
  profile and is shown again: the list entry and the entity to everyone, a same-level respawn for
  the player, `server.dismountPlayerOnSkinUpdate` / `remountPlayerOnSkinUpdate` /
  `dismountPassengersOnSkinUpdate`, and `server.sound` (parsed like the original's `SoundParser`:
  `[~][category@]name, volume, pitch, seed`). An empty property clears the textures. An ack
  request is answered, so the proxy does not warn about a backend without SkinsRestorer.
- **Join.** The proxy puts the skin into the login profile (Velocity modern forwarding); Horizon
  shows it as it is. *Port extra:* with `database.type: MYSQL`, a join whose forwarded profile has
  no skin gets the stored selection from the shared SQL storage (the original's backend never
  reads storage in proxy mode). `advanced.disableOnJoinSkins` turns it off.
- **Commands** (`/sr` and `/skinsrestorer`, as `SRProxyCommand`): `/sr` (help), `/sr help [query]`,
  `/sr status` (service checks of Mojang, Eclipse and, with `api.elyByEnabled`, Ely.by),
  `/sr dump` (uploads the support report to bytebin.lucko.me and prints the link). Permission nodes:
  `skinsrestorer.admincommand` (root, help), `skinsrestorer.admincommand.status`,
  `skinsrestorer.admincommand.dump`; `skinsrestorer.admin` and `skinsrestorer.admincommand.*` also allow
  everything but `dump`, as in the original. Without a permissions provider operator level 2
  stands in (Bukkit's default for these nodes is "op"); `horizon.adminLevel` is not read. The console may do all.
  The other nodes of the original's permission registry are declared with Bukkit's defaults too (`DECLARED_NODES`).
  The skin commands (`/skin`, `/skull`, `/skins`) stay on the proxy.

## Config

The original asks for the same `config.yml` on the proxy and on every backend. This mod reads
`plugins/SkinsRestorer/config.yml` in the server folder when it exists (the keys it knows have the
original's names and defaults; the rest is ignored), and `config/skinsrestorer.toml` laid over it
key by key. With neither file the TOML is written with the defaults
([core/default-config.toml](core/default-config.toml)). `[horizon]` holds what only this port has:
`joinLookup`, `joinTimeoutMs`, `pollSeconds` (off by default), `adminLevel` and `[horizon.schema]`.

## SQL schema (what it reads)

The original's tables, read only: `<prefix>players` (`uuid`, `skin_identifier`, `skin_variant`,
`skin_type`), `<prefix>player_skins` (`uuid`, `value`, `signature`), `<prefix>url_skins` (key `url` +
`skin_variant`), `<prefix>url_index`, `<prefix>custom_skins` (`name`, lower case), `<prefix>legacy_skins`.
UUIDs are dashed; `skin_type` is `PLAYER`, `URL`, `CUSTOM` or `LEGACY`. A database without the tables
yet reads as no skin. The mod never writes.

## Build

Build it as a native trait mod with arismoko's launcher (see the "native mods" how-to in the arismoko/horizon docs):

```sh
horizon-launcher native-build <path to this folder> --server-output <dir> --version <tag> --trust-native-code
```

`--trust-native-code` is your consent to compile and run this code inside the server. The logic crate tests run without the engine: `(cd core && cargo test)`.

## Known gaps

What does not work yet on arismoko/horizon (each is marked `TODO(arismoko-api: ...)` in `src/lib.rs`):

- No skin is applied unless the mod is built with the cargo feature `horizon_profile_api` (off by default): it needs
  `player_set_profile_property` and `player_refresh` from arismoko/horizon #17923. That PR is merged upstream but the
  pinned SDK predates it, so bump the pin before enabling the feature. With it, a reset to the default skin writes an empty `textures` value.
- A joining player is registered with the core on login (so acks, the menu and skulls work). A stored skin the join lookup finds is written into the profile only with that feature.
- Mount keeping across a skin refresh is missing.
- Messages are fixed English text (no `locales/*.json`), `/sr help` is a plain list (cloud's paginated help is not reproduced). The `messages.*`, `login.*` and `commands.*` keys are read with the original's defaults but not used on the backend (the proxy does the logins and the commands).
- The apply path needs a SQL back end or proxy payloads; it was not run against either.

## Licence

Derived from SkinsRestorer (<https://github.com/SkinsRestorer/SkinsRestorer>, GPL-3.0). This port is
GPL-3.0-only as well, see [LICENSE](LICENSE) and [NOTICE](NOTICE).
