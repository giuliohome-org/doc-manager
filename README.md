# Doc Manager

A zero-knowledge document vault backed by Azure Blob Storage, with a built-in
**MCP server** so you can add, read and search your docs straight from a
Claude conversation.

<p align="center">
  <img alt="Claude on the left creates a document through the Doc Manager MCP connector; the web UI on the right shows it instantly in the vault" src="https://github.com/user-attachments/assets/5deab74c-b3a0-4b17-b101-c0f287e2eb68" />
</p>
<p align="center"><sub><i>Talk to your docs through Claude — create from chat, see it in the vault instantly.</i></sub></p>

- **Zero-knowledge by design** — documents and attachments are encrypted in the
  browser with AES-256-GCM (PBKDF2-SHA256, 600k iterations); the password never
  leaves the client.
- **Talk to your docs** — Claude (web, desktop, Claude Code) can list, fetch,
  search, create and update plaintext documents through the MCP endpoint.
- **One Rust binary** — Rocket backend + React 19 / Vite 8 frontend, deployable
  to Azure Container Apps, plain Docker, or `containerd` + Kaniko in a homelab.

---

## Talking to your docs through Claude

The same Rust process exposes a Streamable-HTTP [Model Context Protocol]
(https://modelcontextprotocol.io) server at `/mcp`, guarded by a bearer token
of your choosing. Encrypted documents stay private (their content is never
returned to Claude); plaintext documents become first-class citizens in the
chat.

### Tools

| Tool                | What it does                                                                    | Available in `MCP_READ_ONLY` |
| ------------------- | ------------------------------------------------------------------------------- | ---------------------------- |
| `list_documents`    | List every doc with `id`, `title`, `encrypted` flag, attachment info, size      | yes                          |
| `get_document`      | Fetch one plaintext document by id; refuses encrypted ones                      | yes                          |
| `search_documents`  | Case-insensitive substring search over titles (always) and plaintext content    | yes                          |
| `create_document`   | Create a new plaintext document; returns the new id                             | no                           |
| `update_document`   | Replace content (and optionally title) of a plaintext document                  | no                           |

Encrypted documents are surfaced to Claude with `encrypted: true` so it can
discover them by title, but their contents and writes are refused server-side.
That keeps the zero-knowledge guarantee intact: encrypted = "private to me",
plaintext = "shareable with Claude".

### Add it to Claude

1. Generate a strong random token and set it on your container app:
   ```sh
   export MCP_BEARER_TOKEN=$(openssl rand -hex 32)
   # optional — disables create/update tools:
   export MCP_READ_ONLY=true
   ```
2. **Claude.ai (web / desktop) custom connector** — Settings → Connectors →
   Add custom connector, URL:
   ```
   https://doc-manager.giuliohome.com/mcp/<MCP_BEARER_TOKEN>
   ```
   The token-in-URL form is for clients without a bearer-header field.
3. **Claude Code / MCP Inspector / curl** — header form:
   ```
   URL:    https://doc-manager.giuliohome.com/mcp
   Header: Authorization: Bearer <MCP_BEARER_TOKEN>
   ```

If `MCP_BEARER_TOKEN` is unset the endpoint replies `503 Service Unavailable`,
so the MCP surface is fully opt-in.

### Quick smoke test

```sh
curl -sS https://doc-manager.giuliohome.com/mcp \
  -H "Authorization: Bearer $MCP_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

---

## Building the frontend

```sh
cd frontend
npm i
npm run build
cd ..
```

## Running with Docker

```sh
docker build -t giuliohome/doc-manager:latest .
export AZURE_STORAGE_ACCOUNT=youraccount
export AZURE_STORAGE_ACCESS_KEY=yourkey
export RUST_ROCKET_EXACT_ORIGIN=http://localhost:8080
export MCP_BEARER_TOKEN=$(openssl rand -hex 32)   # optional, enables /mcp
docker run -p 8080:8080 \
  -e AZURE_STORAGE_ACCOUNT \
  -e AZURE_STORAGE_ACCESS_KEY \
  -e RUST_ROCKET_EXACT_ORIGIN \
  -e MCP_BEARER_TOKEN \
  giuliohome/doc-manager:latest
```

## TL;DR — `containerd` + Kaniko (homelab)

```
sudo mkdir /kcache
sudo ctr i pull gcr.io/kaniko-project/warmer:latest
sudo ctr run --net-host --rm --mount type=bind,src=$(pwd),dst=/workspace,options=rbind:rw --mount type=bind,src=/kcache,dst=/cache,options=rbind:rw gcr.io/kaniko-project/warmer:latest kaniko-warmer /kaniko/warmer --cache-dir=/cache --image=docker.io/rust:1-slim-bookworm --skip-tls-verify-registry index.docker.io --dockerfile=/workspace/Dockerfile

sudo ctr i pull gcr.io/kaniko-project/executor:latest
sudo ctr run --net-host --rm --mount type=bind,src=$(pwd),dst=/workspace,options=rbind:rw --mount type=bind,src=/kcache,dst=/cache,options=rbind:rw gcr.io/kaniko-project/executor:latest kaniko-executor /kaniko/executor -cache-dir=/cache --dockerfile=/workspace/Dockerfile --context=/workspace --no-push --skip-tls-verify --build-arg pkg=docs-app --tarPath=/workspace/doc-manager-latest.tar --destination=giuliohome/doc-manager:latest --cache=true --cache-repo=giuliohome/doc-manager:latest --no-push-cache

sudo ctr image import doc-manager-latest.tar
sudo ctr c create --net-host docker.io/giuliohome/doc-manager:latest doc-manager
sudo ctr t start doc-manager
```

<img width="1906" alt="image" src="https://github.com/user-attachments/assets/e8c7cecb-adac-4f5f-9f94-143b0e867e3d" />

<img width="1897" alt="image" src="https://github.com/user-attachments/assets/64cfae0f-47fa-40b5-a743-b0f02e160b78" />

## Configuration reference

| Env var                       | Required           | Purpose                                                            |
| ----------------------------- | ------------------ | ------------------------------------------------------------------ |
| `AZURE_STORAGE_ACCOUNT`       | yes                | Azure Storage account name (container `documents` is auto-created) |
| `AZURE_STORAGE_ACCESS_KEY`    | yes                | Storage access key                                                 |
| `RUST_ROCKET_EXACT_ORIGIN`    | yes                | CORS origin for the React frontend                                 |
| `MCP_BEARER_TOKEN`            | no (enables MCP)   | Token required by the `/mcp` endpoint                              |
| `MCP_READ_ONLY`               | no (default false) | When `true`/`1`/`yes`, hides create/update MCP tools               |

## End-to-end tests

See the [e2e repo](https://github.com/giuliohome-org/e2e-doc-manager).
