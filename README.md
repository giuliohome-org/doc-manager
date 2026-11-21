# Doc Manager

<p align="center">
  <a href="https://glama.ai/mcp/servers/giuliohome-org/doc-manager">
    <img alt="doc-manager MCP server on Glama" src="https://glama.ai/mcp/servers/giuliohome-org/doc-manager/badges/card.svg" />
  </a>
</p>

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

The same Rust process exposes a Streamable HTTP [Model Context Protocol](https://modelcontextprotocol.io)
server at `/mcp`. Users sign in with GitHub. Doc Manager issues a 15-minute
access token bound to the exact MCP URL, the registered connector and the user's
numeric GitHub ID. The endpoint verifies this binding on every HTTP request.
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
| `delete_document`   | Delete a document and all its blobs (main, title, attachment); irreversible     | no                           |
| `delete_attachment` | Delete only the attachment of a document, leaving the document itself intact    | no                           |

Encrypted documents are surfaced to Claude with `encrypted: true` so it can
discover them by title, but their contents and writes are refused server-side.
That keeps the zero-knowledge guarantee intact: encrypted = "private to me",
plaintext = "shareable with Claude".

### Connect ChatGPT or Claude

1. Open the existing GitHub OAuth App for the connector. Add the exact
   `<origin>/mcp/oauth/callback` redirect URI with wildcard matching disabled.
   GitHub returns the sign-in result to Doc Manager at this address.
2. Set `OAUTH_CONNECTOR_REDIRECT_URI` to the exact callback displayed by the
   connector. Set `OAUTH_GITHUB_CLIENT_ID` to the public client ID of the
   existing OAuth App. The connector keeps its client ID and secret. During
   token exchange Doc Manager submits those credentials to GitHub over HTTPS.
3. Set `OAUTH_GITHUB_USER_IDS` to the numeric IDs of the GitHub accounts that
   may open the vault. Generate `OAUTH_TOKEN_KEY` from 32 cryptographically
   random bytes encoded in standard base64. Supply it to every replica through
   an Azure Container Apps secret reference. Rotating it invalidates sessions.
4. Connect to `<origin>/mcp` and sign in with GitHub. When upgrading an existing
   connector, reconnect once to obtain a Doc Manager token. Clients request
   `mcp.access` and can request `offline_access` for renewal.
5. If Azure Easy Auth protects the website, pass `/mcp`, `/mcp/*`,
   `/.well-known/oauth-protected-resource` and `/.well-known/oauth-authorization-server`
   through to Rocket.

A push to `main` triggers the Azure deployment workflow. Configure the secret
reference and OAuth environment variables on the candidate revision before
moving traffic to it.

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
export OAUTH_GITHUB_CLIENT_ID='your-github-oauth-client-id'
export OAUTH_CONNECTOR_REDIRECT_URI='https://your-connector.example/oauth/callback'
export OAUTH_GITHUB_USER_IDS='123456'
export OAUTH_TOKEN_KEY="$(openssl rand -base64 32)"
docker run -p 8080:8080 \
  -e AZURE_STORAGE_ACCOUNT \
  -e AZURE_STORAGE_ACCESS_KEY \
  -e RUST_ROCKET_EXACT_ORIGIN \
  -e OAUTH_PROVIDER \
  -e OAUTH_PUBLIC_BASE_URL \
  -e OAUTH_GITHUB_CLIENT_ID \
  -e OAUTH_CONNECTOR_REDIRECT_URI \
  -e OAUTH_GITHUB_USER_IDS \
  -e OAUTH_TOKEN_KEY \
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
| `OAUTH_PROVIDER`              | no (enables MCP)        | `github` for GitHub login and Doc Manager access tokens |
| `OAUTH_PUBLIC_BASE_URL`       | with `OAUTH_PROVIDER`   | Public base URL of this server, e.g. `https://doc-manager.giuliohome.com`                        |
| `OAUTH_GITHUB_CLIENT_ID`      | with `OAUTH_PROVIDER`   | Public client ID of the connector's GitHub OAuth App |
| `OAUTH_CONNECTOR_REDIRECT_URI` | with `OAUTH_PROVIDER`  | Exact callback URL for the connector |
| `OAUTH_GITHUB_USER_IDS`       | with `OAUTH_PROVIDER`   | Comma-separated list of allowed numeric GitHub user IDs |
| `OAUTH_TOKEN_KEY`             | with `OAUTH_PROVIDER`   | Standard base64 encoding of a shared 32-byte random encryption key |
| `MCP_READ_ONLY`               | no (default false)      | When `true`/`1`/`yes`, hides create/update MCP tools                                             |
| `MCP_PUBLIC_INTROSPECT`       | no (auto)               | Opens `/mcp` for unauthenticated `tools/list` only (calls are 403'd) so registries like Glama can probe. Auto-on when both `AZURE_STORAGE_ACCOUNT` and `OAUTH_PROVIDER` are unset; set `0` to force-disable. |
| `MCP_STDIO`                   | no (default false)      | When `true`/`1`/`yes` (or pass `--stdio`), runs as a stdio MCP server (newline-delimited JSON-RPC on stdin/stdout) instead of starting the HTTP listener. Used by registries that wrap servers with `mcp-proxy`. |

## End-to-end tests

See the [e2e repo](https://github.com/giuliohome-org/e2e-doc-manager).
