#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod api;
mod models;
mod pipeline;
mod scheduler;
mod storage;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use anyhow::{Context, Result};
use clap::Parser;
use rust_embed::Embed;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing_actix_web::TracingLogger;

use crate::{
    models::{source::Source, state::JobStatus},
    storage::Storage,
};

#[derive(Parser)]
#[command(name = "csaf-trove-server")]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "/etc/csaf-trove/config.toml")]
    config: PathBuf,
}

/// Top-level server configuration.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// HTTP server settings.
    pub server: ServerConfig,
    /// Data directory paths.
    pub data: DataConfig,
    /// GitHub integration settings.
    pub github: GithubConfig,
    /// Scheduler settings.
    pub scheduler: SchedulerConfig,
}

/// HTTP server configuration.
#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Socket address to listen on (e.g. `0.0.0.0:8080`).
    pub listen: String,
    /// Path to the file containing the bearer token for authenticating POST API requests.
    pub api_token_file: Option<PathBuf>,
}

/// Data directory configuration.
#[derive(Debug, Deserialize)]
pub struct DataConfig {
    /// Root directory for repos, state, results, and metrics.
    pub dir: PathBuf,
}

/// GitHub integration configuration.
#[derive(Debug, Deserialize)]
pub struct GithubConfig {
    /// URL of the csaf-trove GitHub repository.
    pub repo: String,
    /// Path to the file containing the webhook secret for GitHub signature verification.
    pub webhook_secret_file: Option<PathBuf>,
    /// Interval between polling GitHub for config changes (e.g. `5m`).
    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    pub poll_interval: Duration,
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(300)
}

/// Scheduler configuration.
#[derive(Debug, Deserialize)]
pub struct SchedulerConfig {
    /// Interval between full sync runs (e.g. `1d`, `12h`).
    #[serde(default = "default_sync_interval", with = "humantime_serde")]
    pub sync_interval: Duration,
    /// Maximum providers syncing in parallel.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_sync_interval() -> Duration {
    Duration::from_secs(86400)
}

fn default_max_concurrent() -> u32 {
    5
}

/// Shared application state accessible from handlers and the scheduler.
pub struct AppState {
    /// Server configuration.
    pub config: Config,
    /// Persistent storage layer.
    pub storage: Storage,
    /// Currently loaded provider sources, keyed by domain.
    pub sources: RwLock<HashMap<String, Source>>,
    /// In-flight and recent job statuses, keyed by domain.
    pub jobs: RwLock<HashMap<String, JobStatus>>,
    /// Loaded API bearer token for authenticating POST requests.
    pub api_token: Option<String>,
    /// Loaded webhook secret for verifying GitHub signatures.
    pub webhook_secret: Option<String>,
    /// Root data directory.
    data_dir: PathBuf,
}

impl AppState {
    /// Returns the scratch directory for temporary worktrees.
    pub fn work_dir(&self) -> PathBuf {
        self.data_dir.join("work")
    }

    /// Reloads provider sources from the sources directory on disk.
    pub async fn reload_sources(&self) {
        match load_sources_from_dir(&self.data_dir.join("sources")).await {
            Ok(sources) => {
                let mut current = self.sources.write().await;
                *current = sources;
                tracing::info!("Reloaded {} sources", current.len());
            }
            Err(e) => {
                tracing::error!("Failed to reload sources: {e}");
            }
        }
    }

    /// Inserts or replaces the job status for a provider.
    pub async fn update_job(&self, domain: &str, status: JobStatus) {
        self.jobs.write().await.insert(domain.to_string(), status);
    }

    /// Returns the current job status for a provider, if any.
    pub async fn get_job(&self, domain: &str) -> Option<JobStatus> {
        self.jobs.read().await.get(domain).cloned()
    }

    /// Updates only the phase field of an existing job.
    pub async fn update_job_phase(&self, domain: &str, phase: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.get_mut(domain) {
            job.phase = Some(phase.to_string());
        }
    }
}

/// Pre-built WASM dashboard files embedded at compile time.
#[derive(Embed)]
#[folder = "../dashboard/dist/"]
struct DashboardAssets;

/// Serves embedded dashboard assets, falling back to `index.html` for SPA routing.
async fn serve_dashboard(req: HttpRequest) -> HttpResponse {
    let path = req.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match DashboardAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            HttpResponse::Ok()
                .content_type(mime.as_ref())
                .body(file.data.into_owned())
        }
        None => match DashboardAssets::get("index.html") {
            Some(file) => HttpResponse::Ok()
                .content_type("text/html")
                .body(file.data.into_owned()),
            None => HttpResponse::NotFound().finish(),
        },
    }
}

/// Reads a secret from a plain text file, trimming whitespace.
fn read_secret_file(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read secret from {}", path.display()))?;
    let secret = content.trim().to_string();
    anyhow::ensure!(
        !secret.is_empty(),
        "Secret file is empty: {}",
        path.display()
    );
    Ok(secret)
}

async fn load_sources_from_dir(dir: &Path) -> Result<HashMap<String, Source>> {
    let mut sources = HashMap::new();

    if !dir.exists() {
        return Ok(sources);
    }

    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            let data = tokio::fs::read_to_string(&path).await?;
            match toml::from_str::<Source>(&data) {
                Ok(source) => {
                    sources.insert(source.domain.clone(), source);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse source {}: {e}", path.display());
                }
            }
        }
    }

    Ok(sources)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,csaf_trove_server=debug")
            }),
        )
        .init();

    let cli = Cli::parse();

    let config_str = fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config from {}", cli.config.display()))?;
    let config: Config = toml::from_str(&config_str).context("Failed to parse config")?;

    let listen = config.server.listen.clone();
    let data_dir = config.data.dir.clone();

    let storage = Storage::new(&data_dir).context("Failed to initialize storage")?;

    fs::create_dir_all(data_dir.join("work"))?;
    fs::create_dir_all(data_dir.join("sources"))?;

    let api_token = config
        .server
        .api_token_file
        .as_ref()
        .map(|path| read_secret_file(path))
        .transpose()
        .context("Failed to read API token")?;

    let webhook_secret = config
        .github
        .webhook_secret_file
        .as_ref()
        .map(|path| read_secret_file(path))
        .transpose()
        .context("Failed to read webhook secret")?;

    let sources = load_sources_from_dir(&data_dir.join("sources"))
        .await
        .unwrap_or_default();
    tracing::info!("Loaded {} sources", sources.len());

    let state = Arc::new(AppState {
        config,
        storage,
        sources: RwLock::new(sources),
        jobs: RwLock::new(HashMap::new()),
        api_token,
        webhook_secret,
        data_dir,
    });

    let scheduler_state = state.clone();
    tokio::spawn(async move {
        scheduler::run(scheduler_state).await;
    });

    tracing::info!("Starting server on {listen}");

    anyhow::ensure!(
        DashboardAssets::get("index.html").is_some(),
        "Dashboard assets not embedded. Build the dashboard first: cd dashboard && trunk build"
    );

    let server_state = state.clone();
    HttpServer::new(move || {
        App::new()
            .wrap(TracingLogger::default())
            .app_data(web::Data::from(server_state.clone()))
            .service(web::scope("/api").configure(api::config))
            .default_service(web::route().to(serve_dashboard))
    })
    .bind(&listen)?
    .run()
    .await?;

    Ok(())
}
