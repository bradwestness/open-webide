//! Target-agnostic database abstraction.

use std::future::Future;

use crate::StorageError;

/// A single SQL parameter or result value.
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// One row of query results.
#[derive(Debug, Clone)]
pub struct QueryRow {
    pub values: Vec<DbValue>,
}

impl QueryRow {
    pub fn get_text(&self, index: usize) -> Result<&str, StorageError> {
        match self.values.get(index) {
            Some(DbValue::Text(t)) => Ok(t),
            Some(DbValue::Blob(b)) => std::str::from_utf8(b)
                .map_err(|_| StorageError::InvalidValue("blob is not valid UTF-8".into())),
            other => Err(StorageError::InvalidValue(format!(
                "column {index} is not text: {other:?}"
            ))),
        }
    }

    pub fn get_text_opt(&self, index: usize) -> Option<&str> {
        match self.values.get(index) {
            Some(DbValue::Text(t)) => Some(t),
            Some(DbValue::Blob(b)) => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }

    pub fn get_int(&self, index: usize) -> Result<i64, StorageError> {
        match self.values.get(index) {
            Some(DbValue::Int(i)) => Ok(*i),
            other => Err(StorageError::InvalidValue(format!(
                "column {index} is not an integer: {other:?}"
            ))),
        }
    }
}

/// The outcome of executing a statement.
#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    pub last_insert_rowid: i64,
    pub changes: u64,
    pub rows: Vec<QueryRow>,
}

/// The database surface the repositories depend on.
pub trait Db: Send + Sync {
    fn execute(
        &self,
        sql: &str,
        params: &[DbValue],
    ) -> impl Future<Output = Result<ExecResult, StorageError>> + Send;
}
