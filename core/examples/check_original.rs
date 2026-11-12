//! `cargo run --example check_original -- <path to plugins/SkinsRestorer/config.yml>...`
//!
//! Loads each original file with the port's own loader and prints only key paths: which keys of the
//! file the loader reads, which it does not, and whether it parses. Never a value: a serde message
//! can quote a value, so a load that fails prints the fact, not the message.
use horizon_skinsrestorer_core::config::{yaml_to_toml, Config};
use horizon_skinsrestorer_core::yaml::{self, Node};
use std::collections::BTreeSet;

/// The key paths of the file's maps, leaves (scalars, lists, empty maps) only.
fn file_leaves(node: &Node, prefix: &str, out: &mut BTreeSet<String>) {
    match node {
        Node::Map { entries, .. } if !entries.is_empty() => {
            for e in entries {
                let path = if prefix.is_empty() { e.key.clone() } else { format!("{prefix}.{}", e.key) };
                file_leaves(&e.value, &path, out);
            }
        }
        _ => {
            if !prefix.is_empty() {
                out.insert(prefix.to_owned());
            }
        }
    }
}

/// The key paths of the loader's config (every field it has, and the tables above them).
fn loader_keys(value: &toml::Value, prefix: &str, out: &mut BTreeSet<String>) {
    if let toml::Value::Table(t) = value {
        for (k, child) in t {
            let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            out.insert(path.clone());
            loader_keys(child, &path, out);
        }
    }
}

fn check(path: &str) {
    println!("== {path}");
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            println!("  cannot read the file");
            return;
        }
    };
    let doc = match yaml::parse_document(&text) {
        Ok(d) => d,
        Err(e) => {
            println!("  parse error at line {}: {}", e.line, e.message);
            return;
        }
    };
    let mut file = BTreeSet::new();
    file_leaves(&doc.root, "", &mut file);

    let mut loader = BTreeSet::new();
    let defaults = toml::Value::try_from(Config::default()).expect("the config serialises");
    loader_keys(&defaults, "", &mut loader);

    match yaml_to_toml(&text).and_then(|_| Config::parse_layers(Some(&text), "")) {
        Ok(_) => println!("  loads: yes"),
        Err(_) => println!("  loads: NO (the message is not printed: it can quote a value)"),
    }
    let (read, unread): (Vec<_>, Vec<_>) = file.iter().partition(|k| loader.contains(*k));
    println!("  keys of the file: {}, read: {}, not read: {}", file.len(), read.len(), unread.len());
    println!("  read:");
    for k in &read {
        println!("    {k}");
    }
    println!("  not read:");
    for k in &unread {
        println!("    {k}");
    }
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: check_original <config.yml>...");
        std::process::exit(2);
    }
    for p in &paths {
        check(p);
    }
}
