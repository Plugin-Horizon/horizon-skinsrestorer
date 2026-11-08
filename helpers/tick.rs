fn skinsrestorer_tick(game: &mut crate::game::Game<'_>) {
    // native_server_tick (start and end phase): apply what the core decided with the
    // profile helpers. The core's own clock paces the work, so two calls a tick are harmless.
    use crate::skinsrestorer::property::TEXTURES;
    use crate::skinsrestorer::Action;
    let actions = crate::skinsrestorer::tick();
    if actions.is_empty() {
        return;
    }
    let players = game.native_server_players();
    for action in actions {
        let (player, property, refresh) = match action {
            Action::Apply {
                player,
                property,
                refresh,
            } => (player, property, refresh),
            // The ack the proxy asked for (a plugin message on the player's own connection).
            Action::SendPayload {
                player,
                channel,
                data,
            } => {
                if let Err(e) = game.native_send_custom_payload_to_player(player, &channel, &data) {
                    log::warn!("[SkinsRestorer] could not answer the proxy: {e}");
                }
                continue;
            }
            // The late lines of /sr status and /sr dump.
            Action::Tell { player, line } => {
                match player {
                    Some(uuid) => {
                        game.native_tell(uuid, horizon_client::text::Text::legacy(&line));
                    }
                    None => {
                        for l in line.lines() {
                            log::info!("{}", crate::skinsrestorer::strip_codes(l));
                        }
                    }
                }
                continue;
            }
        };
        let Some(p) = players.iter().find(|p| p.uuid == player) else {
            continue;
        };
        // 1. The profile: what every later player list entry of this player carries.
        let value = property
            .as_ref()
            .map(|t| (t.value.as_str(), t.signature.as_deref()));
        if !game.native_set_profile_property(player, TEXTURES, value) {
            continue;
        }
        // 2. server.dismountPlayerOnSkinUpdate (unless remountPlayerOnSkinUpdate rides on
        //    through the refresh) and server.dismountPassengersOnSkinUpdate.
        let entity = p.entity_id as u32;
        if refresh.dismount && !refresh.remount {
            game.native_dismount(entity);
        }
        if refresh.dismount_passengers {
            game.native_eject_passengers(entity);
        }
        // 3. Everyone who sees the player gets the list entry and entity anew; the player is
        //    respawned in place so their own client draws the skin too.
        game.native_refresh_player(player);
        // 4. server.sound, to the player alone (hooks-b-all's per-player sound).
        if let Some(s) = &refresh.sound {
            game.native_play_sound_to(player, &s.id, &s.category, None, s.volume, s.pitch);
        }
    }
}
