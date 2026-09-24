//! Spin `sqlite` capability-backed [`Db`] for wasm builds.

use spin_sdk::sqlite::{Connection, Value};

use crate::StorageError;
use crate::db::{Db, DbValue, ExecResult, QueryRow};

#[derive(Clone)]
enum SpinTarget {
    Default,
    Named(String),
}

#[derive(Clone)]
enum ConnectionState {
    Idle(std::sync::Arc<Connection>),
    InTransaction,
    Poisoned,
}

/// Spin `sqlite` capability-backed [`Db`].
#[derive(Clone)]
pub struct SpinDb {
    target: std::sync::Arc<SpinTarget>,
    state: std::sync::Arc<std::sync::Mutex<ConnectionState>>,
}

impl SpinDb {
    /// Open the default database declared in the Spin manifest.
    pub async fn open_default() -> Result<Self, StorageError> {
        let conn = Connection::open_default()
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;
        conn.execute("PRAGMA foreign_keys = ON", vec![])
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self {
            target: std::sync::Arc::new(SpinTarget::Default),
            state: std::sync::Arc::new(std::sync::Mutex::new(ConnectionState::Idle(
                std::sync::Arc::new(conn),
            ))),
        })
    }

    /// Open a named database declared in the Spin manifest.
    pub async fn open(name: impl AsRef<str>) -> Result<Self, StorageError> {
        let conn = Connection::open(name.as_ref())
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;
        conn.execute("PRAGMA foreign_keys = ON", vec![])
            .await
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(Self {
            target: std::sync::Arc::new(SpinTarget::Named(name.as_ref().to_string())),
            state: std::sync::Arc::new(std::sync::Mutex::new(ConnectionState::Idle(
                std::sync::Arc::new(conn),
            ))),
        })
    }

    async fn acquire_conn(&self, for_tx: bool) -> Result<std::sync::Arc<Connection>, StorageError> {
        loop {
            let is_poisoned = {
                let mut state = self.state.lock().unwrap();
                match &*state {
                    ConnectionState::Idle(c) => {
                        let c = c.clone();
                        if for_tx {
                            *state = ConnectionState::InTransaction;
                        }
                        return Ok(c);
                    }
                    ConnectionState::InTransaction => {
                        let msg = if for_tx {
                            "Concurrent transactions not supported"
                        } else {
                            "Connection in use by transaction"
                        };
                        return Err(StorageError::Db(msg.into()));
                    }
                    ConnectionState::Poisoned => true,
                }
            };

            if is_poisoned {
                let new_conn = match &*self.target {
                    SpinTarget::Default => Connection::open_default().await,
                    SpinTarget::Named(name) => Connection::open(name).await,
                }
                .map_err(|e| StorageError::Db(e.to_string()))?;

                new_conn
                    .execute("PRAGMA foreign_keys = ON", vec![])
                    .await
                    .map_err(|e| StorageError::Db(e.to_string()))?;

                let mut state = self.state.lock().unwrap();
                if matches!(*state, ConnectionState::Poisoned) {
                    *state = ConnectionState::Idle(std::sync::Arc::new(new_conn));
                }
            }
        }
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

fn map_spin_err(e: spin_sdk::sqlite::Error) -> StorageError {
    match e {
        spin_sdk::sqlite::Error::Io(msg)
            if msg.contains("UNIQUE constraint failed")
                || msg.contains("PRIMARY KEY constraint failed") =>
        {
            StorageError::Conflict(msg)
        }
        _ => StorageError::Db(e.to_string()),
    }
}

impl Db for SpinDb {
    type Tx<'a> = SpinTx;

    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<ExecResult, StorageError> {
        let values: Vec<Value> = params.iter().map(to_spin_value).collect();
        let conn = self.acquire_conn(false).await?;

        let mut result = conn.execute(sql, values).await.map_err(map_spin_err)?;

        let mut rows = Vec::new();
        while let Some(row) = result.next().await {
            rows.push(QueryRow {
                values: row.values.iter().map(to_db_value).collect(),
            });
        }
        result.result().await.map_err(map_spin_err)?;

        Ok(ExecResult {
            last_insert_rowid: conn.last_insert_rowid().await,
            changes: conn.changes().await,
            rows,
        })
    }

    async fn transaction<'a, F, Fut, T: 'a + Send>(&'a self, body: F) -> Result<T, StorageError>
    where
        F: FnOnce(Self::Tx<'a>) -> Fut + Send + 'a,
        Fut: std::future::Future<Output = Result<T, StorageError>> + Send + 'a,
    {
        let conn = self.acquire_conn(true).await?;

        if let Err(e) = conn.execute("BEGIN IMMEDIATE", vec![]).await {
            *self.state.lock().unwrap() = ConnectionState::Idle(conn);
            return Err(map_spin_err(e));
        }

        let tx_db = SpinDb {
            target: self.target.clone(),
            state: std::sync::Arc::new(std::sync::Mutex::new(ConnectionState::Idle(conn.clone()))),
        };
        let tx = SpinTx { db: tx_db };

        struct TxGuard {
            tx_state: std::sync::Arc<std::sync::Mutex<ConnectionState>>,
            db_state: std::sync::Arc<std::sync::Mutex<ConnectionState>>,
        }

        impl Drop for TxGuard {
            fn drop(&mut self) {
                let mut tx_s = self.tx_state.lock().unwrap();
                if let ConnectionState::Idle(c) =
                    std::mem::replace(&mut *tx_s, ConnectionState::Poisoned)
                {
                    drop(c);
                    *self.db_state.lock().unwrap() = ConnectionState::Poisoned;
                }
            }
        }

        let guard = TxGuard {
            tx_state: tx.db.state.clone(),
            db_state: self.state.clone(),
        };

        let res = body(tx).await;

        let taken_state = {
            let mut s = guard.tx_state.lock().unwrap();
            std::mem::replace(&mut *s, ConnectionState::Poisoned)
        };

        if let ConnectionState::Idle(c) = taken_state {
            if res.is_ok() {
                let _ = c.execute("COMMIT", vec![]).await;
            } else {
                let _ = c.execute("ROLLBACK", vec![]).await;
            }
            *self.state.lock().unwrap() = ConnectionState::Idle(c);
        }

        res
    }
}

pub struct SpinTx {
    pub(crate) db: SpinDb,
}

impl Db for SpinTx {
    type Tx<'b> = SpinTx;

    async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<ExecResult, StorageError> {
        self.db.execute(sql, params).await
    }

    async fn transaction<'b, F, Fut, T: 'b>(&'b self, _body: F) -> Result<T, StorageError>
    where
        F: FnOnce(Self::Tx<'b>) -> Fut + Send + 'b,
        Fut: std::future::Future<Output = Result<T, StorageError>> + Send + 'b,
    {
        Err(StorageError::Db(
            "Nested transactions are not supported".into(),
        ))
    }
}
