//! The core's behaviour on synthetic data (no real database, no real player).
use super::*;
use crate::jobs::InlineJobs;
use crate::payload::{tests::v3, Ack, CHANNEL};
use crate::property::tests::sample;
use crate::property::uuid_dashed;
use crate::storage::tests::{select_custom, storage, CUSTOM, PLAYER, SEL};
use crate::storage::MemorySql;

const P: u128 = 0x1111;

fn core_with(cfg: Config) -> (Core, Arc<MemorySql>) {
    let (st, mem) = storage();
    let st = Arc::new(st);
    let jobs = InlineJobs {
        storage: st.clone(),
        out: Vec::new(),
        submitted: Vec::new(),
    };
    let mut c = Core::new(cfg, Some(st), Some(Box::new(jobs)));
    c.behind_proxy = Some(true);
    (c, mem)
}

fn core() -> (Core, Arc<MemorySql>) {
    core_with(Config::default())
}

fn prefetch(c: &mut Core, player: u128) {
    let st = c.storage.clone().unwrap();
    let result = Core::prefetch_with(&st, player);
    c.prefetched.insert(player, Prefetch { result });
}

fn applied(actions: &[Action]) -> Vec<(u128, Option<String>)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Apply { player, property, .. } => {
                Some((*player, property.as_ref().map(|p| p.short_hash())))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_forwarded_skin_needs_no_lookup_in_proxy_mode() {
    let (mut c, mem) = core();
    let fwd = sample("f0f0", false);
    assert!(!c.wants_join_lookup(Some(&fwd)));
    assert_eq!(c.joining(P, "A", Some(fwd), Instant::now()), None);
    assert_eq!(mem.queries.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn a_join_without_a_skin_gets_the_stored_one_before_placement() {
    let (mut c, mem) = core();
    let p = select_custom(&mem, P, "knight", "abc123");
    assert!(c.wants_join_lookup(None));
    prefetch(&mut c, P);
    assert_eq!(c.joining(P, "A", None, Instant::now()), Some(Some(p)));
    // Nothing more to show: the profile had it from the start.
    assert!(c.tick(Instant::now()).is_empty());
}

#[test]
fn a_join_whose_prefetch_missed_is_applied_with_a_refresh() {
    let (mut c, mem) = core();
    select_custom(&mem, P, "knight", "abc123");
    let now = Instant::now();
    assert_eq!(c.joining(P, "A", None, now), None);
    let a = c.tick(now);
    // The inline job answers on the next tick.
    let a = if a.is_empty() { c.tick(now) } else { a };
    assert_eq!(applied(&a), vec![(P, Some("abc123".into()))]);
}

#[test]
fn standalone_lookup_replaces_a_forwarded_skin() {
    let mut cfg = Config::default();
    cfg.server.proxy_mode.detection = "DISABLED".into();
    let (mut c, mem) = core_with(cfg);
    let p = select_custom(&mem, P, "knight", "abc123");
    prefetch(&mut c, P);
    assert_eq!(c.joining(P, "A", Some(sample("f0f0", false)), Instant::now()), Some(Some(p)));
}

#[test]
fn a_message_with_a_skin_is_applied_every_time() {
    let (mut c, _) = core();
    c.joining(P, "A", Some(sample("f0f0", false)), Instant::now());
    let p = sample("beef01", true);
    assert!(c.message(P, CHANNEL, &v3(&p, None)));
    let a = c.tick(Instant::now());
    assert_eq!(applied(&a), vec![(P, Some("beef01".into()))]);
    let Action::Apply { property, refresh, .. } = &a[0] else {
        panic!("{a:?}")
    };
    assert_eq!(property.as_ref().unwrap().signature, p.signature);
    assert!(refresh.dismount && refresh.remount && !refresh.dismount_passengers);
    let sound = refresh.sound.as_ref().unwrap();
    assert_eq!(sound.id, "minecraft:entity.player.teleport");
    assert_eq!((sound.volume, sound.pitch), (0.7, 1.0));
    // SharedSkinApplier applies again even when the player shows it already.
    c.message(P, CHANNEL, &v3(&p, None));
    assert_eq!(applied(&c.tick(Instant::now())), vec![(P, Some("beef01".into()))]);
}

#[test]
fn an_ack_request_is_answered_and_the_empty_skin_clears() {
    let (mut c, _) = core();
    c.joining(P, "A", Some(sample("f0f0", false)), Instant::now());
    let ack = Ack {
        id: 77,
        proxy_version: "15.12.7".into(),
    };
    c.message(P, CHANNEL, &v3(&sample("beef01", false), Some(&ack)));
    let a = c.tick(Instant::now());
    assert_eq!(a.len(), 2);
    let Action::SendPayload { player, channel, data } = &a[1] else {
        panic!("{a:?}")
    };
    assert_eq!((*player, channel.as_str()), (P, CHANNEL));
    assert_eq!(data, &crate::payload::encode_ack(&ack).unwrap());
    // The empty property (value and signature empty) takes the textures away.
    let empty = Property::new("", Some(String::new()));
    c.message(P, CHANNEL, &v3(&empty, None));
    let a = c.tick(Instant::now());
    assert_eq!(applied(&a), vec![(P, None)]);
}

#[test]
fn other_channels_and_garbage_change_nothing() {
    let (mut c, _) = core();
    c.joining(P, "A", None, Instant::now());
    assert!(!c.message(P, "voicechat:x", b""));
    assert!(!c.message(P, "sr:skinchange", &v3(&sample("beef01", false), None)));
    assert!(c.message(P, CHANNEL, b"\x00\x04opaque"));
    assert!(c.tick(Instant::now()).is_empty());
    // A message for a player the mod does not know is ignored.
    assert!(c.message(0x99, CHANNEL, &v3(&sample("beef01", false), None)));
    assert!(c.tick(Instant::now()).is_empty());
}

#[test]
fn the_periodic_check_finds_a_change_and_a_clear() {
    let mut cfg = Config::default();
    cfg.horizon.poll_seconds = 10;
    let (mut c, mem) = core_with(cfg);
    let t0 = Instant::now();
    let own = sample("0w0", false);
    mem.put(PLAYER, &[&uuid_dashed(P)], vec![Some(own.value.clone()), own.signature.clone()]);
    c.joining(P, "A", Some(own.clone()), t0);
    assert!(c.tick(t0).is_empty());
    select_custom(&mem, P, "knight", "aaaa");
    let t1 = t0 + Duration::from_secs(10);
    c.tick(t1);
    assert_eq!(applied(&c.tick(t1)), vec![(P, Some("aaaa".into()))]);
    // /skin clear on the proxy: the selection goes, the account skin comes back.
    mem.remove(SEL, &[&uuid_dashed(P)]);
    mem.remove(CUSTOM, &["knight"]);
    let t2 = t1 + Duration::from_secs(10);
    c.tick(t2);
    assert_eq!(applied(&c.tick(t2)), vec![(P, Some("0w0".into()))]);
    // A player who never had a selection is left alone.
    let t3 = t2 + Duration::from_secs(10);
    c.tick(t3);
    assert!(c.tick(t3).is_empty());
}

#[test]
fn polling_is_off_by_default_and_reads_nothing() {
    let (mut c, mem) = core();
    c.joining(P, "A", Some(sample("f0f0", false)), Instant::now());
    for s in 0..40 {
        c.tick(Instant::now() + Duration::from_secs(s));
    }
    assert_eq!(mem.queries.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn a_database_error_changes_nothing_and_is_kept() {
    let (mut c, mem) = core();
    mem.fail.store(true, std::sync::atomic::Ordering::Relaxed);
    let t0 = Instant::now();
    prefetch(&mut c, P);
    assert_eq!(c.joining(P, "A", None, t0), None);
    assert_eq!(c.stats.last_error.as_deref(), Some("database down"));
}

#[test]
fn sr_lines_parse_to_the_proxy_mode_commands() {
    assert_eq!(parse(""), Ok(Sub::Root));
    assert_eq!(parse("  "), Ok(Sub::Root));
    assert_eq!(parse("help"), Ok(Sub::Help(String::new())));
    assert_eq!(parse("Help status dump"), Ok(Sub::Help("status dump".into())));
    assert_eq!(parse("STATUS"), Ok(Sub::Status));
    assert_eq!(parse("dump"), Ok(Sub::Dump));
    assert_eq!(parse("reload"), Err("/sr".into()));
    assert_eq!(parse("applyskin Alex"), Err("/sr".into()));
    assert_eq!(parse("status now"), Err("/sr status".into()));
}

#[test]
fn permissions_follow_the_original() {
    // status is in the admin groups; dump needs its own node.
    assert!(Sub::Status.nodes().contains(&"skinsrestorer.admincommand.status"));
    assert!(Sub::Status.nodes().contains(&"skinsrestorer.admin"));
    assert_eq!(Sub::Dump.nodes(), &["skinsrestorer.admincommand.dump"]);
    assert!(Sub::Root.nodes().contains(&"skinsrestorer.admincommand"));
    assert!(nodes("bogus").is_empty());
    assert_eq!(command(Some(P), "dump", &|n| n == "skinsrestorer.admin"), vec![NO_PERMISSION.to_string()]);
    assert_eq!(command(Some(P), "status extra", &|_| true)[0], invalid_syntax("/sr status"));
}

#[test]
fn help_lists_the_commands_and_filters_by_query() {
    let all = help_lines("");
    assert_eq!(all.len(), 4);
    assert!(all.iter().any(|l| l.contains("/sr status") && l.contains("Checks the API services")));
    let some = help_lines("dump");
    assert_eq!(some.len(), 2);
    assert!(some[1].contains("/sr dump"));
}

#[test]
fn the_prefix_is_the_originals() {
    assert_eq!(prefixed("\u{a7}7x"), "\u{a7}e[\u{a7}2SkinsRestorer\u{a7}e] \u{a7}e\u{a7}7x");
}

#[test]
fn leaving_forgets_the_player() {
    let mut cfg = Config::default();
    cfg.horizon.poll_seconds = 10;
    let (mut c, mem) = core_with(cfg);
    let t0 = Instant::now();
    c.joining(P, "A", None, t0);
    c.left(P);
    select_custom(&mem, P, "knight", "aaaa");
    c.tick(t0);
    assert!(c.tick(t0 + Duration::from_secs(20)).is_empty());
}

#[test]
fn no_storage_means_forwarded_skins_only() {
    let mut c = Core::new(Config::default(), None, None);
    c.behind_proxy = Some(true);
    assert!(!c.wants_join_lookup(None));
    assert_eq!(c.joining(P, "A", None, Instant::now()), None);
    assert!(c.tick(Instant::now()).is_empty());
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sr-proxy-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn behind_proxy_needs_the_secret_file_and_offline_mode() {
    let d = scratch_dir("none");
    std::fs::write(d.join("server.properties"), "online-mode=false\n").unwrap();
    assert!(!detect_behind_proxy(&d), "no forwarding.secret: no proxy");

    let d = scratch_dir("offline");
    std::fs::write(d.join("forwarding.secret"), "x").unwrap();
    std::fs::write(d.join("server.properties"), "motd=a\nonline-mode = false\n").unwrap();
    assert!(detect_behind_proxy(&d));

    let d = scratch_dir("online");
    std::fs::write(d.join("forwarding.secret"), "x").unwrap();
    std::fs::write(d.join("server.properties"), "online-mode=true\n").unwrap();
    assert!(!detect_behind_proxy(&d), "the engine ignores the secret in online mode");

    let d = scratch_dir("missing");
    std::fs::write(d.join("forwarding.secret"), "x").unwrap();
    assert!(!detect_behind_proxy(&d), "a missing line means vanilla's default, online");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_loaded_resource_pack_shows_the_skin_again_without_a_dismount_or_sound() {
    let (mut c, _mem) = core();
    let fwd = sample("f0f0", false);
    c.joining(P, "A", Some(fwd.clone()), Instant::now());
    // Declined, accepted, failed: nothing to fix.
    for status in [1, 2, 3, 4, 5, 6, 7] {
        c.resource_pack_status(P, status);
    }
    assert!(c.tick(Instant::now()).is_empty());
    c.resource_pack_status(P, 0);
    let actions = c.tick(Instant::now());
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::Apply { player, property, refresh } => {
            assert_eq!((*player, property.as_ref().map(|p| p.value.clone())), (P, Some(fwd.value.clone())));
            assert!(!refresh.dismount && !refresh.remount && !refresh.dismount_passengers && refresh.sound.is_none());
        }
        other => panic!("{other:?}"),
    }
    // An unknown player, and the switch off.
    c.resource_pack_status(0x2222, 0);
    assert!(c.tick(Instant::now()).is_empty());
    let mut cfg = Config::default();
    cfg.server.resource_pack_fix = false;
    let (mut off, _mem) = core_with(cfg);
    off.joining(P, "A", Some(sample("f0f0", false)), Instant::now());
    off.resource_pack_status(P, 0);
    assert!(off.tick(Instant::now()).is_empty());
}
