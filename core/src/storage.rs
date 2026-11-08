//! The shared SQL storage the proxy writes: a player's selection (an identifier and its type)
//! and the skin records it names, in the original's tables (`MySQLAdapter`: `players`,
//! `player_skins`, `url_skins` keyed by url and variant, `url_index`, `custom_skins`,
//! `legacy_skins`). The queries come from the config ([`crate::config::Schema`]) so a changed
//! layout needs no rebuild; the defaults are the original's.
use crate::config::Schema;
use crate::property::{uuid_dashed, uuid_plain, Property};

/// What kind of record a selection names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinType {
    /// An account's skin; the identifier is that account's UUID.
    Player,
    /// A skin generated from an image URL.
    Url,
    /// A named skin an admin created.
    Custom,
    /// A record carried over from an older storage format.
    Legacy,
}

/// A player's saved selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub identifier: String,
    pub kind: SkinType,
    /// The model the selection asks for (CLASSIC/SLIM), when the table has it.
    pub variant: Option<String>,
}

/// One query's kind: which statement of the schema runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Selection,
    Skin(SkinType),
    /// The variant `url_index` holds for a URL (a selection without a variant).
    UrlIndex,
}

/// Values a statement may bind by name.
#[derive(Clone, Debug, Default)]
pub struct Binds {
    pub uuid: Option<String>,
    pub identifier: Option<String>,
    pub variant: Option<String>,
}

impl Binds {
    pub fn get(&self, name: &str) -> Option<&str> {
        match name {
            "uuid" => self.uuid.as_deref(),
            "identifier" => self.identifier.as_deref(),
            "variant" => self.variant.as_deref(),
            _ => None,
        }
    }
}

/// The schema with `{prefix}` filled in.
#[derive(Clone, Debug)]
pub struct Statements {
    pub schema: Schema,
    pub prefix: String,
}

impl Statements {
    pub fn new(schema: Schema, prefix: &str) -> Result<Self, String> {
        // A prefix goes into SQL text: letters, digits and underscores only.
        if !prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("tablePrefix {prefix:?} may only hold letters, digits and _"));
        }
        Ok(Self {
            schema,
            prefix: prefix.to_owned(),
        })
    }

    pub fn sql(&self, q: Query) -> String {
        let s = &self.schema;
        let raw = match q {
            Query::Selection => &s.selection,
            Query::Skin(SkinType::Player) => &s.player,
            Query::Skin(SkinType::Url) => &s.url,
            Query::Skin(SkinType::Custom) => &s.custom,
            Query::Skin(SkinType::Legacy) => &s.legacy,
            Query::UrlIndex => &s.url_index,
        };
        raw.replace("{prefix}", &self.prefix)
    }

    pub fn uuid(&self, u: u128) -> String {
        if self.schema.uuid_format.eq_ignore_ascii_case("plain") {
            uuid_plain(u)
        } else {
            uuid_dashed(u)
        }
    }

    pub fn kind_of(&self, raw: &str) -> Option<SkinType> {
        let s = &self.schema;
        [
            (&s.type_player, SkinType::Player),
            (&s.type_url, SkinType::Url),
            (&s.type_custom, SkinType::Custom),
            (&s.type_legacy, SkinType::Legacy),
        ]
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(raw.trim()))
        .map(|(_, k)| k)
    }
}

/// The `:name` placeholders of a statement, in order, and the statement with each replaced by
/// `?` (positional binding works on every driver version; quoted text is left alone).
pub fn placeholders(sql: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(sql.len());
    let mut names = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '\'' || c == '"' || c == '`' => {
                quote = Some(c);
                out.push(c);
            }
            None if c == ':' && chars.peek().is_some_and(|n| n.is_ascii_alphabetic()) => {
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                names.push(name);
                out.push('?');
            }
            None => out.push(c),
        }
    }
    (out, names)
}

/// A row: up to three text columns (NULL as None).
pub type Row = Vec<Option<String>>;

/// Something that runs the schema's statements: MariaDB in the server, a map in the tests.
pub trait Sql: Send + Sync {
    /// The first row the statement returns.
    fn first_row(&self, sql: &str, binds: &[Option<String>]) -> Result<Option<Row>, String>;
}

/// Reads selections and skins through [`Sql`].
pub struct Storage {
    pub statements: Statements,
    pub sql: Box<dyn Sql>,
}

impl Storage {
    fn run(&self, q: Query, binds: &Binds) -> Result<Option<Row>, String> {
        let (sql, names) = placeholders(&self.statements.sql(q));
        let mut values = Vec::with_capacity(names.len());
        for n in &names {
            match binds.get(n) {
                Some(v) => values.push(Some(v.to_owned())),
                None if n == "variant" => values.push(None),
                None => return Err(format!("the statement binds :{n}, which is not known here")),
            }
        }
        self.sql.first_row(&sql, &values)
    }

