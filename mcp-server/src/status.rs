//! Server status and stable failure codes shown to the desktop host.

use std::fmt;

use serde::Serialize;

/// Why the server could not start or stopped serving. The camelCase strings
/// are the stable values the UI keys its messages on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum McpFailureCode {
    /// Another live process already owns the endpoint.
    EndpointInUse,
    /// The socket or pipe could not be created for a reason not listed below.
    EndpointCreateFailed,
    /// The operating system refused access to the endpoint or its directory.
    PermissionDenied,
    /// The discovery file could not be written.
    DiscoveryWriteFailed,
    /// No secure random token could be generated.
    TokenUnavailable,
    /// The listener failed after the server had started.
    AcceptFailed,
}

impl McpFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EndpointInUse => "endpointInUse",
            Self::EndpointCreateFailed => "endpointCreateFailed",
            Self::PermissionDenied => "permissionDenied",
            Self::DiscoveryWriteFailed => "discoveryWriteFailed",
            Self::TokenUnavailable => "tokenUnavailable",
            Self::AcceptFailed => "acceptFailed",
        }
    }
}

impl fmt::Display for McpFailureCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned by [`crate::McpServer::start`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct McpStartError {
    pub code: McpFailureCode,
}

impl fmt::Display for McpStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MCP server failed to start: {}", self.code)
    }
}

impl std::error::Error for McpStartError {}

/// One open session. The label is self-reported by the client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct McpClient {
    /// `clientInfo.title` if present, else `clientInfo.name`, sanitized.
    /// `None` until the client identifies itself.
    pub label: Option<String>,
}

/// What the settings page shows. Serializes as `{"state":"running",...}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum McpStatus {
    #[default]
    Off,
    #[serde(rename_all = "camelCase")]
    Running {
        /// Authenticated sessions currently open, in connection order.
        clients: Vec<McpClient>,
    },
    #[serde(rename_all = "camelCase")]
    Failed { code: McpFailureCode },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn status_serializes_with_stable_shape() {
        assert_eq!(
            serde_json::to_value(McpStatus::Off).unwrap(),
            json!({"state": "off"})
        );
        assert_eq!(
            serde_json::to_value(McpStatus::Running {
                clients: vec![
                    McpClient {
                        label: Some("x".into())
                    },
                    McpClient { label: None }
                ]
            })
            .unwrap(),
            json!({"state": "running", "clients": [{"label": "x"}, {"label": null}]})
        );
        assert_eq!(
            serde_json::to_value(McpStatus::Failed {
                code: McpFailureCode::EndpointInUse
            })
            .unwrap(),
            json!({"state": "failed", "code": "endpointInUse"})
        );
    }
}
