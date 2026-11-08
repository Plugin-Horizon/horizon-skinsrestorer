fn skinsrestorer_perform(
    game: &mut crate::game::Game<'_>,
    player: &crate::game::native_server::NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    let mut words = command.splitn(2, char::is_whitespace);
    let root = words.next().unwrap_or("");
    if !(root.eq_ignore_ascii_case("sr") || root.eq_ignore_ascii_case("skinsrestorer")) {
        return;
    }
    let args = words.next().unwrap_or("");
    // Each subcommand names the permission nodes of the original; the permissions provider
    // (hooks-all native_has_permission) answers them, and without one the operator level
    // `horizon.adminLevel` stands in, as Bukkit's default for an unset node is "op".
    let admin = crate::skinsrestorer::admin_level();
    let uuid = player.uuid;
    let lines = crate::skinsrestorer::command(Some(uuid), args, &|node: &str| {
        game.native_has_permission(uuid, node, admin)
    });
    *ran = true;
    for l in lines {
        game.native_feedback(horizon_client::text::Text::legacy(&l));
    }
}
