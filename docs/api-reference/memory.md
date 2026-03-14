# Memory

## Search {#search}

### GET /api/memory/search

Full-text search across all indexed memory and knowledge files.

```bash
curl "http://localhost:3000/api/memory/search?query=database+migrations&limit=5" \
  -H "X-API-Key: your-key"
```

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `query` | string | — | Search query (required) |
| `limit` | integer | `10` | Maximum results |

#### Response

```json
[
  {
    "source": "knowledge/databases.md",
    "text": "## Migrations\n\nWe use sqlx for database migrations...",
    "line_start": 15,
    "line_end": 28,
    "rank": -4.21
  }
]
```

#### Response Fields

| Field | Type | Description |
|-------|------|-------------|
| `source` | string | File path relative to the data directory |
| `text` | string | Matching text chunk |
| `line_start` | integer | First line of the chunk in the source file |
| `line_end` | integer | Last line of the chunk |
| `rank` | float | FTS5 relevance rank (lower = more relevant) |

## Reindex {#reindex}

### POST /api/memory/reindex

Rebuild the FTS5 search index. Run this after manually editing files in `.starpod/data/`.

```bash
curl -X POST http://localhost:3000/api/memory/reindex \
  -H "X-API-Key: your-key"
```

#### Response

```json
{
  "status": "ok"
}
```
