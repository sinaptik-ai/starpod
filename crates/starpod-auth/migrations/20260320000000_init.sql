-- Users table
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    display_name TEXT,
    role TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user')),
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- API keys table
CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL,
    label TEXT,
    expires_at TEXT,
    revoked_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(prefix);
CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys(user_id);

-- Telegram links table
CREATE TABLE IF NOT EXISTS telegram_links (
    telegram_id INTEGER PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    username TEXT,
    linked_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_telegram_links_user_id ON telegram_links(user_id);

-- Audit log
CREATE TABLE IF NOT EXISTS auth_audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT,
    event_type TEXT NOT NULL,
    detail TEXT,
    ip_address TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_auth_audit_log_user_id ON auth_audit_log(user_id);
CREATE INDEX IF NOT EXISTS idx_auth_audit_log_created_at ON auth_audit_log(created_at);
