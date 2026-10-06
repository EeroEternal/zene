# Session backends

`zene-session` persists agent sessions through the `SessionStore` trait. Three
backends ship in-tree; the active one is picked at agent build time from the
environment.

| Backend | Selected by | Storage |
| --- | --- | --- |
| `FileSessionStore` | default | one JSON file per session under `~/.zene/sessions` |
| `SqliteSessionStore` | `ZENE_SESSION_SQLITE=/path/sessions.db` | single SQLite file, one row per session |
| `HttpSessionStore` | `ZENE_SESSION_URL=http://host:port` | a Durable Object cell (Cloudflare or self-hosted [`celld`](https://github.com/denoland/celld)) |

## HTTP contract

- `GET /sessions/{id}` → `200` with the session JSON body, or `404`
- `PUT /sessions/{id}` → body is the session JSON; respond `2xx`

Failures surface as errors; there is no silent fallback to another backend.

## Reference worker (Cloudflare Durable Object / celld)

The same Workers bundle deploys as-is to Cloudflare (`wrangler deploy`) and to a
self-hosted celld fleet (`celld`, from the same `wrangler.jsonc`). Each session
cell is a Durable Object with SQLite storage.

```jsonc
// wrangler.jsonc
{
  "main": "worker.js",
  "durable_objects": {
    "bindings": [{ "name": "SESSION_CELL", "class_name": "SessionCell" }]
  },
  "migrations": [{ "tag": "v1", "new_classes": ["SessionCell"] }]
}
```

```js
// worker.js
const DDL = `CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  json TEXT NOT NULL
)`;

export class SessionCell {
  constructor(ctx) {
    this.sql = ctx.storage.sql;
    this.sql.exec(DDL);
  }

  async fetch(request) {
    const url = new URL(request.url);
    const id = decodeURIComponent(url.pathname.split("/").pop());
    if (!id) return new Response("missing id", { status: 400 });

    if (request.method === "PUT") {
      const json = await request.text();
      this.sql.exec(
        "INSERT INTO sessions (id, json) VALUES (?, ?) " +
          "ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        id,
        json,
      );
      return new Response(null, { status: 204 });
    }

    if (request.method === "GET") {
      for (const row of this.sql.exec("SELECT json FROM sessions WHERE id = ?", id)) {
        return new Response(row.json, {
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(null, { status: 404 });
    }

    return new Response(null, { status: 405 });
  }
}

export default {
  async fetch(request, env) {
    const stub = env.SESSION_CELL.get(env.SESSION_CELL.idFromName("sessions"));
    return stub.fetch(request);
  },
};
```

## SQLite schema (`SqliteSessionStore`)

```sql
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

Writes are upserts (`ON CONFLICT(id) DO UPDATE`); the `json` column holds the
serialized `SessionRecord` (the `messages` cache is omitted when the event log
rebuilds it, see the session crate docs).
