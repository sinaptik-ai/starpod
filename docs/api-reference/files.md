# Files

Browse and manage files in the agent's home directory sandbox. All endpoints require authentication and the `filesystem_enabled` flag to be set on the user.

When auth is disabled (no users in the database), all endpoints are accessible without authentication.

## Security

- Paths must be relative to the home directory (e.g., `notes.txt`, `docs/readme.md`)
- Absolute paths, `..` traversal, and access to `.starpod/` are rejected
- Existing paths are canonicalized to prevent symlink escapes

## List Directory {#list}

### GET /api/files

```bash
# List home directory
curl http://localhost:3000/api/files \
  -H "X-API-Key: your-key"

# List subdirectory
curl "http://localhost:3000/api/files?path=docs" \
  -H "X-API-Key: your-key"
```

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `path` | string | `.` | Relative directory path |

#### Response

```json
[
  {
    "name": "docs/",
    "type": "directory",
    "size": 0
  },
  {
    "name": "notes.txt",
    "type": "file",
    "size": 1234
  }
]
```

Entries are sorted alphabetically. Directory names have a trailing `/`. The `.starpod/` directory is always hidden.

## Read File {#read}

### GET /api/files/read

```bash
curl "http://localhost:3000/api/files/read?path=notes.txt" \
  -H "X-API-Key: your-key"
```

#### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Relative file path |

#### Response

```json
{
  "path": "notes.txt",
  "content": "File content as UTF-8 text",
  "size": 26
}
```

| Status | Condition |
|--------|-----------|
| `200` | File read successfully |
| `400` | Path points to a directory |
| `404` | File does not exist |
| `422` | File is binary (not valid UTF-8) |

## Write File {#write}

### PUT /api/files/write

Creates or overwrites a file. Parent directories are created automatically.

```bash
curl -X PUT http://localhost:3000/api/files/write \
  -H "X-API-Key: your-key" \
  -H "Content-Type: application/json" \
  -d '{"path": "docs/readme.md", "content": "# Hello\n\nWelcome."}'
```

#### Request Body

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Relative file path |
| `content` | string | yes | File content (UTF-8 text) |

#### Response

```json
{ "status": "ok" }
```

## Delete File or Directory {#delete}

### DELETE /api/files

Deletes a file or directory. Directories are removed recursively.

```bash
curl -X DELETE "http://localhost:3000/api/files?path=old-notes.txt" \
  -H "X-API-Key: your-key"
```

#### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Relative path to delete |

#### Response

```json
{ "status": "ok" }
```

Returns `404` if the path does not exist.

## Create Directory {#mkdir}

### POST /api/files/mkdir

Creates a directory and all parent directories.

```bash
curl -X POST http://localhost:3000/api/files/mkdir \
  -H "X-API-Key: your-key" \
  -H "Content-Type: application/json" \
  -d '{"path": "projects/2026/q1"}'
```

#### Request Body

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | string | yes | Relative directory path |

#### Response

```json
{ "status": "ok" }
```

---

## Enabling Filesystem Access

Filesystem access is controlled per-user via the `filesystem_enabled` flag. It defaults to `false` for new users and is automatically set to `true` for the bootstrap admin.

To toggle it for an existing user:

```bash
curl -X PUT http://localhost:3000/api/settings/auth/users/{user-id} \
  -H "X-API-Key: admin-key" \
  -H "Content-Type: application/json" \
  -d '{"filesystem_enabled": true}'
```

The current user's filesystem access status is returned in the `GET /api/auth/verify` response:

```json
{
  "authenticated": true,
  "auth_disabled": false,
  "user": {
    "id": "...",
    "display_name": "Alice",
    "role": "user",
    "filesystem_enabled": true
  }
}
```
