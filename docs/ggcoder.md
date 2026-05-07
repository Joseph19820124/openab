# GG Coder CLI

GG Coder does not currently expose ACP directly. OpenAB's GG Coder image installs
`ggcoder-acp`, a small wrapper that speaks ACP on stdio and translates requests
to GG Coder's `--rpc` JSON-RPC mode.

## Docker Image

```bash
docker build -f Dockerfile.ggcoder -t openab-ggcoder:latest .
```

The image installs `@kenkaiiii/ggcoder` globally via npm and copies the
`ggcoder-acp` wrapper into `/usr/local/bin`.

## Helm Install

```bash
helm install openab openab/openab \
  --set agents.kiro.enabled=false \
  --set agents.ggcoder.discord.enabled=true \
  --set agents.ggcoder.discord.botToken="$DISCORD_BOT_TOKEN" \
  --set-string 'agents.ggcoder.discord.allowedChannels[0]=YOUR_CHANNEL_ID' \
  --set agents.ggcoder.image=ghcr.io/joseph19820124/openab-ggcoder:latest \
  --set agents.ggcoder.command=ggcoder-acp \
  --set agents.ggcoder.workingDir=/home/node
```

> Set `agents.kiro.enabled=false` to disable the default Kiro agent.
>
> (Optional) Use `--set agents.ggcoder.args='{--provider,openai}'` to pass
> GG Coder CLI options through the ACP wrapper.

## Manual config.toml

```toml
[agent]
command = "ggcoder-acp"
args = []
working_dir = "/home/node"
```

## Authentication

GG Coder supports Anthropic and OpenAI OAuth:

- **Anthropic**: Run `ggcoder login` inside the pod and follow the OAuth flow
- **OpenAI**: Run `ggcoder login --provider openai` inside the pod
- **API key via env**: Set `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`

```bash
# Interactive login (OAuth)
kubectl exec -it deployment/<release>-ggcoder -- ggcoder login

# Or set API key via Helm
helm upgrade <release> openab/openab \
  --set agents.ggcoder.env.ANTHROPIC_API_KEY="<key>"
```

After authenticating, restart the deployment:

```bash
kubectl rollout restart deployment/<release>-ggcoder
```
