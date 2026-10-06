//! Observes real application connections without keeping idle connections alive.

use anyhow::{Result, ensure};
use libsqlite3_sys::{
    SQLITE_DBSTATUS_CACHE_USED, SQLITE_DBSTATUS_SCHEMA_USED, SQLITE_DBSTATUS_STMT_USED, SQLITE_OK,
    sqlite3_db_status,
};
use parking_lot::Mutex;
use sea_orm::ConnectOptions;
use sqlx::SqliteConnection;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Minimum interval between released-connection samples for each provider.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

/// Samples one returning connection per interval; never creates or borrows a connection.
pub(super) fn configure(options: &mut ConnectOptions, domain: &str) {
    let domain = domain.to_owned();
    let last_sample = Arc::new(Mutex::new(None::<Instant>));
    options.map_sqlx_sqlite_pool_opts(move |pool| {
        let domain = domain.clone();
        let last_sample = last_sample.clone();
        pool.after_release(move |connection, _| {
            let domain = domain.clone();
            let sample = {
                let mut last = last_sample.lock();
                if last.is_none_or(|time| time.elapsed() >= SAMPLE_INTERVAL) {
                    *last = Some(Instant::now());
                    true
                } else {
                    false
                }
            };
            Box::pin(async move {
                if sample {
                    match connection_memory(connection).await {
                        Ok([cache_bytes, statement_bytes, schema_bytes]) => tracing::info!(
                            domain,
                            cache_bytes,
                            statement_bytes,
                            schema_bytes,
                            "SQLite released connection sample"
                        ),
                        Err(error) => {
                            tracing::warn!(%domain, %error, "SQLite memory sample failed")
                        }
                    }
                }
                // Diagnostics must not evict connections or fail application queries.
                Ok(true)
            })
        })
    });
}

/// Reads current cache, statement and schema bytes with exclusive access to the handle.
async fn connection_memory(connection: &mut SqliteConnection) -> Result<[i32; 3]> {
    let mut handle = connection.lock_handle().await?;
    let mut values = [0; 3];
    for (value, operation) in values.iter_mut().zip([
        SQLITE_DBSTATUS_CACHE_USED,
        SQLITE_DBSTATUS_STMT_USED,
        SQLITE_DBSTATUS_SCHEMA_USED,
    ]) {
        let mut high_water = 0;
        // SAFETY: SQLx's guard excludes its worker from this live handle. Both output
        // pointers remain valid for the call; reset=0 leaves statistics unchanged.
        let status = unsafe {
            sqlite3_db_status(
                handle.as_raw_handle().as_ptr(),
                operation,
                value,
                &mut high_water,
                0,
            )
        };
        ensure!(
            status == SQLITE_OK,
            "SQLite db_status({operation}) returned {status}"
        );
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Connection, Executor, query, query_scalar};

    /// Counters observe real allocations without resetting them or altering stored data.
    #[tokio::test]
    async fn measures_connection_allocations() {
        let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
        connection
            .execute("CREATE TABLE sample (data BLOB)")
            .await
            .unwrap();
        query("INSERT INTO sample VALUES (zeroblob(65536))")
            .execute(&mut connection)
            .await
            .unwrap();
        let before = connection_memory(&mut connection).await.unwrap();
        assert!(before.iter().all(|bytes| *bytes > 0), "{before:?}");
        assert_eq!(before, connection_memory(&mut connection).await.unwrap());
        let length: i64 = query_scalar("SELECT length(data) FROM sample")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(length, 65536);
    }
}
