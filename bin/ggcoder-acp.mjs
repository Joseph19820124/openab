#!/usr/bin/env node
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const passthroughArgs = process.argv.slice(2).filter((arg) => arg !== "--rpc");
let sessionId = null;
let rpc = null;
let nextRpcId = 1;
const pending = new Map();
const toolNames = new Map();

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function result(id, value) {
  send({ jsonrpc: "2.0", id, result: value });
}

function error(id, code, message) {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}

function notify(update) {
  if (!sessionId) return;
  send({
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId, update },
  });
}

function flattenPrompt(prompt) {
  if (!Array.isArray(prompt)) return "";
  return prompt
    .map((block) => {
      if (block?.type === "text") return block.text ?? "";
      if (block?.type === "image") return "[Image attachment omitted: GG Coder RPC accepts text only.]";
      return `[Unsupported content block: ${JSON.stringify(block)}]`;
    })
    .filter(Boolean)
    .join("\n\n");
}

function toolTitle(name, args) {
  if (!args || Object.keys(args).length === 0) return name;
  const detail = args.command ?? args.file_path ?? args.path ?? args.pattern ?? "";
  return detail ? `${name}: ${String(detail).slice(0, 120)}` : name;
}

function handleRpcEvent(event) {
  if (event.type === "ready") {
    rpc.ready = true;
    for (const resolve of rpc.readyWaiters) resolve();
    rpc.readyWaiters = [];
    return;
  }

  if (event.type === "result" || event.type === "error") {
    const acpId = pending.get(event.id);
    if (acpId === undefined) return;
    pending.delete(event.id);
    if (event.type === "error") {
      error(acpId, -32000, event.message ?? "GG Coder RPC error");
    } else {
      result(acpId, event.data ?? {});
    }
    return;
  }

  if (event.type === "text_delta" && event.text) {
    notify({
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: event.text },
    });
    return;
  }

  if (event.type === "thinking_delta") {
    notify({
      sessionUpdate: "agent_thought_chunk",
      content: { type: "text", text: event.text ?? "" },
    });
    return;
  }

  if (event.type === "tool_call_start") {
    toolNames.set(event.toolCallId, event.name);
    notify({
      sessionUpdate: "tool_call",
      toolCallId: event.toolCallId,
      title: toolTitle(event.name, event.args),
    });
    return;
  }

  if (event.type === "tool_call_update") {
    notify({
      sessionUpdate: "tool_call_update",
      toolCallId: event.toolCallId,
      title: toolNames.get(event.toolCallId) ?? "Tool",
      status: "running",
    });
    return;
  }

  if (event.type === "tool_call_end") {
    notify({
      sessionUpdate: "tool_call_update",
      toolCallId: event.toolCallId,
      title: toolNames.get(event.toolCallId) ?? "Tool",
      status: event.isError ? "failed" : "completed",
    });
    toolNames.delete(event.toolCallId);
    return;
  }

  if (event.type === "error" && event.message) {
    notify({
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: `GG Coder error: ${event.message}` },
    });
  }
}

function startRpc(cwd) {
  if (rpc) return rpc;

  const child = spawn("ggcoder", [...passthroughArgs, "--rpc"], {
    cwd,
    env: process.env,
    stdio: ["pipe", "pipe", "inherit"],
  });

  rpc = {
    child,
    ready: false,
    exitError: null,
    readyWaiters: [],
  };

  child.on("error", (err) => {
    if (rpc) rpc.exitError = err;
    const message = `failed to start GG Coder RPC: ${err.message}`;
    for (const acpId of pending.values()) error(acpId, -32000, message);
    pending.clear();
    for (const resolve of rpc?.readyWaiters ?? []) resolve();
  });

  child.on("exit", (code, signal) => {
    const message = `GG Coder RPC exited${signal ? ` by ${signal}` : ` with code ${code}`}`;
    if (rpc) rpc.exitError = new Error(message);
    for (const acpId of pending.values()) error(acpId, -32000, message);
    pending.clear();
    if (!rpc.ready) {
      for (const resolve of rpc.readyWaiters) resolve();
      rpc.readyWaiters = [];
    }
    rpc = null;
  });

  const lines = createInterface({ input: child.stdout, terminal: false });
  lines.on("line", (line) => {
    if (!line.trim()) return;
    try {
      handleRpcEvent(JSON.parse(line));
    } catch (err) {
      process.stderr.write(`ggcoder-acp: ignored non-JSON RPC output: ${line}\n`);
    }
  });

  return rpc;
}

function waitReady(state, timeoutMs = 120_000) {
  if (state.ready) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("timeout waiting for GG Coder RPC ready")), timeoutMs);
    state.readyWaiters.push(() => {
      clearTimeout(timeout);
      if (state.ready) {
        resolve();
      } else {
        reject(state.exitError ?? new Error("GG Coder RPC exited before ready"));
      }
    });
  });
}

function sendRpc(command) {
  if (!rpc?.child?.stdin?.writable) {
    throw new Error("GG Coder RPC is not running");
  }
  const id = nextRpcId++;
  rpc.child.stdin.write(`${JSON.stringify({ id, ...command })}\n`);
  return id;
}

function waitForPending(timeoutMs = 5_000) {
  if (pending.size === 0) return Promise.resolve();
  return new Promise((resolve) => {
    const started = Date.now();
    const interval = setInterval(() => {
      if (pending.size === 0 || Date.now() - started >= timeoutMs) {
        clearInterval(interval);
        resolve();
      }
    }, 25);
  });
}

async function handleAcp(message) {
  const { id, method, params } = message;

  if (method === "initialize") {
    result(id, {
      protocolVersion: 1,
      agentInfo: { name: "ggcoder", version: "ggcoder-acp" },
      agentCapabilities: { loadSession: false },
    });
    return;
  }

  if (method === "session/new") {
    sessionId = `ggcoder-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    const cwd = params?.cwd ?? process.cwd();
    const state = startRpc(cwd);
    await waitReady(state);
    result(id, { sessionId });
    return;
  }

  if (method === "session/prompt") {
    if (!sessionId) throw new Error("session/new must be called before session/prompt");
    const text = flattenPrompt(params?.prompt);
    const rpcId = sendRpc({ command: "prompt", text });
    pending.set(rpcId, id);
    return;
  }

  if (method === "session/cancel") {
    if (rpc?.child?.stdin?.writable) {
      sendRpc({ command: "abort" });
    }
    return;
  }

  if (method === "session/load") {
    error(id, -32601, "session/load is not supported by ggcoder-acp");
    return;
  }

  if (method === "session/set_config_option") {
    error(id, -32601, "session/set_config_option is not supported by ggcoder-acp");
    return;
  }

  error(id, -32601, `Unsupported method: ${method}`);
}

const acpLines = createInterface({ input: process.stdin, terminal: false });
for await (const line of acpLines) {
  if (!line.trim()) continue;
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    continue;
  }

  try {
    await handleAcp(message);
  } catch (err) {
    if (message.id !== undefined) {
      error(message.id, -32000, err instanceof Error ? err.message : String(err));
    }
  }
}

await waitForPending();
if (rpc?.child) rpc.child.kill("SIGTERM");