    pub fn selection(&self, player: u128) -> Result<Option<Selection>, String> {
        let binds = Binds {
            uuid: Some(self.statements.uuid(player)),
            ..Binds::default()
        };
        let Some(row) = self.run(Query::Selection, &binds)? else {
            return Ok(None);
        };
        let col = |i: usize| row.get(i).cloned().flatten();
        let (Some(identifier), Some(raw_kind)) = (col(0), col(1)) else {
            return Ok(None);
        };
        let kind = self
            .statements
            .kind_of(&raw_kind)
            .ok_or_else(|| format!("unknown skin type {raw_kind:?} in the selection"))?;
        Ok(Some(Selection {
            identifier,
            kind,
            variant: col(2).filter(|v| !v.is_empty()),
        }))
    }

    pub fn skin(&self, sel: &Selection) -> Result<Option<Property>, String> {
        // An account skin's identifier is a UUID; it is written the table's way. A custom skin's
        // name is lower case (`CustomSkinData.sanitizeCustomSkinName`).
        let identifier = match sel.kind {
            SkinType::Player => crate::property::parse_uuid(&sel.identifier)
                .map(|u| self.statements.uuid(u))
                .unwrap_or_else(|| sel.identifier.clone()),
            SkinType::Custom => sel.identifier.to_lowercase(),
            _ => sel.identifier.clone(),
        };
        // A URL skin is keyed by its variant; a selection without one asks `url_index`.
        let variant = match (&sel.variant, sel.kind) {
            (None, SkinType::Url) => {
                let binds = Binds {
                    identifier: Some(identifier.clone()),
                    ..Binds::default()
                };
                self.run(Query::UrlIndex, &binds)?
                    .and_then(|r| r.into_iter().next().flatten())
                    .filter(|v| !v.is_empty())
            }
            (v, _) => v.clone(),
        };
        let binds = Binds {
            identifier: Some(identifier),
            variant,
            ..Binds::default()
        };
        let Some(row) = self.run(Query::Skin(sel.kind), &binds)? else {
            return Ok(None);
        };
        Ok(row
            .first()
            .cloned()
            .flatten()
            .filter(|v| !v.is_empty())
            .map(|value| Property::new(value, row.get(1).cloned().flatten())))
    }

    /// The skin a player's selection names: None when there is no selection or its record is
    /// gone.
    pub fn skin_of(&self, player: u128) -> Result<Lookup, String> {
        match self.selection(player)? {
            None => Ok(Lookup::NoSelection),
            Some(sel) => Ok(match self.skin(&sel)? {
                Some(p) => Lookup::Skin(sel, p),
                None => Lookup::Dangling(sel),
            }),
        }
    }

    /// The account skin the proxy cached for `player` (what /skin clear goes back to).
    pub fn own_skin(&self, player: u128) -> Result<Option<Property>, String> {
        self.skin(&Selection {
            identifier: crate::property::uuid_dashed(player),
            kind: SkinType::Player,
            variant: None,
        })
    }
}

/// A player's lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    NoSelection,
    /// A selection whose record is missing.
    Dangling(Selection),
    Skin(Selection, Property),
}

/// A map from (statement, binds) to a row: the tests' database.
#[derive(Default)]
pub struct MemorySql {
    pub rows: std::sync::Mutex<std::collections::HashMap<(String, Vec<Option<String>>), Row>>,
    pub fail: std::sync::atomic::AtomicBool,
    pub queries: std::sync::atomic::AtomicUsize,
}

impl MemorySql {
    pub fn put(&self, sql: &str, binds: &[&str], row: Row) {
        self.rows.lock().unwrap().insert(
            (sql.to_owned(), binds.iter().map(|b| Some(b.to_string())).collect()),
            row,
        );
    }
    pub fn remove(&self, sql: &str, binds: &[&str]) {
        self.rows
            .lock()
            .unwrap()
            .remove(&(sql.to_owned(), binds.iter().map(|b| Some(b.to_string())).collect()));
    }
}

impl Sql for MemorySql {
    fn first_row(&self, sql: &str, binds: &[Option<String>]) -> Result<Option<Row>, String> {
        self.queries.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("database down".into());
        }
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&(sql.to_owned(), binds.to_vec()))
            .cloned())
    }
}

