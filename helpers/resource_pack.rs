fn skinsrestorer_resource_pack(player: u128, status: i32) {
    // server.resourcePackFix: a pack that finished loading hides the skin on the client; the core
    // shows the profile's textures again with the next tick's actions. Heard for the server's own
    // pack too (given in the configuration), once the player has logged in.
    crate::skinsrestorer::resource_pack_status(player, status);
}
