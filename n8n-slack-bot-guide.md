# n8n Slack Coding Agent Bot — 配置指南

## 架构
```
Slack @mention → n8n Webhook → Claude API (tool use) → Slack 线程回复
```

## 第一步：创建 Slack App

1. 打开 https://api.slack.com/apps → **Create New App** → **From scratch**
2. 填名字（如 `Coding Agent`），选你的 workspace

### Bot Token Scopes
进入 **OAuth & Permissions** → **Scopes** → **Bot Token Scopes**，添加：
- `app_mentions:read` — 读取 @mention
- `chat:write` — 发送消息
- `channels:history` — 读取 channel 历史（获取线程上下文）
- `groups:history` — 读取 private channel 历史

### Event Subscriptions
进入 **Event Subscriptions** → 开启 **Enable Events**：
- **Request URL**: 先空着，等 n8n workflow 激活后填
- **Subscribe to bot events** → 添加 `app_mention`

### 安装 App
进入 **Install App** → **Install to Workspace** → 授权
- 记下 **Bot User OAuth Token**（`xoxb-...`）

---

## 第二步：导入 n8n Workflow

1. 打开你的 n8n：https://josephchen.app.n8n.cloud
2. 点 **+ Add Workflow** → 右上角 **⋯** → **Import from File**
3. 选择 `n8n-slack-bot-workflow.json`

---

## 第三步：配置 Credentials

### Slack Bot Token
在 n8n 中需要手动替换几个地方的凭证：

**方法：在 n8n 中创建 Header Auth credential：**
1. Settings → Credentials → + Add → **Header Auth**
2. Name: `Slack Bot Token`
3. Header Name: `Authorization`
4. Header Value: `Bearer xoxb-你的token`

然后更新以下节点使用这个 credential：
- `Get Thread History`
- `Slack Reply`

### Anthropic API Key
1. Settings → Credentials → + Add → **Header Auth**
2. Name: `Anthropic API Key`
3. Header Name: `x-api-key`
4. Header Value: `sk-ant-你的key`

然后更新以下节点：
- `Claude API`
- `Claude Follow-up`

> **注意**：由于 n8n HTTP Request 节点直接发请求，你需要在每个 HTTP 节点中手动设置 header，而不是用 `$credentials` 变量。导入后检查每个 HTTP 节点的 header 配置。

---

## 第四步：激活 Webhook

1. 在 n8n 中打开 workflow → 点击 **Slack Webhook** 节点
2. 复制 **Production URL**（类似 `https://josephchen.app.n8n.cloud/webhook/slack-bot`）
3. 回到 Slack App 设置 → **Event Subscriptions**
4. 将 URL 粘贴到 **Request URL**
5. Slack 会发送一个 challenge 验证 — workflow 会自动回复
6. 看到 ✅ **Verified** 就成功了

---

## 第五步：激活 Workflow

1. 在 n8n 中打开 workflow
2. 右上角切换为 **Active** ✅
3. 去 Slack 的任意 channel，@mention 你的 bot 试试：
   ```
   @Coding Agent 帮我写一个 Python 的快速排序
   ```

---

## 工作流程说明

```
1. 用户在 Slack @mention bot
2. Slack 发 event 到 n8n webhook
3. n8n 检查：是 challenge? → 回复 challenge
4. n8n 检查：是 bot 自己的消息? → 忽略（防循环）
5. 获取线程历史 → 构建 Claude 对话上下文
6. 调 Claude API（带 tool use 定义）
7. 如果 Claude 要用 tool → 执行 tool → 把结果喂回 Claude
8. 格式化最终回复 → 发到 Slack 线程
```

## 支持的 Tools
- **execute_javascript**: 执行 JS 代码片段（计算、数据转换等）
- **search_web**: 预留接口（可接入 Brave/Google 搜索 API）

## 后续优化建议
- 添加 `:thinking_face:` emoji reaction 表示正在处理
- 接入更多 tools（GitHub API、数据库查询等）
- 添加用户白名单控制
- 支持图片/文件上传分析
- 添加 error handling + retry 逻辑
