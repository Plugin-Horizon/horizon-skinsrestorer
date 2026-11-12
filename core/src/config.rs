//! The mod's config: the original's `plugins/SkinsRestorer/config.yml` where it exists, else the
//! TOML file (`config/skinsrestorer.toml`), with the original's key names where a key means the same.
use crate::yaml::{self, Node};
use serde::{Deserialize, Serialize};

pub const DEFAULT: &str = include_str!("../default-config.toml");

/// A number written as a number or as text (the YAML reader keeps scalars as text).
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr + Deserialize<'de>,
    T::Err: std::fmt::Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either<T> {
        Number(T),
        Text(String),
    }
    match Either::<T>::deserialize(d)? {
        Either::Number(n) => Ok(n),
        Either::Text(s) => s.trim().parse().map_err(serde::de::Error::custom),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub database: Database,
    pub server: Server,
    pub advanced: Advanced,
    pub api: Api,
    pub dev: Dev,
    pub horizon: Horizon,
    pub gui: Gui,
    pub commands: Commands,
    pub storage: StorageConfig,
    pub proxy: Proxy,
}

/// A list the original reads from `config.yml`. The backend has no use for most of them (the
/// proxy shows the menus and runs the commands); they are read so that the whole file is read.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Listed {
    pub enabled: bool,
    pub list: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Gui {
    pub custom: Listed,
    pub recommendations: Listed,
    pub players: Listed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commands {
    pub disabled_skins: Listed,
    pub restrict_skin_urls: Listed,
    pub custom_help: CustomHelp,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CustomHelp {
    pub enabled: bool,
    pub message: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    pub default_skins: Listed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proxy {
    pub not_allowed_command_servers: Listed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Database {
    #[serde(rename = "type")]
    pub kind: String,
    pub host: String,
    #[serde(deserialize_with = "lenient")]
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(deserialize_with = "lenient")]
    pub max_pool_size: usize,
    pub table_prefix: String,
    pub connection_options: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub resource_pack_fix: bool,
    pub dismount_player_on_skin_update: bool,
    pub remount_player_on_skin_update: bool,
    pub dismount_passengers_on_skin_update: bool,
    pub sound: SoundConfig,
    pub proxy_mode: ProxyMode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SoundConfig {
    pub enabled: bool,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProxyMode {
    pub detection: String,
    pub api: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Advanced {
    pub disable_on_join_skins: bool,
    pub enable_paper_join_listener: bool,
    pub teleport_refresh: bool,
    pub no_connections: bool,
    pub player_head_chat_objects: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Api {
    pub ely_by_enabled: bool,
    /// The recommended skins are loaded from `plugins/SkinsRestorer/recommendations.json` only when this is on.
    pub fetch_recommended_skins: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dev {
    pub debug: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Horizon {
    pub join_lookup: String,
    pub join_timeout_ms: u64,
    pub poll_seconds: u64,
    pub admin_level: u8,
    pub schema: Schema,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    pub selection: String,
    pub player: String,
    pub url: String,
    pub url_index: String,
    pub custom: String,
    pub legacy: String,
    pub type_player: String,
    pub type_url: String,
    pub type_custom: String,
    pub type_legacy: String,
    pub uuid_format: String,
}

impl Default for Config {
    fn default() -> Self {
        Self::parse("").expect("the default config parses")
    }
}

/// `over` laid over `base`, table by table: a key the file leaves out keeps the default.
fn merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

/// When a join's skin is read from the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinLookup {
    Missing,
    Always,
    Never,
}

impl Config {
    /// The defaults, then `text` (the TOML file), then `yaml` (the original's config.yml, when there
    /// is one), each laid over the one before key by key. So the TOML file is only a fallback: it
    /// fills the keys the original file leaves out (the `[horizon]` section, and anything the file
    /// does not set), and a key the original file has is the original's.
    pub fn parse_layers(yaml: Option<&str>, text: &str) -> Result<Self, String> {
        let mut value: toml::Value = toml::from_str(DEFAULT).map_err(|e| e.to_string())?;
        let user: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
        merge(&mut value, user);
        if let Some(y) = yaml {
            merge(&mut value, yaml_to_toml(y).map_err(|e| format!("config.yml: {e}"))?);
        }
        value.try_into().map_err(|e: toml::de::Error| e.to_string())
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        Self::parse_layers(None, text)
    }

    /// Proxy mode: AUTO follows whether the server takes a proxy's forwarding.
    pub fn proxy_mode(&self, behind_proxy: bool) -> bool {
        match self.server.proxy_mode.detection.to_ascii_uppercase().as_str() {
            "ENABLED" => true,
            "DISABLED" => false,
            _ => behind_proxy,
        }
    }

    pub fn join_lookup(&self, proxy_mode: bool) -> JoinLookup {
        if self.advanced.disable_on_join_skins {
            return JoinLookup::Never;
        }
        match self.horizon.join_lookup.to_ascii_lowercase().as_str() {
            "missing" => JoinLookup::Missing,
            "always" => JoinLookup::Always,
            "never" => JoinLookup::Never,
            _ if proxy_mode => JoinLookup::Missing,
            _ => JoinLookup::Always,
        }
    }

    /// Whether the network may be used (`advanced.noConnections`).
    pub fn connections_allowed(&self) -> bool {
        !self.advanced.no_connections
    }

    /// The config as /sr dump sends it: the `database.*` keys are left out, as the original does.
    pub fn dump_json(&self) -> serde_json::Value {
        serde_json::json!({
            "server": {
                "resourcePackFix": self.server.resource_pack_fix,
                "dismountPlayerOnSkinUpdate": self.server.dismount_player_on_skin_update,
                "remountPlayerOnSkinUpdate": self.server.remount_player_on_skin_update,
                "dismountPassengersOnSkinUpdate": self.server.dismount_passengers_on_skin_update,
                "sound": { "enabled": self.server.sound.enabled, "value": self.server.sound.value },
                "proxyMode": { "detection": self.server.proxy_mode.detection, "api": self.server.proxy_mode.api },
            },
            "advanced": {
                "disableOnJoinSkins": self.advanced.disable_on_join_skins,
                "enablePaperJoinListener": self.advanced.enable_paper_join_listener,
                "teleportRefresh": self.advanced.teleport_refresh,
                "noConnections": self.advanced.no_connections,
                "playerHeadChatObjects": self.advanced.player_head_chat_objects,
            },
            "api": { "elyByEnabled": self.api.ely_by_enabled },
            "dev": { "debug": self.dev.debug },
        })
    }
}

/// The original's `config.yml` as a TOML tree, read by the vendored YAML parser ([`crate::yaml`]):
/// maps keep their keys, scalars are text except booleans (SnakeYAML's resolution), lists are arrays
/// of text, and a null (`key:` with nothing under it) is left out. Keys this mod has no field for
/// are carried along and dropped by [`Config`]'s deserialisation.
pub fn yaml_to_toml(text: &str) -> Result<toml::Value, String> {
    let node = yaml::parse(text).map_err(|e| e.to_string())?;
    Ok(node_to_toml(&node).unwrap_or_else(|| toml::Value::Table(toml::map::Map::new())))
}

fn node_to_toml(node: &Node) -> Option<toml::Value> {
    if node.is_null() {
        return None;
    }
    Some(match node {
        Node::Map { entries, .. } => {
            let mut table = toml::map::Map::new();
            for e in entries {
                if let Some(v) = node_to_toml(&e.value) {
                    table.insert(e.key.clone(), v);
                }
            }
            toml::Value::Table(table)
        }
        Node::Seq { items, .. } => toml::Value::Array(items.iter().filter_map(node_to_toml).collect()),
        Node::Scalar { .. } => match node.tag() {
            "bool" => toml::Value::Boolean(node.as_bool()?),
            _ => toml::Value::String(node.as_str()?.to_owned()),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses_with_the_originals_defaults() {
        let c = Config::default();
        assert_eq!(c.database.kind, "FILE");
        assert_eq!(c.database.host, "localhost");
        assert_eq!(c.database.database, "db");
        assert_eq!(c.database.username, "root");
        assert_eq!(c.database.password, "pass");
        assert_eq!(c.database.max_pool_size, 10);
        assert_eq!(c.database.table_prefix, "sr_");
        assert_eq!(c.database.connection_options, "sslMode=trust&serverTimezone=UTC");
        assert!(c.server.dismount_player_on_skin_update);
        assert!(c.server.sound.enabled);
        assert_eq!(c.server.sound.value, "ENTITY_PLAYER_TELEPORT, 0.7");
        assert_eq!(c.server.proxy_mode.detection, "AUTO");
        assert!(!c.advanced.no_connections);
        assert_eq!(c.horizon.schema.type_player, "PLAYER");
        assert_eq!(c.horizon.poll_seconds, 0);
    }

    #[test]
    fn missing_keys_take_the_default() {
        let c = Config::parse("[database]\nhost = \"db\"\n").unwrap();
        assert_eq!(c.database.host, "db");
        assert_eq!(c.database.port, 3306);
    }

    #[test]
    fn modes() {
        let mut c = Config::default();
        assert!(c.proxy_mode(true));
        assert!(!c.proxy_mode(false));
        assert_eq!(c.join_lookup(true), JoinLookup::Missing);
        assert_eq!(c.join_lookup(false), JoinLookup::Always);
        c.server.proxy_mode.detection = "ENABLED".into();
        assert!(c.proxy_mode(false));
        c.advanced.disable_on_join_skins = true;
        assert_eq!(c.join_lookup(true), JoinLookup::Never);
    }

    const YAML: &str = r#"# SkinsRestorer
database:
  # where
  type: MYSQL
  host: 'db.internal'
  port: 3307
  password: 12345
  maxPoolSize: 4 # small
storage:
  defaultSkins:
    enabled: false
    list:
      - steve
server:
  dismountPlayerOnSkinUpdate: false
  sound:
    value: "none"
  proxyMode:
    detection: ENABLED
"#;

    #[test]
    fn the_originals_config_yml_is_read() {
        let c = Config::parse_layers(Some(YAML), "").unwrap();
        assert_eq!(c.database.kind, "MYSQL");
        assert_eq!(c.database.host, "db.internal");
        assert_eq!(c.database.port, 3307);
        assert_eq!(c.database.password, "12345");
        assert_eq!(c.database.max_pool_size, 4);
        assert!(!c.server.dismount_player_on_skin_update);
        assert!(c.server.remount_player_on_skin_update, "other keys keep the default");
        assert_eq!(c.server.sound.value, "none");
        assert!(c.proxy_mode(false));
        // The TOML file is the fallback: the original's key wins, the keys it leaves out come from TOML.
        let c = Config::parse_layers(Some(YAML), "[database]\nhost = \"other\"\n[horizon]\njoinLookup = \"never\"\n").unwrap();
        assert_eq!(c.database.host, "db.internal");
        assert_eq!(c.horizon.join_lookup, "never");
        // Without the original's file the TOML file is all there is.
        let c = Config::parse_layers(None, "[database]\nhost = \"other\"\n").unwrap();
        assert_eq!(c.database.host, "other");
    }

    #[test]
    fn the_dump_leaves_the_database_out() {
        let mut c = Config::default();
        c.database.password = "hunter2".into();
        let text = c.dump_json().to_string();
        assert!(!text.contains("hunter2") && !text.contains("database"));
        assert!(text.contains("dismountPlayerOnSkinUpdate"));
    }

    const LISTS: &str = r#"# lists and a key the mod does not keep
gui:
  custom:
    enabled: true
    list:
      - "Alpha"
      - beta
  recommendations:
    list: []
commands:
  customHelp:
    enabled: false
    message:
      - "<gray>one"
api:
  mineskinAPIKey: sk-test-SECRET-VALUE-0042
  fetchRecommendedSkins: no
proxy:
  notAllowedCommandServers:
    enabled: yes
    list:
      - lobby
database:
  host:
  port: 3308
"#;

    #[test]
    fn the_lists_of_config_yml_are_read() {
        let c = Config::parse_layers(Some(LISTS), "").unwrap();
        assert!(c.gui.custom.enabled);
        assert_eq!(c.gui.custom.list, vec!["Alpha".to_owned(), "beta".to_owned()]);
        assert!(c.gui.recommendations.list.is_empty());
        assert_eq!(c.commands.custom_help.message, vec!["<gray>one".to_owned()]);
        assert!(c.proxy.not_allowed_command_servers.enabled, "yes is a boolean");
        assert_eq!(c.proxy.not_allowed_command_servers.list, vec!["lobby".to_owned()]);
        assert!(!c.api.fetch_recommended_skins);
        // A key with nothing under it is null: the default stays.
        assert_eq!(c.database.host, "localhost");
        assert_eq!(c.database.port, 3308);
    }

    #[test]
    fn the_api_key_is_never_kept_or_dumped() {
        let c = Config::parse_layers(Some(LISTS), "").unwrap();
        let secret = "sk-test-SECRET-VALUE-0042";
        assert!(!toml::to_string(&c).unwrap().contains(secret));
        assert!(!c.dump_json().to_string().contains(secret));
        assert!(!c.dump_json().to_string().to_ascii_lowercase().contains("mineskin"));
        let text = format!("{:?}", c);
        assert!(!text.contains(secret));
    }

    #[test]
    fn a_broken_config_yml_is_an_error_not_a_partial_read() {
        assert!(Config::parse_layers(Some("a:\n\tb: 1\n"), "").is_err());
    }
}
