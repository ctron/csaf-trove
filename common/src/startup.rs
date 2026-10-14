use serde::{Deserialize, Serialize};

/// Server startup progress, reported by `GET /api/startup`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupStatus {
    /// Whether startup has finished and all endpoints are available.
    pub ready: bool,
    /// Task currently running, if any.
    pub task: Option<String>,
    /// Items processed so far within the current task.
    pub current: u64,
    /// Total items of the current task, `0` if unknown.
    pub total: u64,
    /// Tasks already finished, in order.
    pub completed: Vec<String>,
}
