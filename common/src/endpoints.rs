use serde::{Deserialize, Serialize};

/// Public endpoint availability configured by the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoints {
    /// Whether the MCP endpoint is enabled.
    pub mcp: bool,
    /// Whether CSAF aggregator generation is enabled.
    pub aggregator: bool,
}
