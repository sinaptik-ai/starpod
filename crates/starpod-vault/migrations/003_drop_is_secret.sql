CREATE TABLE vault_entries_new (
    key TEXT PRIMARY KEY,
    encrypted_value BLOB NOT NULL,
    nonce BLOB NOT NULL,
    allowed_hosts TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
INSERT INTO vault_entries_new SELECT key, encrypted_value, nonce, allowed_hosts, created_at, updated_at FROM vault_entries;
DROP TABLE vault_entries;
ALTER TABLE vault_entries_new RENAME TO vault_entries;
