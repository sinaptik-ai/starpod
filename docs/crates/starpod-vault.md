# starpod-vault

AES-256-GCM encrypted credential storage in SQLite with audit logging.

## API

```rust
let vault = Vault::new(&db_path, &master_key).await?;

// Store a credential
vault.set("github_token", "ghp_xxx").await?;

// Retrieve (decrypted)
let value = vault.get("github_token").await?; // Option<String>

// List all keys
let keys = vault.list_keys().await?; // Vec<String>

// Delete
vault.delete("github_token").await?;
```

## Encryption

- **Algorithm**: AES-256-GCM
- **Master key**: 32-byte array (derived from API key in production)
- **Storage**: SQLite database
- **Audit**: All get/set/delete operations are logged

## Tests

7 unit tests.
