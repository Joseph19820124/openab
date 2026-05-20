#!/usr/bin/env node
/**
 * agy-acp: ACP stdio JSON-RPC adapter for Google Antigravity CLI (agy).
 *
 * Translates the ACP protocol (used by openab) into agy --print invocations.
 * Per-session conversation continuity is maintained via agy --continue.
 *
 * Protocol handled:
 *   initialize        → agentInfo + agentCapabilities
 *   session/new       → allocate sessionId, record cwd
 *   session/prompt    → run `agy [--continue] --print "<text>"`, stream result
 *   session/cancel    → no-op (agy --print is not interruptible)
 *   *                 → empty result (graceful fallback)
 */

import { spawn } from "child_process";
import { createInterface } from "readline";
import { randomUUID } from "crypto";

// sessionId → { cwd: string, hasMessages: boolean }
const sessions = new Map();

// ── Helpers ──────────────────────────────────────────────────────────────────

function writeLine(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function respond(id, result) {
  writeLine({ jsonrpc: "2.0", id, result });
}

function respondError(id, code, message) {
  writeLine({ jsonrpc: "2.0", id, error: { code, message } });
}

function notifyTextChunk(text) {
  writeLine({
    jsonrpc: "2.0",
    method: "notification",
    params: {
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { text },
      },
    },
  });
}

// ── agy invocation ───────────────────────────────────────────────────────────

function runAgy(args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn("agy", args, {
      cwd,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(stderr.trim() || `agy exited with code ${code}`));
      } else {
        resolve(stdout);
      }
    });
  });
}

// ── Request handlers ─────────────────────────────────────────────────────────

function handleInitialize(id) {
  respond(id, {
    agentInfo: { name: "agy", version: "1.0.0" },
    agentCapabilities: {},
    protocolVersion: 1,
  });
}

function handleSessionNew(id, params) {
  const sessionId = randomUUID();
  sessions.set(sessionId, {
    cwd: params?.cwd ?? process.cwd(),
    hasMessages: false,
  });
  respond(id, { sessionId });
}

async function handleSessionPrompt(id, params) {
  const sessionId = params?.sessionId;
  const session = sessions.get(sessionId);
  if (!session) {
    respondError(id, -32600, `Unknown session: ${sessionId}`);
    return;
  }

  // Extract text from ACP content blocks (skip images for now)
  const blocks = Array.isArray(params?.prompt) ? params.prompt : [];
  const text = blocks
    .filter((b) => b.type === "text")
    .map((b) => b.text)
    .join("\n")
    .trim();

  if (!text) {
    respond(id, {});
    return;
  }

  const args = [];
  if (session.hasMessages) {
    args.push("--continue");
  }
  args.push("--print", text);
  session.hasMessages = true;

  try {
    const output = await runAgy(args, session.cwd);
    if (output.trim()) {
      notifyTextChunk(output);
    }
    respond(id, {});
  } catch (err) {
    respondError(id, -32000, String(err.message ?? err));
  }
}

// ── Main dispatch loop ────────────────────────────────────────────────────────

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });

rl.on("line", (raw) => {
  const line = raw.trim();
  if (!line) return;

  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }

  const { id, method, params } = msg;

  switch (method) {
    case "initialize":
      handleInitialize(id);
      break;

    case "session/new":
      handleSessionNew(id, params);
      break;

    case "session/prompt":
      handleSessionPrompt(id, params).catch((err) => {
        if (id != null) respondError(id, -32000, String(err));
      });
      break;

    case "session/cancel":
      // agy --print is not cancellable; acknowledge silently
      break;

    default:
      if (id != null) respond(id, {});
  }
});

rl.on("close", () => process.exit(0));
