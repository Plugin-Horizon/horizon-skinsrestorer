fn skinsrestorer_profile(
    player: u128,
    name: &str,
    properties: &mut Vec<horizon_protocol::packets::login::Property>,
) {
    // The connection's own thread (native_player_profile): the player was let in, nobody (their
    // own client included) has the profile yet. A blocking lookup here holds up only this login.
    use horizon_skinsrestorer_core::property::{replace_textures, Property, TEXTURES};
    let forwarded = properties
        .iter()
        .find(|p| p.name == TEXTURES)
        .map(|p| Property::new(p.value.clone(), p.signature.clone()));
    if let Some(new) = horizon_skinsrestorer_core::profile(player, name, forwarded) {
        replace_textures(
            properties,
            new.as_ref(),
            |p| p.name == TEXTURES,
            |n| horizon_protocol::packets::login::Property {
                name: TEXTURES.into(),
                value: n.value.clone(),
                signature: n.signature.clone(),
            },
        );
    }
}
