use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for MCP servers - union of all transport types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpServerConfig {
    /// Local process communicating via stdin/stdout.
    #[serde(rename = "stdio")]
    Stdio(McpStdioServerConfig),

    /// Server-Sent Events transport.
    #[serde(rename = "sse")]
    Sse(McpSseServerConfig),

    /// HTTP transport.
    #[serde(rename = "http")]
    Http(McpHttpServerConfig),
}

/// stdio MCP server - local process via stdin/stdout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStdioServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// SSE MCP server - Server-Sent Events transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSseServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// HTTP MCP server - standard HTTP transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpHttpServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

impl McpServerConfig {
    /// Create a stdio MCP server config.
    pub fn stdio(command: impl Into<String>) -> Self {
        McpServerConfig::Stdio(McpStdioServerConfig {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
        })
    }

    /// Create an SSE MCP server config.
    pub fn sse(url: impl Into<String>) -> Self {
        McpServerConfig::Sse(McpSseServerConfig {
            url: url.into(),
            headers: HashMap::new(),
        })
    }

    /// Create an HTTP MCP server config.
    pub fn http(url: impl Into<String>) -> Self {
        McpServerConfig::Http(McpHttpServerConfig {
            url: url.into(),
            headers: HashMap::new(),
        })
    }
}

impl McpStdioServerConfig {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

impl McpSseServerConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: HashMap::new(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

impl McpHttpServerConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: HashMap::new(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}
