# horizon-skinsrestorer

> **Unofficial Rust port of SkinsRestorer for the Horizon server (https://github.com/horizon-rust/horizon). Not affiliated with or endorsed by the SkinsRestorer authors.**

The backend side of [SkinsRestorer](https://skinsrestorer.net) for [Horizon](https://github.com/horizon-rust/horizon),
the Rust server/client for Minecraft 26.3, as a native mod. On a proxy network the proxy keeps
running SkinsRestorer (commands, selections, storage); a Horizon backend with this mod behaves as
a Paper backend with the plugin in **proxy mode** does.

Derived from SkinsRestorer (<https://github.com/SkinsRestorer/SkinsRestorer>, GPL-3.0).
Not affiliated with SkinsRestorer or its authors.

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
  everything but `dump`, as in the original. Without a permissions provider the operator level
  `horizon.adminLevel` stands in (Bukkit's default for these nodes is "op"). The console may do all.
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

A horizon-mixins bundle (this folder): one crate (`core`, using only crates in Horizon's
`Cargo.lock`: `base64`, `serde`, `serde_json`, `toml`, `log`, `mysql`, `ureq`), one module and seven
injections.

```sh
horizon-mixins apply --workspace <horizon source> --mods <folder containing this bundle> --out <stage>
cd <stage> && cargo build --profile fast -p horizon-client --no-default-features --locked
```

`horizon_revision` in `horizon-mixin.toml` pins the Horizon commit the bundle was written against.
Unit tests: `cargo test` in `core/` with Horizon's `Cargo.lock` copied next to it.

The profile, payload and tick injections need Horizon's `hooks-all` branch (`native_player_profile`,
`native_custom_payload_from_client`, `native_send_custom_payload_to_player`,
`native_set_profile_property`, `native_refresh_player`, `native_dismount`, `native_eject_passengers`)
and `hooks-b-all` (`native_play_sound_to`). [GAPS.md](GAPS.md) lists what is still missing and
[STATUS.md](STATUS.md) the feature-by-feature state against the original's source.

## Untested

Everything that touches the engine: the bundle has not been built or run yet. The core's logic
(config, wire format, storage statements on a fake database, change handling, commands) has unit
tests on synthetic data.

## Licence

Derived from SkinsRestorer (<https://github.com/SkinsRestorer/SkinsRestorer>, GPL-3.0). This port is
GPL-3.0-only as well, see [LICENSE](LICENSE) and [NOTICE](NOTICE).

## Contributing

Pull requests and improvements are always welcome! Open an issue or a PR -- direct pushes to main are disabled, every change goes through review.