impl<T: Sql + ?Sized> Sql for std::sync::Arc<T> {
    fn first_row(&self, sql: &str, binds: &[Option<String>]) -> Result<Option<Row>, String> {
        (**self).first_row(sql, binds)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::Config;
    use crate::property::tests::sample;
    use std::sync::Arc;

    pub const SEL: &str = "SELECT skin_identifier, skin_type, skin_variant FROM sr_players WHERE uuid = ?";
    pub const PLAYER: &str = "SELECT value, signature FROM sr_player_skins WHERE uuid = ?";
    pub const CUSTOM: &str = "SELECT value, signature FROM sr_custom_skins WHERE name = ?";
    pub const URL: &str = "SELECT value, signature FROM sr_url_skins WHERE url = ? AND skin_variant = ?";
    pub const URL_INDEX: &str = "SELECT skin_variant FROM sr_url_index WHERE url = ?";

    pub fn storage() -> (Storage, Arc<MemorySql>) {
        let c = Config::default();
        let mem = Arc::new(MemorySql::default());
        let st = Storage {
            statements: Statements::new(c.horizon.schema, &c.database.table_prefix).unwrap(),
            sql: Box::new(mem.clone()),
        };
        (st, mem)
    }

    pub fn select_custom(mem: &MemorySql, player: u128, name: &str, hash: &str) -> Property {
        let p = sample(hash, false);
        mem.put(SEL, &[&uuid_dashed(player)], vec![Some(name.into()), Some("CUSTOM".into()), None]);
        mem.put(CUSTOM, &[name], vec![Some(p.value.clone()), p.signature.clone()]);
        p
    }

    #[test]
    fn placeholders_become_positional() {
        let (sql, names) = placeholders("SELECT a FROM t WHERE x = :uuid AND y = ':no' AND z = :variant");
        assert_eq!(sql, "SELECT a FROM t WHERE x = ? AND y = ':no' AND z = ?");
        assert_eq!(names, vec!["uuid", "variant"]);
    }

    #[test]
    fn default_schema_with_prefix() {
        let (st, _) = storage();
        assert_eq!(placeholders(&st.statements.sql(Query::Selection)).0, SEL);
        assert!(Statements::new(Config::default().horizon.schema, "sr_; DROP").is_err());
    }

    #[test]
    fn resolves_a_custom_selection() {
        let (st, mem) = storage();
        let p = select_custom(&mem, 5, "knight", "aa11");
        match st.skin_of(5).unwrap() {
            Lookup::Skin(sel, got) => {
                assert_eq!(sel.kind, SkinType::Custom);
                assert_eq!(got, p);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(st.skin_of(6).unwrap(), Lookup::NoSelection);
    }

    #[test]
    fn resolves_a_player_selection_and_reports_dangling() {
        let (st, mem) = storage();
        let other = 0xabcdu128;
        mem.put(SEL, &[&uuid_dashed(1)], vec![Some(uuid_plain(other)), Some("player".into()), None]);
        assert!(matches!(st.skin_of(1).unwrap(), Lookup::Dangling(_)));
        let p = sample("bb22", true);
        mem.put(PLAYER, &[&uuid_dashed(other)], vec![Some(p.value.clone()), p.signature.clone()]);
        assert_eq!(st.skin_of(1).unwrap(), Lookup::Skin(st.selection(1).unwrap().unwrap(), p));
    }

    #[test]
    fn a_url_skin_is_keyed_by_its_variant_and_falls_back_to_the_index() {
        let (st, mem) = storage();
        let p = sample("dd44", false);
        let u = "https://example.com/skin.png";
        mem.put(SEL, &[&uuid_dashed(7)], vec![Some(u.into()), Some("URL".into()), Some("SLIM".into())]);
        mem.put(URL, &[u, "SLIM"], vec![Some(p.value.clone()), p.signature.clone()]);
        assert!(matches!(st.skin_of(7).unwrap(), Lookup::Skin(_, got) if got == p));
        // No variant on the selection: the index names it.
        mem.put(SEL, &[&uuid_dashed(8)], vec![Some(u.into()), Some("URL".into()), None]);
        assert!(matches!(st.skin_of(8).unwrap(), Lookup::Dangling(_)));
        mem.put(URL_INDEX, &[u], vec![Some("SLIM".into())]);
        assert!(matches!(st.skin_of(8).unwrap(), Lookup::Skin(_, got) if got == p));
    }

    #[test]
    fn a_custom_name_is_looked_up_in_lower_case() {
        let (st, mem) = storage();
        let p = sample("ee55", false);
        mem.put(SEL, &[&uuid_dashed(9)], vec![Some("Knight".into()), Some("CUSTOM".into()), None]);
        mem.put(CUSTOM, &["knight"], vec![Some(p.value.clone()), p.signature.clone()]);
        assert!(matches!(st.skin_of(9).unwrap(), Lookup::Skin(_, got) if got == p));
    }

    #[test]
    fn unknown_type_and_errors_surface() {
        let (st, mem) = storage();
        mem.put(SEL, &[&uuid_dashed(2)], vec![Some("x".into()), Some("WEIRD".into()), None]);
        assert!(st.skin_of(2).is_err());
        mem.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(st.skin_of(3).is_err());
    }
}
