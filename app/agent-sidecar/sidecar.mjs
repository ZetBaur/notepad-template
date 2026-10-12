// Long-lived bridge between the Rust backend and Claude Code (Agent SDK).
//
// One query() stays open for the whole interview and new questions are fed
// into its input queue — calling query() per question respawns the claude
// process every time (~1.5s extra, measured). Protocol: one JSON object per
// line on stdin/stdout; stdout carries protocol lines only.
//
//   in:  {type:"start", system, model}       (re)open the session
//        {type:"ask", id, text, image?:{media_type,data}}
//        {type:"set_model", model}
//        {type:"cancel"}
//   out: {type:"ready"} | {type:"delta",id,text} | {type:"done",id}
//        | {type:"error",id?,message}

import os from "node:os";
import readline from "node:readline";
import { query } from "@anthropic-ai/claude-agent-sdk";

const out = (obj) => process.stdout.write(JSON.stringify(obj) + "\n");
console.log = (...a) => console.error(...a);

class InputQueue {
  constructor() {
    this.items = [];
    this.waiting = null;
    this.closed = false;
  }
  push(item) {
    if (this.waiting) {
      const w = this.waiting;
      this.waiting = null;
      w({ value: item, done: false });
    } else this.items.push(item);
  }
  close() {
    this.closed = true;
    if (this.waiting) this.waiting({ value: undefined, done: true });
  }
  [Symbol.asyncIterator]() {
    return {
      next: () => {
        if (this.items.length) return Promise.resolve({ value: this.items.shift(), done: false });
        if (this.closed) return Promise.resolve({ value: undefined, done: true });
        return new Promise((r) => (this.waiting = r));
      },
    };
  }
}

let session = null; // { query, input, pendingIds }

async function closeSession() {
  if (!session) return;
  const s = session;
  session = null;
  s.input.close();
  try {
    s.query.close();
  } catch {}
}

async function consume(s) {
  try {
    for await (const msg of s.query) {
      if (msg.type === "stream_event") {
        const ev = msg.event;
        if (ev?.type === "content_block_delta" && ev.delta?.type === "text_delta" && ev.delta.text) {
          out({ type: "delta", id: s.pendingIds[0], text: ev.delta.text });
        }
      } else if (msg.type === "result") {
        const id = s.pendingIds.shift();
        if (msg.is_error) out({ type: "error", id, message: String(msg.result ?? msg.subtype ?? "agent error") });
        else out({ type: "done", id });
      }
    }
  } catch (e) {
    if (session === s) out({ type: "error", id: s.pendingIds.shift(), message: String(e?.message ?? e) });
  }
}

async function start({ system, model }) {
  await closeSession();
  const options = {
    model,
    systemPrompt: system,
    tools: [],
    mcpServers: {},
    strictMcpConfig: true,
    thinking: { type: "disabled" },
    settingSources: [],
    persistSession: false,
    includePartialMessages: true,
    cwd: os.tmpdir(),
  };
  const input = new InputQueue();
  const s = { query: query({ prompt: input, options }), input, pendingIds: [] };
  session = s;
  void consume(s);
  out({ type: "ready" });
}

async function handle(line) {
  let m;
  try {
    m = JSON.parse(line);
  } catch {
    return out({ type: "error", message: "bad json from host" });
  }
  try {
    if (m.type === "start") return await start(m);
    if (!session) return out({ type: "error", id: m.id, message: "session not started" });
    if (m.type === "ask") {
      const content = [];
      if (m.image) content.push({ type: "image", source: { type: "base64", media_type: m.image.media_type, data: m.image.data } });
      content.push({ type: "text", text: m.text });
      session.pendingIds.push(m.id);
      session.input.push({ type: "user", message: { role: "user", content }, parent_tool_use_id: null });
    } else if (m.type === "set_model") {
      await session.query.setModel(m.model);
    } else if (m.type === "cancel") {
      await session.query.interrupt();
    }
  } catch (e) {
    out({ type: "error", id: m.id, message: String(e?.message ?? e) });
  }
}

// Handle lines strictly in order so `ask` never overtakes a pending `start`.
let chain = Promise.resolve();
const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  if (line.trim()) chain = chain.then(() => handle(line));
});
rl.on("close", async () => {
  await closeSession();
  process.exit(0);
});
