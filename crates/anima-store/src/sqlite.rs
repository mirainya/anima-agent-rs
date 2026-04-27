use rusqlite::{params, Connection, Transaction};
use parking_lot::Mutex;
use std::path::Path;

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = path.as_ref().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn ensure_table(&self, table: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS [{table}] (id TEXT PRIMARY KEY, data TEXT NOT NULL)"
        ))?;
        Ok(())
    }

    pub fn upsert(&self, table: &str, id: &str, data: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute(
            &format!("INSERT OR REPLACE INTO [{table}] (id, data) VALUES (?1, ?2)"),
            params![id, data],
        )?;
        Ok(())
    }

    pub fn get(&self, table: &str, id: &str) -> Result<Option<String>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("SELECT data FROM [{table}] WHERE id = ?1"))?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn get_all(&self, table: &str) -> Result<Vec<(String, String)>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("SELECT id, data FROM [{table}]"))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    pub fn append(&self, table: &str, id: &str, data: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute(
            &format!("INSERT INTO [{table}] (id, data) VALUES (?1, ?2)"),
            params![id, data],
        )?;
        Ok(())
    }

    pub fn get_all_ordered(&self, table: &str) -> Result<Vec<(String, String)>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("SELECT id, data FROM [{table}] ORDER BY rowid"))?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    pub fn with_transaction<F, T>(&self, f: F) -> Result<T, rusqlite::Error>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, rusqlite::Error>,
    {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn execute_in_transaction(&self, table: &str, ops: &[(&str, &str)]) -> Result<(), rusqlite::Error> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for (id, data) in ops {
            tx.execute(
                &format!("INSERT OR REPLACE INTO [{table}] (id, data) VALUES (?1, ?2)"),
                params![id, data],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn count(&self, table: &str) -> Result<usize, rusqlite::Error> {
        let conn = self.conn.lock();
        conn.query_row(&format!("SELECT COUNT(*) FROM [{table}]"), [], |row| row.get(0))
    }

    pub fn delete(&self, table: &str, id: &str) -> Result<bool, rusqlite::Error> {
        let conn = self.conn.lock();
        let affected = conn.execute(
            &format!("DELETE FROM [{table}] WHERE id = ?1"),
            params![id],
        )?;
        Ok(affected > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_get() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("test").unwrap();
        store.upsert("test", "k1", r#"{"a":1}"#).unwrap();
        let val = store.get("test", "k1").unwrap();
        assert_eq!(val, Some(r#"{"a":1}"#.to_string()));
    }

    #[test]
    fn upsert_overwrites() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("test").unwrap();
        store.upsert("test", "k1", "v1").unwrap();
        store.upsert("test", "k1", "v2").unwrap();
        assert_eq!(store.get("test", "k1").unwrap(), Some("v2".to_string()));
    }

    #[test]
    fn get_missing_returns_none() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("test").unwrap();
        assert_eq!(store.get("test", "nope").unwrap(), None);
    }

    #[test]
    fn get_all_and_count() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("t").unwrap();
        store.upsert("t", "a", "1").unwrap();
        store.upsert("t", "b", "2").unwrap();
        assert_eq!(store.count("t").unwrap(), 2);
        let all = store.get_all("t").unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn append_and_ordered() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("log").unwrap();
        store.append("log", "1", "first").unwrap();
        store.append("log", "2", "second").unwrap();
        let rows = store.get_all_ordered("log").unwrap();
        assert_eq!(rows[0], ("1".to_string(), "first".to_string()));
        assert_eq!(rows[1], ("2".to_string(), "second".to_string()));
    }

    #[test]
    fn delete_existing_and_missing() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("t").unwrap();
        store.upsert("t", "k", "v").unwrap();
        assert!(store.delete("t", "k").unwrap());
        assert!(!store.delete("t", "k").unwrap());
    }

    #[test]
    fn transaction_batch() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.ensure_table("t").unwrap();
        store.execute_in_transaction("t", &[("a", "1"), ("b", "2")]).unwrap();
        assert_eq!(store.count("t").unwrap(), 2);
    }
}
