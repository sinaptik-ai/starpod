-- Connectors: service-level authentication abstraction.
-- Each row represents a configured connection to an external service.
-- Secrets live in the vault; this table stores resolved vault key names.

CREATE TABLE IF NOT EXISTS connectors (
    name TEXT PRIMARY KEY,                          -- instance name ("github", "analytics-db")
    type TEXT NOT NULL,                             -- template name ("github", "postgres")
    display_name TEXT NOT NULL,                     -- human-readable label
    description TEXT NOT NULL,                      -- what this connection is for
    auth_method TEXT NOT NULL DEFAULT 'token',      -- "token" or "oauth"
    secrets TEXT NOT NULL DEFAULT '[]',             -- JSON array of resolved vault keys
    config TEXT NOT NULL DEFAULT '{}',              -- JSON object of config values
    oauth_token_url TEXT,                           -- token endpoint for refresh
    oauth_token_key TEXT,                           -- vault key for access token
    oauth_refresh_key TEXT,                         -- vault key for refresh token
    oauth_expires_at TEXT,                          -- ISO 8601 expiry
    status TEXT NOT NULL DEFAULT 'pending',         -- "connected", "pending", "error"
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
