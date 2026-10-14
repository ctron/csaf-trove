use crate::models::state::SyncState;
use anyhow::Result;
use std::path::Path;
use tokio::fs;

/// Lists all persisted sync states from subdirectories.
pub async fn list_sync_states(state_dir: &Path) -> Result<Vec<SyncState>> {
    let mut states = Vec::new();
    if !state_dir.exists() {
        return Ok(states);
    }
    let mut entries = fs::read_dir(state_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            let path = entry.path().join("sync.json");
            if path.exists() {
                let data = fs::read_to_string(&path).await?;
                if let Ok(state) = serde_json::from_str::<SyncState>(&data) {
                    states.push(state);
                }
            }
        }
    }
    Ok(states)
}
