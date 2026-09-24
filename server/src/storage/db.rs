use crate::models::source::sanitize_domain;
use anyhow::Result;
use csaf_trove_migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::RwLock;

/// Manages per-provider SQLite database connections with lazy initialization.
pub struct DbPool {
    results_dir: PathBuf,
    connections: RwLock<HashMap<String, DatabaseConnection>>,
}

impl DbPool {
    /// Creates a new pool that stores databases under the given results directory.
    pub fn new(results_dir: PathBuf) -> Self {
        Self {
            results_dir,
            connections: RwLock::new(HashMap::new()),
        }
    }

    /// Returns a connection for the given provider, creating the database and
    /// running migrations if needed.
    pub async fn get(&self, domain: &str) -> Result<DatabaseConnection> {
        let key = sanitize_domain(domain);

        {
            let conns = self.connections.read().await;
            if let Some(conn) = conns.get(&key) {
                return Ok(conn.clone());
            }
        }

        let mut conns = self.connections.write().await;
        if let Some(conn) = conns.get(&key) {
            return Ok(conn.clone());
        }

        let conn = self.open_and_migrate(&key).await?;
        conns.insert(key, conn.clone());
        Ok(conn)
    }

    /// Returns a connection only if the database file already exists on disk.
    pub async fn get_if_exists(&self, domain: &str) -> Result<Option<DatabaseConnection>> {
        let key = sanitize_domain(domain);

        {
            let conns = self.connections.read().await;
            if let Some(conn) = conns.get(&key) {
                return Ok(Some(conn.clone()));
            }
        }

        let db_path = self.db_path(&key);
        if !db_path.exists() {
            return Ok(None);
        }

        let mut conns = self.connections.write().await;
        if let Some(conn) = conns.get(&key) {
            return Ok(Some(conn.clone()));
        }

        let conn = self.open_and_migrate(&key).await?;
        conns.insert(key, conn.clone());
        Ok(Some(conn))
    }

    /// Returns the path to the SQLite database file for a provider key.
    fn db_path(&self, key: &str) -> PathBuf {
        self.results_dir.join(key).join("documents.db")
    }

    /// Opens a SQLite connection, sets PRAGMAs, and runs pending migrations.
    async fn open_and_migrate(&self, key: &str) -> Result<DatabaseConnection> {
        let dir = self.results_dir.join(key);
        tokio::fs::create_dir_all(&dir).await?;
        let db_path = dir.join("documents.db");

        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let mut opts = ConnectOptions::new(url);
        opts.max_connections(5).sqlx_logging(false);

        let conn = Database::connect(opts).await?;

        conn.execute_unprepared("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .await?;

        Migrator::up(&conn, None).await?;
        // TODO: drop after v0.1.5 — one-time backfill for test count columns added in v0.1.5
        super::documents::backfill_test_counts(&conn).await?;

        Ok(conn)
    }
}
