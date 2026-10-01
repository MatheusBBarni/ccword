use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::ngram::{self, Observation};
use crate::paths::{self, Paths};
use crate::tokenize::{self, Token};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ngrams (
    order_n INTEGER NOT NULL,
    context TEXT NOT NULL,
    token TEXT NOT NULL,
    display TEXT NOT NULL,
    count INTEGER NOT NULL,
    mass REAL NOT NULL,
    last_seen INTEGER NOT NULL,
    PRIMARY KEY (order_n, context, token)
);
CREATE INDEX IF NOT EXISTS ngrams_context ON ngrams(order_n, context);
";

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    pub fn open(paths: &Paths) -> Result<Self> {
        paths.ensure_root()?;
        let path = paths.database_file();
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        restrict_db(&path)?;
        Ok(Self { conn, path })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn learn_prompt(&mut self, prompt: &str, now: i64, half_life_days: f64) -> Result<usize> {
        let tokens = tokenize::tokenize(prompt);
        self.learn_tokens(&tokens, now, half_life_days)
    }

    pub fn learn_tokens(
        &mut self,
        tokens: &[Token],
        now: i64,
        half_life_days: f64,
    ) -> Result<usize> {
        let observations = ngram::observations_for(tokens, now);
        let tx = self.conn.transaction()?;
        for obs in &observations {
            upsert(&tx, obs, now, half_life_days)?;
        }
        let count = observations.len();
        tx.commit()?;
        Ok(count)
    }

    pub fn candidates(&self, order_n: usize, context: &str) -> Result<Vec<Observation>> {
        let mut stmt = self.conn.prepare(
            "SELECT order_n, context, token, display, count, mass, last_seen
             FROM ngrams WHERE order_n = ?1 AND context = ?2",
        )?;
        let rows = stmt.query_map(params![order_n as i64, context], row_to_obs)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn count_rows(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM ngrams", [], |row| row.get(0))?;
        Ok(n as u64)
    }

    pub fn clear(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM ngrams", [])?;
        Ok(())
    }

    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if self.path.as_os_str() == ":memory:" {
            return Ok(());
        }
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&self.path, dest)?;
        paths::restrict_file(dest)?;
        Ok(())
    }
}

fn upsert(conn: &Connection, obs: &Observation, now: i64, half_life_days: f64) -> Result<()> {
    let existing = conn
        .query_row(
            "SELECT count, mass, last_seen, display FROM ngrams
             WHERE order_n = ?1 AND context = ?2 AND token = ?3",
            params![obs.order_n as i64, obs.context, obs.token],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    if let Some((count, mass, last_seen, _display)) = existing {
        let mass = ngram::touch(mass, last_seen, now, half_life_days);
        conn.execute(
            "UPDATE ngrams SET count = ?1, mass = ?2, last_seen = ?3, display = ?4
             WHERE order_n = ?5 AND context = ?6 AND token = ?7",
            params![
                count + 1,
                mass,
                now,
                obs.display,
                obs.order_n as i64,
                obs.context,
                obs.token
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO ngrams (order_n, context, token, display, count, mass, last_seen)
             VALUES (?1, ?2, ?3, ?4, 1, 1, ?5)",
            params![obs.order_n as i64, obs.context, obs.token, obs.display, now],
        )?;
    }
    Ok(())
}

fn row_to_obs(row: &rusqlite::Row<'_>) -> rusqlite::Result<Observation> {
    Ok(Observation {
        order_n: row.get::<_, i64>(0)? as usize,
        context: row.get(1)?,
        token: row.get(2)?,
        display: row.get(3)?,
        count: row.get::<_, i64>(4)? as u64,
        mass: row.get(5)?,
        last_seen_ms: row.get(6)?,
    })
}

fn restrict_db(path: &Path) -> Result<()> {
    paths::restrict_file(path)?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
        if sidecar.exists() {
            paths::restrict_file(&sidecar)?;
        }
    }
    Ok(())
}

pub fn predict(
    store: &Store,
    preceding: &[Token],
    prefix: &str,
    config: &crate::config::Config,
    now: i64,
) -> Result<Option<ngram::Scored>> {
    let mut rows = Vec::new();
    for order_n in 1..=ngram::MAX_ORDER {
        let Some(context) = ngram::context_key(preceding, order_n) else {
            continue;
        };
        rows.extend(store.candidates(order_n, &context)?);
    }
    Ok(ngram::rank(&rows, prefix, config, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    use crate::config::{now_ms, Config};
    use crate::ngram::suffix_for;

    #[test]
    fn learns_and_clears_without_storing_the_prompt() {
        let mut store = Store::open_in_memory().unwrap();
        let prompt = "please help the build";
        store.learn_prompt(prompt, now_ms(), 30.0).unwrap();
        assert!(store.count_rows().unwrap() > 0);
        let dump: String = store
            .conn
            .query_row(
                "SELECT group_concat(token || ' ' || context, '|') FROM ngrams",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!dump.contains(prompt));
        store.clear().unwrap();
        assert_eq!(store.count_rows().unwrap(), 0);
    }

    #[test]
    fn cold_start_predicts_nothing_until_support_exists() {
        let mut store = Store::open_in_memory().unwrap();
        let config = Config {
            mode: crate::config::Mode::Ngram,
            min_support: 2,
            min_confidence: 0.15,
            ..Config::default()
        };
        let tokens = tokenize::tokenize("please");
        assert!(predict(&store, &tokens, "he", &config, 0)
            .unwrap()
            .is_none());
        store.learn_prompt("please help", 0, 30.0).unwrap();
        store.learn_prompt("please help", 1, 30.0).unwrap();
        let scored = predict(&store, &tokens, "he", &config, 1).unwrap().unwrap();
        assert_eq!(suffix_for(&scored.display, "he").as_deref(), Some("lp"));
    }

    #[test]
    fn file_database_is_user_readable_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("state"));
        let mut store = Store::open(&paths).unwrap();
        store.learn_prompt("please help", 0, 30.0).unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
