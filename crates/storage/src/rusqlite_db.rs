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
    conn: std::sync::Arc<Mutex<Option<SqliteConnection>>>,
}

impl RusqliteDb {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
        let conn = SqliteConnection::open(path).map_err(|e| StorageError::Db(e.to_string()))?;
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self {
            conn: std::sync::Arc::new(Mutex::new(Some(conn))),
        })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn =
            SqliteConnection::open_in_memory().map_err(|e| StorageError::Db(e.to_string()))?;
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self {
            conn: std::sync::Arc::new(Mutex::new(Some(conn))),
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
    type Tx<'a> = RusqliteTx;

    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<ExecResult, StorageError> {
        let mut conn_guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let conn = conn_guard.as_mut().ok_or_else(|| {
            StorageError::Db("Connection in use by transaction".to_string())
        })?;
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
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, Some(msg))
                    if err.code == rusqlite::ErrorCode::ConstraintViolation
                        && (err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                            || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY) =>
                {
                    StorageError::Conflict(msg)
                }
                _ => StorageError::Db(e.to_string()),
            })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, Some(msg))
                    if err.code == rusqlite::ErrorCode::ConstraintViolation
                        && (err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                            || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY) =>
                {
                    StorageError::Conflict(msg)
                }
                _ => StorageError::Db(e.to_string()),
            })?);
        }

        Ok(ExecResult {
            last_insert_rowid: conn.last_insert_rowid(),
            changes: conn.changes(),
            rows: out,
        })
    }

    async fn transaction<'a, F, Fut, T: 'a + Send>(&'a self, body: F) -> Result<T, StorageError>
    where
        F: FnOnce(Self::Tx<'a>) -> Fut + Send + 'a,
        Fut: std::future::Future<Output = Result<T, StorageError>> + Send + 'a,
    {
        let conn = {
            let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            guard.take().ok_or_else(|| StorageError::Db("Concurrent transactions not supported".into()))?
        };

        if let Err(e) = conn.execute("BEGIN IMMEDIATE", []) {
            let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(conn);
            return Err(StorageError::Db(e.to_string()));
        }

        let tx = RusqliteTx {
            conn: std::sync::Arc::new(std::sync::Mutex::new(Some(conn))),
        };

        struct TxGuard<'a> {
            tx_conn: std::sync::Arc<std::sync::Mutex<Option<SqliteConnection>>>,
            db_conn: &'a std::sync::Arc<Mutex<Option<SqliteConnection>>>,
            commit_requested: bool,
        }

        impl<'a> Drop for TxGuard<'a> {
            fn drop(&mut self) {
                if let Some(c) = self.tx_conn.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    if self.commit_requested {
                        let _ = c.execute("COMMIT", []);
                    } else {
                        let _ = c.execute("ROLLBACK", []);
                    }
                    let mut slot_guard = self.db_conn.lock().unwrap_or_else(|e| e.into_inner());
                    *slot_guard = Some(c);
                }
            }
        }

        let mut guard = TxGuard {
            tx_conn: tx.conn.clone(),
            db_conn: &self.conn,
            commit_requested: false,
        };

        let res = body(tx).await;
        if res.is_ok() {
            guard.commit_requested = true;
        }

        res
    }
}

pub struct RusqliteTx {
    pub(crate) conn: std::sync::Arc<std::sync::Mutex<Option<SqliteConnection>>>,
}

impl Db for RusqliteTx {
    type Tx<'b> = RusqliteTx;

    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<ExecResult, StorageError> {
        let mut conn_guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let conn = conn_guard.as_mut().ok_or_else(|| {
            StorageError::Db("Connection in use by transaction".to_string())
        })?;
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
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, Some(msg))
                    if err.code == rusqlite::ErrorCode::ConstraintViolation
                        && (err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                            || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY) =>
                {
                    StorageError::Conflict(msg)
                }
                _ => StorageError::Db(e.to_string()),
            })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| match e {
                rusqlite::Error::SqliteFailure(err, Some(msg))
                    if err.code == rusqlite::ErrorCode::ConstraintViolation
                        && (err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                            || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY) =>
                {
                    StorageError::Conflict(msg)
                }
                _ => StorageError::Db(e.to_string()),
            })?);
        }

        Ok(ExecResult {
            last_insert_rowid: conn.last_insert_rowid(),
            changes: conn.changes(),
            rows: out,
        })
    }

    async fn transaction<'b, F, Fut, T: 'b>(&'b self, _body: F) -> Result<T, StorageError>
    where
        F: FnOnce(Self::Tx<'b>) -> Fut + Send + 'b,
        Fut: std::future::Future<Output = Result<T, StorageError>> + Send + 'b,
    {
        Err(StorageError::Db("Nested transactions are not supported".into()))
    }
}

