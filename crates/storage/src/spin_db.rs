//! Spin `sqlite` capability-backed [`Db`] for wasm builds.

use spin_sdk::sqlite::{Connection, Value};

use crate::StorageError;
use crate::db::{Db, DbValue, ExecResult, QueryRow};

/// Spin `sqlite` capability-backed [`Db`].
pub struct SpinDb {
    conn: Connection,
}

impl SpinDb {
    /// Open the default database declared in the Spin manifest.
    pub async fn open_default() -> Result<Self, StorageError> {
        let conn = Connection::open_default()
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self { conn })
    }

    /// Open a named database declared in the Spin manifest.
    pub async fn open(name: impl AsRef<str>) -> Result<Self, StorageError> {
        let conn = Connection::open(name.as_ref())
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self { conn })
    }
}

fn to_spin_value(v: &DbValue) -> Value {
    match v {
        DbValue::Null => Value::Null,
        DbValue::Int(i) => Value::Integer(*i),
        DbValue::Real(f) => Value::Real(*f),
        DbValue::Text(t) => Value::Text(t.clone()),
        DbValue::Blob(b) => Value::Blob(b.clone()),
    }
}

fn to_db_value(v: &Value) -> DbValue {
    match v {
        Value::Null => DbValue::Null,
        Value::Integer(i) => DbValue::Int(*i),
        Value::Real(f) => DbValue::Real(*f),
        Value::Text(t) => DbValue::Text(t.clone()),
        Value::Blob(b) => DbValue::Blob(b.clone()),
    }
}

impl Db for SpinDb {
    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<ExecResult, StorageError> {
        let values: Vec<Value> = params.iter().map(to_spin_value).collect();

        let mut result = self
            .conn
            .execute(sql, values)
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;

        let mut rows = Vec::new();
        while let Some(row) = result.next().await {
            rows.push(QueryRow {
                values: row.values.iter().map(to_db_value).collect(),
            });
        }
        result
            .result()
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;

        Ok(ExecResult {
            last_insert_rowid: self.conn.last_insert_rowid().await,
            changes: self.conn.changes().await,
            rows,
        })
    }
}
