# Antigravity (Experimental)

> **Status:** EXPERIMENTAL. Antigravity is Google's IDE product (released as part of Google I/O 2026). There is **no official `@google/antigravity-cli` package** at the time of this writing — this image is a community-built bridge that re-uses `@google/gemini-cli`'s ACP implementation against Antigravity's backend until Google ships an official CLI.

## What this gives you

- Access to **Gemini 3.5 Flash / Gemini 3 Pro** plus **Claude 4.6** (and other Antigravity-hosted models) through your Google account's Antigravity subscription
- **Native ACP support** via `gemini --acp` — no protocol changes in OpenAB
- Same Discord / Slack / Custom Gateway integration as every other OpenAB agent

## How it works

Antigravity reuses the same wire protocol as Gemini CLI:

```text
POST https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse
Authorization: Bearer <Antigravity OAuth token>
```

…on a different host (`daily-cloudcode-pa.googleapis.com` instead of `cloudcode-pa.googleapis.com`) with a different OAuth client.

`Dockerfile.antigravity` installs `@google/gemini-cli` (which already speaks ACP) and rewrites three constants in its bundle at container startup:

1. `OAUTH_CLIENT_ID` → Antigravity OAuth client
2. `OAUTH_CLIENT_SECRET` → Antigravity OAuth secret
3. `CODE_ASSIST_ENDPOINT` → `https://daily-cloudcode-pa.googleapis.com`

After patching, the standard `gemini --acp` entrypoint just works — OpenAB doesn't see any difference vs `Dockerfile.gemini`.

**No literal Antigravity secrets are shipped in the image.** All three values are supplied at container start via environment variables.

## Obtaining Antigravity OAuth credentials

You have two options:

### Option A — Create your own Google OAuth client (recommended for production)

1. Open the [Google Cloud Console → APIs & Services → Credentials](https://console.cloud.google.com/apis/credentials).
2. Create credentials → **OAuth client ID** → Application type **Desktop app**.
3. Enable the **Cloud Code (cloudcode-pa.googleapis.com)** API on the same project.
4. Copy the client ID and client secret into the env vars below.

### Option B — Reuse the published Antigravity desktop app credentials

The Antigravity desktop application uses a published OAuth client. The values can be found in community projects that integrate with Antigravity, e.g. [`su-kaka/gcli2api`](https://github.com/su-kaka/gcli2api/blob/master/src/utils.py). Google's policy ([OAuth installed-app guidelines](https://developers.google.com/identity/protocols/oauth2#installed)) explicitly states client_secret for installed apps is not treated as a secret.

⚠️ Using these shared values means your traffic shares quota / fingerprint with all other community users of the same OAuth client. For low-volume personal use it's usually fine; for any deployment serving multiple users, prefer Option A.

## Docker Image

```bash
docker build -f Dockerfile.antigravity -t openab-antigravity:latest .
```

The image installs `@google/gemini-cli@0.42.0` by default. Override with `--build-arg GEMINI_CLI_VERSION=...`.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.antigravity.discord.enabled=true \
  --set agents.antigravity.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.antigravity.discord.allowedChannels[0]=YOUR_CHANNEL_ID' \
  --set agents.antigravity.image=ghcr.io/openabdev/openab-antigravity:latest \
  --set agents.antigravity.command=gemini \
  --set agents.antigravity.args='{--acp}' \
  --set agents.antigravity.workingDir=/home/node \
  --set agents.antigravity.env.ANTIGRAVITY_OAUTH_CLIENT_ID="$ANTIGRAVITY_OAUTH_CLIENT_ID" \
  --set agents.antigravity.env.ANTIGRAVITY_OAUTH_CLIENT_SECRET="$ANTIGRAVITY_OAUTH_CLIENT_SECRET"
```

> Set `agents.kiro.enabled=false` to disable the default Kiro agent.
>
> ⚠️ This integration reuses the `agents.gemini.*` Helm config schema for now since `gemini --acp` is the actual binary being invoked. A dedicated `agents.antigravity` schema can be added once the integration stabilizes.

## Manual config.toml

```toml
[agent]
command = "gemini"
args = ["--acp"]
working_dir = "/home/node"

[agent.env]
ANTIGRAVITY_OAUTH_CLIENT_ID = "${ANTIGRAVITY_OAUTH_CLIENT_ID}"
ANTIGRAVITY_OAUTH_CLIENT_SECRET = "${ANTIGRAVITY_OAUTH_CLIENT_SECRET}"
# Optional override (default: https://daily-cloudcode-pa.googleapis.com)
# ANTIGRAVITY_API_URL = "${ANTIGRAVITY_API_URL}"
```

## Authentication flow

On first launch, `gemini` (now repointed at Antigravity) opens a browser-based OAuth flow with Antigravity's OAuth client. After consent, tokens are persisted to `/home/node/.gemini/` and refreshed automatically. The patched constants flow through unchanged:

- gemini-cli emits requests to `daily-cloudcode-pa.googleapis.com`
- Refresh tokens are bound to Antigravity's OAuth client (so refresh works correctly)
- Account ID (extracted from `id_token`) routes the request to the user's Antigravity account

## Limitations and risks

| Item | Notes |
|------|------|
| **gemini-cli bundle layout** | The runtime patch grepps gemini-cli's chunked JS bundle for the OAuth constants. If a future gemini-cli release changes its bundle naming or constant patterns, the patch will fail loudly at container start (clear error message, no silent corruption). |
| **Antigravity API contract** | `daily-cloudcode-pa.googleapis.com /v1internal:streamGenerateContent` is an internal API. Wire format may change without notice. |
| **Quota / fingerprint** | Using Option B (shared OAuth client) means your traffic is indistinguishable from other community users of that client; Antigravity may rate-limit aggressively. |
| **No SLA** | This integration is unsupported by Google. For commercial workloads, use Vertex AI with a service account. |

## Future replacement

Once Google ships an official `@google/antigravity-cli` package (with `--acp` support), this image will become a thin wrapper around it (mirroring `Dockerfile.gemini` exactly) and the runtime patch script will be removed.

## See also

- [`Dockerfile.gemini`](../Dockerfile.gemini) — the standard Gemini CLI integration this image is based on.
- [`docs/gemini.md`](gemini.md) — Gemini CLI integration docs.
- [`@google/gemini-cli` source](https://github.com/google-gemini/gemini-cli) — upstream CLI.
- [`google-gemini/gemini-cli/.../oauth2.ts`](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth2.ts) — published OAuth flow.
