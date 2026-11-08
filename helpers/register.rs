fn skinsrestorer_register(commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    use crate::game::native_server::NativeCommand as C;
    // SRProxyCommand: `/sr` (help), `/sr help [query]`, `/sr status`, `/sr dump`, under both of the
    // original's names. Level 0 and no node on the tree: the permission nodes of the original differ
    // per subcommand (and `dump` is outside the admin groups), which one node per command cannot say,
    // so every run is checked against the right nodes in perform.rs and the answer is the original's
    // "no permission" line.
    for root in ["sr", "skinsrestorer"] {
        commands.push(C::new(root));
        for tail in ["help [query:text]", "status", "dump"] {
            commands.push(C::new(&format!("{root} {tail}")));
        }
    }
}
