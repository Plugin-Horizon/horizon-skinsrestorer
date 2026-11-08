fn skinsrestorer_console(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    let mut words = command.splitn(2, char::is_whitespace);
    let root = words.next().unwrap_or("");
    if !(root.eq_ignore_ascii_case("sr") || root.eq_ignore_ascii_case("skinsrestorer")) {
        return;
    }
    // The console may do everything.
    let lines = crate::skinsrestorer::command(None, words.next().unwrap_or(""), &|_: &str| true);
    *ran = true;
    for l in lines {
        game.native_feedback(horizon_client::text::Text::legacy(&l));
    }
}
