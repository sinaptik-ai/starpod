# starpod-vault

AES-256-GCM encrypted credential storage in SQLite with audit logging.

## API

```rust
let vault = Vault::new(&db_path, &master_key).await?;

// Store a credential
vault.set("github_token", "ghp_xxx", Some("user_123")).await?;

// Retrieve (decrypted)
let value = vault.get("github_token", Some("user_123")).await?; // Option<String>

// List all keys
let keys = vault.list_keys().await?; // Vec<String>

// Delete
vault.delete("github_token", None).await?;
```

## System Keys

`SYSTEM_KEYS` is a centralized list of environment variable names that hold
system-managed secrets (LLM provider keys, service tokens, platform secrets).
The `is_system_key(key)` helper performs a case-insensitive check against this
list.

System keys are protected at two layers:
- The `EnvGet` agent tool uses `is_system_key()` to block reads and return an error.
- The `ToolExecutor` Bash runner uses `env_blocklist` (populated from `SYSTEM_KEYS`) to strip them from child process environments via `env_remove()`.

| Category | Keys |
|----------|------|
| LLM providers | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GROQ_API_KEY`, `DEEPSEEK_API_KEY`, `OPENROUTER_API_KEY` |
| Services | `BRAVE_API_KEY`, `TELEGRAM_BOT_TOKEN` |
| Platform | `STARPOD_API_KEY` |

## Encryption

- **Algorithm**: AES-256-GCM
- **Master key**: 32-byte array (derived from API key in production)
- **Storage**: SQLite database
- **Audit**: All get/set/delete operations are logged with optional `user_id`

## Tests

10 unit tests + 3 doc-tests.
