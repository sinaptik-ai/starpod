# System

Endpoints for version checking and self-updating. All endpoints require admin authentication.

## GET /api/system/version

Check the current binary version against the latest GitHub release.

```bash
curl -H "X-API-Key: sp_live_..." http://localhost:3000/api/system/version
```

### Response

```json
{
  "current": "0.2.1",
  "latest": "0.3.0",
  "update_available": true,
  "release_notes_url": "https://github.com/sinaptik-ai/starpod/releases/tag/v0.3.0",
  "published_at": "2026-03-30T10:00:00Z",
  "platform": "aarch64-apple-darwin"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `current` | string | Compile-time version of the running binary |
| `latest` | string \| null | Latest version from GitHub (null if check failed) |
| `update_available` | boolean | `true` when `latest` is strictly newer than `current` |
| `release_notes_url` | string \| null | Link to the GitHub release page |
| `published_at` | string \| null | ISO 8601 publication timestamp |
| `platform` | string | Target triple of the running binary |

The response is cached in memory for 1 hour. If the GitHub API is unreachable, `latest` will be `null` and a stale cached value is returned if available.

## POST /api/system/update

Trigger a self-update to the latest version. The update runs in the background — the endpoint returns immediately.

```bash
curl -X POST -H "X-API-Key: sp_live_..." http://localhost:3000/api/system/update
```

### Response

```json
{
  "status": "updating",
  "version": "0.3.0",
  "message": "Update started. Starpod will restart automatically."
}
```

### Update pipeline

1. **Download** — Fetches the platform-appropriate `.tar.gz` from GitHub Releases
2. **Verify** — Checks SHA-256 against the release checksums (if available)
3. **Backup** — Copies to `.starpod/backups/`:
   - `starpod-{old_version}` — current binary
   - `db-{old_version}/` — all `.db` files
   - `agent-{old_version}.toml` — config file
4. **Replace** — Swaps the binary (rename old to `.bak`, write new, `chmod +x`)
5. **Restart** — Spawns the new binary with the same CLI arguments
6. **Monitor** — Watches the new process for 30 seconds; rolls back on early crash

### Errors

| Status | Meaning |
|--------|---------|
| `409` | Already on the latest version |
| `404` | No release asset found for this platform |
| `502` | Cannot reach GitHub to fetch release info |

### Frontend polling

After receiving the response, poll `GET /api/health` every 2 seconds. When it returns the new version, the update is complete.

## Rollback

If the new binary crashes within 30 seconds of spawning, the old binary automatically restores itself. For manual recovery, backups are stored in `.starpod/backups/`.
