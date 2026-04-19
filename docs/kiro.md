# Kiro CLI (Default Agent)

Kiro CLI is the default agent backend for OpenAB. It supports ACP natively — no adapter needed.

## Docker Image

The default `Dockerfile` bundles both `openab` and `kiro-cli`:

```bash
docker build -t openab:latest .
```

## Helm Install

```bash
helm repo add openab https://openabdev.github.io/openab
helm repo update

helm install openab openab/openab \
  --set agents.kiro.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.kiro.discord.allowedChannels[0]=YOUR_CHANNEL_ID'
```

## Manual config.toml

```toml
[agent]
command = "kiro-cli"
args = ["acp", "--trust-all-tools"]
working_dir = "/home/agent"
```

## Authentication

Kiro CLI requires a one-time OAuth login. The PVC persists tokens across pod restarts.

```bash
kubectl exec -it deployment/openab-kiro -- kiro-cli login --use-device-flow
```

Follow the device code flow in your browser, then restart the pod:

```bash
kubectl rollout restart deployment/openab-kiro
```

### Persisted Paths (PVC)

| Path | Contents |
|------|----------|
| `~/.kiro/` | Settings, skills, sessions |
| `~/.local/share/kiro-cli/` | OAuth tokens (`data.sqlite3` → `auth_kv` table), conversation history |

## Slash Commands

### `/models` — Switch AI Model

When using Kiro CLI as the backend, the `/models` slash command lets users dynamically switch models via a Discord select menu.

**How it works:**
1. Kiro CLI returns available models via ACP `configOptions` (category: `"model"`) on session creation
2. User types `/models` in a thread with an active session
3. A select menu appears with available models (e.g. Sonnet 4, Opus 4, Haiku 4)
4. User picks a model → OpenAB sends `session/set_config_option` to Kiro
5. Model switches immediately for that session

**Note:** The `/models` command only works in threads where a conversation is already active. If no session exists, it will prompt the user to start one first.

> ⚠️ This feature has only been tested with Kiro CLI. Other ACP backends (Claude Code, Codex, Gemini) may or may not return `configOptions` with model choices — behavior will vary by agent.

### Future Commands

| Command | Purpose | Status |
|---------|---------|--------|
| `/models` | Switch AI model | ✅ Implemented |
| `/agents` | Switch agent backend | 🔜 Planned |
| `/cancel` | Cancel current generation | 🔜 Planned |
