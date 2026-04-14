# Gemini CLI

Gemini CLI supports ACP natively via the `--acp` flag — no adapter needed.

## Docker Image

```bash
docker build -f Dockerfile.gemini -t openab-gemini:latest .
```

The image installs `@google/gemini-cli` globally via npm.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.gemini.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.gemini.discord.allowedChannels[0]=YOUR_CHANNEL_ID' \
  --set agents.gemini.image=ghcr.io/openabdev/openab-gemini:latest \
  --set agents.gemini.command=gemini \
  --set agents.gemini.args='{--acp}' \
  --set agents.gemini.workingDir=/home/node
```

> Set `agents.kiro.enabled=false` to disable the default Kiro agent.

## First-Run Workaround

Gemini CLI needs `~/.gemini/projects.json` to exist before the first ACP session starts. In ephemeral containers, the file may not be created yet, which can surface as `ENOENT` or `Unexpected end of JSON input` during the first run.

This is a workaround for the upstream Gemini CLI issue `google-gemini/gemini-cli#22583`.

If you control the container startup command, seed the directory before launching `openab`:

```bash
mkdir -p /home/node/.gemini && echo '{}' > /home/node/.gemini/projects.json && exec openab /etc/openab/config.toml
```

The `exec openab /etc/openab/config.toml` part assumes the Docker image default config path.

For Kubernetes or Helm deployments, use the same idea with an init container and a shared volume mounted at `/home/node/.gemini`. A minimal pattern looks like this:

```yaml
extraInitContainers:
  - name: seed-gemini-home
    image: busybox:1.36
    command: ["sh", "-c", "mkdir -p /home/node/.gemini && echo '{}' > /home/node/.gemini/projects.json"]
    volumeMounts:
      - name: gemini-home
        mountPath: /home/node/.gemini
```

The current chart does not expose `extraInitContainers`, `extraVolumes`, or `extraVolumeMounts`, so the workaround is easiest to apply in a custom manifest or wrapper image until those values are added. Chart support for those passthrough fields is tracked in #344.

## Manual config.toml

```toml
[agent]
command = "gemini"
args = ["--acp"]
working_dir = "/home/node"
env = { GEMINI_API_KEY = "${GEMINI_API_KEY}" }
```

## Authentication

Gemini supports Google OAuth or an API key:

- **API key**: Set `GEMINI_API_KEY` environment variable
- **OAuth**: Run Google OAuth flow inside the pod
