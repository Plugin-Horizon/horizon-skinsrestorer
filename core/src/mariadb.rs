//! [`Sql`] on MariaDB/MySQL (the `mysql` crate, blocking). The pool connects on first use, so a
//! database that is down at start costs nothing until a skin is needed.
use crate::config::Database;
use crate::storage::{Row, Sql};
use mysql::prelude::Queryable;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// MySQL/MariaDB error 1146, ER_NO_SUCH_TABLE.
const ER_NO_SUCH_TABLE: u16 = 1146;

pub struct MariaDb {
    opts: mysql::Opts,
    pool: OnceLock<mysql::Pool>,
    timeout: Duration,
    /// Messages already logged for missing tables.
    missing: Mutex<HashSet<String>>,
}

/// `sslMode` from the original's connectionOptions (`a=b&c=d`).
fn ssl_mode(options: &str) -> String {
    options
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case("sslMode"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_else(|| "disable".into())
}

impl MariaDb {
    pub fn new(db: &Database, timeout: Duration) -> Self {
        let max = db.max_pool_size.max(1);
        let constraints = mysql::PoolConstraints::new(0, max).unwrap_or_default();
        let ssl = match ssl_mode(&db.connection_options).as_str() {
            "disable" | "disabled" | "false" => None,
            "trust" => Some(
                mysql::SslOpts::default()
                    .with_danger_accept_invalid_certs(true)
                    .with_danger_skip_domain_validation(true),
            ),
            _ => Some(mysql::SslOpts::default()),
        };
        let opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some(db.host.clone()))
            .tcp_port(db.port)
            .user(Some(db.username.clone()))
            .pass(Some(db.password.clone()).filter(|p| !p.is_empty()))
            .db_name(Some(db.database.clone()))
            .tcp_connect_timeout(Some(timeout))
            .read_timeout(Some(timeout))
            .write_timeout(Some(timeout))
            .ssl_opts(ssl)
            .pool_opts(mysql::PoolOpts::default().with_constraints(constraints));
        Self {
            opts: opts.into(),
            pool: OnceLock::new(),
            timeout,
            missing: Mutex::new(HashSet::new()),
        }
    }

    fn note_missing(&self, message: &str) {
        let first = self
            .missing
            .lock()
            .map(|mut seen| seen.insert(message.to_owned()))
            .unwrap_or(true);
        if first {
            log::info!("[SkinsRestorer] {message}: not created yet, so no skin is read from it");
        }
    }

    fn pool(&self) -> Result<&mysql::Pool, String> {
        if let Some(p) = self.pool.get() {
            return Ok(p);
        }
        let p = mysql::Pool::new(self.opts.clone()).map_err(|e| e.to_string())?;
        Ok(self.pool.get_or_init(|| p))
    }
}

impl Sql for MariaDb {
    fn first_row(&self, sql: &str, binds: &[Option<String>]) -> Result<Option<Row>, String> {
        let mut conn = self
            .pool()?
            .try_get_conn(self.timeout)
            .map_err(|e| e.to_string())?;
        let params = if binds.is_empty() {
            mysql::Params::Empty
        } else {
            mysql::Params::Positional(binds.iter().cloned().map(mysql::Value::from).collect())
        };
        let row: Option<mysql::Row> = match conn.exec_first(sql, params) {
            Ok(row) => row,
            // A fresh database: the proxy has not created its tables yet. That is no selection
            // and no skin, not an error (logged once per table).
            Err(mysql::Error::MySqlError(e)) if e.code == ER_NO_SUCH_TABLE => {
                self.note_missing(&e.message);
                return Ok(None);
            }
            Err(e) => return Err(e.to_string()),
        };
        Ok(row.map(|r| {
            (0..r.len())
                .map(|i| r.get_opt::<Option<String>, usize>(i).and_then(Result::ok).flatten())
                .collect()
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ssl_mode_is_read() {
        assert_eq!(super::ssl_mode("sslMode=trust&serverTimezone=UTC"), "trust");
        assert_eq!(super::ssl_mode("serverTimezone=UTC"), "disable");
    }
}
