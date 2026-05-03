# Qwen Code

Qwen Code supports ACP natively via the `--acp` flag — no adapter needed.

Qwen Code is Alibaba's open-source coding CLI built on Qwen3-Coder. It supports both Qwen OAuth (free tier) and API key authentication.

```
┌──────────┐  Discord  ┌────────┐ ACP stdio ┌────────────┐   ┌───────────────────┐
│ Discord  │◄────────► │ OpenAB │◄────────► │ Qwen Code  │──►│  Qwen3-Coder      │
│ Users    │ Gateway   │ (Rust) │ JSON-RPC  │  (ACP)     │   │                   │
└──────────┘           └────────┘           └────────────┘   │ ┌───────────────┐ │
                                                  │           │ │ Qwen OAuth    │ │
                                             --yolo flag      │ │ (free tier)   │ │
                                             auto-approves    │ │               │ │
                                             all tool calls   │ │ QWEN_API_KEY  │ │
                                                              │ │ (API key)     │ │
                                                              │ └───────────────┘ │
                                                              └───────────────────┘
```

## Docker Image

```bash
docker build -f Dockerfile.qwen -t openab-qwen:latest .
```

The image installs `@qwen-code/qwen-code` globally via npm on `node:22-bookworm-slim`.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.qwen.enabled=true \
  --set agents.qwen.command=qwen \
  --set 'agents.qwen.args={--acp,--yolo}' \
  --set agents.qwen.image=ghcr.io/openabdev/openab-qwen:latest \
  --set agents.qwen.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.qwen.discord.allowedChannels[0]=YOUR_CHANNEL_ID' \
  --set agents.qwen.workingDir=/home/node \
  --set agents.qwen.pool.maxSessions=3
```

> Set `agents.kiro.enabled=false` to disable the default Kiro agent.

## Manual config.toml

```toml
[agent]
command = "qwen"
args = ["--acp", "--yolo"]
working_dir = "/home/node"
```

> **Note:** `--yolo` auto-approves all tool calls without confirmation, equivalent to `--trust-all-tools` on other backends. Remove it to use the default approval mode.

## Authentication

Qwen Code supports two authentication methods:

### Option 1: Qwen OAuth (recommended — free tier available)

```bash
kubectl exec -it deployment/openab-qwen -- qwen auth
```

Follow the browser OAuth flow, then restart the pod:

```bash
kubectl rollout restart deployment/openab-qwen
```

### Option 2: API Key

Set `QWEN_API_KEY` via Helm:

```bash
helm upgrade openab openab/openab --reuse-values \
  --set 'agents.qwen.env.QWEN_API_KEY=YOUR_API_KEY'
```

Or in `config.toml`:

```toml
[agent]
command = "qwen"
args = ["--acp", "--yolo"]
working_dir = "/home/node"
env = { QWEN_API_KEY = "${QWEN_API_KEY}" }
```

### Persisted Paths (PVC)

| Path | Contents |
|------|----------|
| `~/.qwen/` | Settings, auth tokens, session history |

## Notes

- **Tool authorization**: `--yolo` auto-approves all tool calls. This is equivalent to `--trust-all-tools` on other backends. Remove it to use the default approval mode (prompts for each tool call).
- **Model**: Qwen Code uses Qwen3-Coder by default. The model can be changed via `qwen auth` or the `-m` flag.
- **Version pinning**: The pinned version in `Dockerfile.qwen` should be bumped via a dedicated PR when an update is needed.
