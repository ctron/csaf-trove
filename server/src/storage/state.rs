use std::path::Path;

use anyhow::Result;

use crate::models::state::SyncState;

/// Loads sync state from disk, returning a default if the file does not exist.
pub async fn load_sync_state(state_dir: &Path, domain: &str) -> Result<SyncState> {
    let dir = state_dir.join(domain);
    let path = dir.join("sync.json");
    if path.exists() {
        let data = tokio::fs::read_to_string(&path).await?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(SyncState::new(domain.to_string()))
    }
}

/// Persists sync state to disk as JSON.
pub async fn save_sync_state(state_dir: &Path, state: &SyncState) -> Result<()> {
    let dir = state_dir.join(&state.domain);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("sync.json");
    let data = serde_json::to_string_pretty(state)?;
    tokio::fs::write(&path, data).await?;
    Ok(())
}
