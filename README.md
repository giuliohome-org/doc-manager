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
(https://modelcontextprotocol.io) server at `/mcp`, gated by an OAuth 2.1
flow against an upstream identity provider (currently **GitHub** —
Auth0 reserved for a future round, the code is provider-shaped).
Encrypted documents stay private (their content is never returned to
Claude); plaintext documents become first-class citizens in the chat.

### Tools

| Tool                | What it does                                                                    | Available in `MCP_READ_ONLY` |
| ------------------- | ------------------------------------------------------------------------------- | ---------------------------- |
| `list_documents`    | List every doc with `id`, `title`, `encrypted` flag, attachment info, size      | yes                          |
| `get_document`      | Fetch one plaintext document by id; refuses encrypted ones                      | yes                          |
| `search_documents`  | Case-insensitive substring search over titles (always) and plaintext content    | yes                          |
| `get_attachment`    | Fetch the file attached to a doc (filename, bytes, base64, UTF-8 if applicable) | yes                          |
| `create_document`   | Create a new plaintext document; returns the new id                             | no                           |
| `update_document`   | Replace content (and optionally title) of a plaintext document                  | no                           |
| `add_attachment`    | Attach a file (text or base64-encoded binary) to a doc; replaces any existing   | no                           |

Encrypted documents are surfaced to Claude with `encrypted: true` so it can
discover them by title, but their contents and writes are refused server-side.
That keeps the zero-knowledge guarantee intact: encrypted = "private to me",
plaintext = "shareable with Claude".

### Add it to Claude

Claude.ai's custom-connector advanced settings accept an OAuth
**Client ID + Client Secret** ([only those two
fields](https://github.com/anthropics/claude-ai-mcp/issues/112)),
so Claude.ai runs the full OAuth 2.1 + PKCE flow itself and our server
only needs to (a) advertise the IdP via well-known metadata and (b)
validate the resulting bearer.

1. **Register a GitHub OAuth App.** GitHub → *Settings* → *Developer
   settings* → *OAuth Apps* → *New OAuth App*. Set:
   - *Authorization callback URL*: `https://claude.ai/api/mcp/auth_callback`
   - Copy the *Client ID* and generate a *Client Secret*.
2. **Configure the server** (env vars on the container app):
   ```sh
   export OAUTH_PROVIDER=github
   export OAUTH_PUBLIC_BASE_URL=https://doc-manager.giuliohome.com
   export OAUTH_ALLOWED_USERS=giuliohome     # comma-separated GitHub logins
   # optional — disables create/update tools:
   export MCP_READ_ONLY=true
   ```
   `OAUTH_ALLOWED_USERS` is **required** — an empty allowlist refuses to
   start, so a misconfiguration cannot silently let any GitHub user in.
3. **Add the connector in Claude.ai** — Settings → Connectors → Add
   custom connector, URL `https://doc-manager.giuliohome.com/mcp`, then
   *Advanced settings* → paste the GitHub OAuth App's Client ID and
   Client Secret. The first request triggers the OAuth dance: Claude.ai
   sees the `401` + `WWW-Authenticate` from `/mcp`, fetches our
   well-known metadata, walks you through `github.com/login/oauth/authorize`,
   and forwards the resulting bearer on every subsequent call. The server
   validates each request by calling `https://api.github.com/user` (5-min
   cache, SHA-256-hashed token key).

If `OAUTH_PROVIDER` is unset the endpoint replies `503 Service Unavailable`,
so the MCP surface is fully opt-in.

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
# optional — enables the MCP endpoint:
export OAUTH_PROVIDER=github
export OAUTH_PUBLIC_BASE_URL=http://localhost:8080
export OAUTH_ALLOWED_USERS=your-github-login
docker run -p 8080:8080 \
  -e AZURE_STORAGE_ACCOUNT \
  -e AZURE_STORAGE_ACCESS_KEY \
  -e RUST_ROCKET_EXACT_ORIGIN \
  -e OAUTH_PROVIDER \
  -e OAUTH_PUBLIC_BASE_URL \
  -e OAUTH_ALLOWED_USERS \
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

| Env var                       | Required                | Purpose                                                                                          |
| ----------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------ |
| `AZURE_STORAGE_ACCOUNT`       | yes                     | Azure Storage account name (container `documents` is auto-created)                               |
| `AZURE_STORAGE_ACCESS_KEY`    | yes                     | Storage access key                                                                               |
| `RUST_ROCKET_EXACT_ORIGIN`    | yes                     | CORS origin for the React frontend                                                               |
| `OAUTH_PROVIDER`              | no (enables MCP)        | `github` (Auth0 reserved for future)                                                             |
| `OAUTH_PUBLIC_BASE_URL`       | with `OAUTH_PROVIDER`   | Public base URL of this server, e.g. `https://doc-manager.giuliohome.com`                        |
| `OAUTH_ALLOWED_USERS`         | with `OAUTH_PROVIDER`   | Comma-separated GitHub logins; **must list at least one user** — empty refuses to start          |
| `MCP_READ_ONLY`               | no (default false)      | When `true`/`1`/`yes`, hides create/update MCP tools                                             |

## End-to-end tests

See the [e2e repo](https://github.com/giuliohome-org/e2e-doc-manager).
