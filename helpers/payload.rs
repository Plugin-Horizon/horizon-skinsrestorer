fn skinsrestorer_payload(player: u128, channel: &str, data: &[u8]) {
    // A proxy's plugin message reaches the backend as the player's own custom payload
    // (native_custom_payload_from_client: every channel outside horizon:).
    crate::skinsrestorer::message(player, channel, data);
}
