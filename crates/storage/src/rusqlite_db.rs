//! rusqlite-backed [`Db`] for native builds and tests.

use std::sync::Mutex;

use rusqlite::types::Value as SqliteValue;
use rusqlite::{Connection as SqliteConnection, params_from_iter};

use crate::StorageError;
use crate::db::{Db, DbValue, ExecResult, QueryRow};

/// rusqlite-backed [`Db`].
///
/// `rusqlite::Connection` is `!Sync`, so it is held behind a `Mutex`.
pub struct RusqliteDb {
    conn: Mutex<SqliteConnection>,
}

impl RusqliteDb {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
        let conn = SqliteConnection::open(path).map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn =
            SqliteConnection::open_in_memory().map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

fn to_sqlite_value(v: &DbValue) -> SqliteValue {
    match v {
        DbValue::Null => SqliteValue::Null,
        DbValue::Int(i) => SqliteValue::Integer(*i),
        DbValue::Real(f) => SqliteValue::Real(*f),
        DbValue::Text(t) => SqliteValue::Text(t.clone()),
        DbValue::Blob(b) => SqliteValue::Blob(b.clone()),
    }
}

fn to_db_value(v: SqliteValue) -> DbValue {
    match v {
        SqliteValue::Null => DbValue::Null,
        SqliteValue::Integer(i) => DbValue::Int(i),
        SqliteValue::Real(f) => DbValue::Real(f),
        SqliteValue::Text(t) => DbValue::Text(t),
        SqliteValue::Blob(b) => DbValue::Blob(b),
    }
}

impl Db for RusqliteDb {
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<ExecResult, StorageError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let values: Vec<SqliteValue> = params.iter().map(to_sqlite_value).collect();

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| StorageError::Db(e.to_string()))?;
        let column_count = stmt.column_count();

        let rows = stmt
            .query_map(params_from_iter(values.iter()), |row| {
                let mut values = Vec::with_capacity(column_count);
                for i in 0..column_count {
                    let v = row.get::<usize, SqliteValue>(i)?;
                    values.push(to_db_value(v));
                }
                Ok(QueryRow { values })
            })
            .map_err(|e| StorageError::Db(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| StorageError::Db(e.to_string()))?);
        }

        Ok(ExecResult {
            last_insert_rowid: conn.last_insert_rowid(),
            changes: conn.changes(),
            rows: out,
        })
    }
}
