// Prompt regression test (criterion №1: measure, don't guess). Runs every question in
// questions.json EVAL_REPEATS times through the real sidecar with the real system prompt,
// then counts leaks and forbidden facts. Use ./run.sh, it prepares system.txt and hints.json.
//
// A run FAILS an answer when it: leaks bookkeeping ("No company card", "knowledge base",
// routing hint), refuses ("NO_TASK_FOUND"), matches a pattern from profiles/bot/FORBIDDEN.md,
// matches a question's `must_not` (rules marked {о себе} skip kind "tech"), or misses a question's `must` (regexes, case-insensitive).
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";
import { forbiddenRules } from "./forbidden.mjs";

const dir = path.dirname(fileURLToPath(import.meta.url));
const out = process.env.EVAL_OUT || path.join(dir, "out");
const repeats = Number(process.env.EVAL_REPEATS || 3);
const model = process.env.EVAL_MODEL || "claude-haiku-4-5";
const system = fs.readFileSync(path.join(out, "system.txt"), "utf8");
const hints = JSON.parse(fs.readFileSync(path.join(out, "hints.json"), "utf8"));
const only = process.env.EVAL_ONLY; // substring filter on the question
const questions = JSON.parse(fs.readFileSync(process.env.JH_QUESTIONS || path.join(dir, "questions.json"), "utf8")).filter((x) => !only || x.q.includes(only));
const rules = forbiddenRules(process.env.JH_PROFILE);
const LEAKS = /no company card|knowledge base|routing hint|not a behavioral|NO_TASK_FOUND/i;

function ask(text) {
  return new Promise((resolve) => {
    const p = spawn(process.execPath, [path.join(dir, "..", "sidecar.mjs")], { stdio: ["pipe", "pipe", "ignore"] });
    const rl = readline.createInterface({ input: p.stdout });
    let answer = "", t0 = 0, first = null, done = false;
    const finish = (extra = {}) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      try { p.kill(); } catch {}
      resolve({ answer: answer.trim(), first_ms: first, ...extra });
    };
    const timer = setTimeout(() => finish({ error: "timeout" }), 60000);
    rl.on("line", (l) => {
      let m; try { m = JSON.parse(l); } catch { return; }
      if (m.type === "ready") {
        t0 = Date.now();
        p.stdin.write(JSON.stringify({ type: "ask", id: 1, text }) + "\n");
      } else if (m.type === "delta") { if (first === null) first = Date.now() - t0; answer += m.text; }
      else if (m.type === "done") finish();
      else if (m.type === "error") finish({ error: m.message });
    });
    p.stdin.write(JSON.stringify({ type: "start", system, model }) + "\n");
  });
}

const jobs = [];
for (const item of questions) for (let k = 0; k < repeats; k++) jobs.push(item);
const results = [];
let next = 0;
async function worker() {
  while (next < jobs.length) {
    const item = jobs[next++];
    const r = await ask(hints[item.q] || item.q); // hints.json = full user message built by the app (prepare_question)
    const problems = [];
    if (r.error) problems.push(`error: ${r.error}`);
    if (LEAKS.test(r.answer)) problems.push("leak/refusal");
    for (const f of rules) {
      if (f.aboutCandidate && item.kind === "tech") continue; // generic technical advice is not a claim about the candidate
      if (f.re.test(r.answer)) problems.push(`forbidden: ${f.reason}`);
    }
    for (const s of item.must_not || []) if (new RegExp(s, "im").test(r.answer)) problems.push(`must_not: ${s}`);
    for (const s of item.must || []) if (!new RegExp(s, "i").test(r.answer)) problems.push(`missing: ${s}`);
    results.push({ ...item, ...r, problems });
    process.stderr.write(`\r${results.length}/${jobs.length}`);
  }
}
await Promise.all(Array.from({ length: 4 }, worker));
process.stderr.write("\n");
fs.writeFileSync(path.join(out, "results.json"), JSON.stringify(results, null, 1));

let bad = 0;
for (const item of questions) {
  const rs = results.filter((r) => r.q === item.q);
  const fails = rs.filter((r) => r.problems.length);
  bad += fails.length;
  const why = [...new Set(fails.flatMap((r) => r.problems))].join("; ");
  console.log(`${fails.length ? "FAIL" : "ok  "} ${rs.length - fails.length}/${rs.length}  ${item.q}${why ? `  → ${why}` : ""}`);
}
// Answer length per kind (owner 2026-10-12: answers must be shorter, simpler, more natural).
for (const kind of [...new Set(results.map((r) => r.kind))]) {
  const w = results.filter((r) => r.kind === kind).map((r) => r.answer.split(/\s+/).filter(Boolean).length).sort((a, b) => a - b);
  console.log(`words ${kind}: median ${w[Math.floor(w.length / 2)]}, max ${w[w.length - 1]} (${w.length} answers)`);
}
const avg = Math.round(results.reduce((a, r) => a + (r.first_ms || 0), 0) / results.length);
console.log(`\n${bad ? "FAILED" : "PASSED"}: ${bad} bad of ${results.length} answers (avg first word ${avg} ms, 4 in parallel, cold sessions: not comparable with live speed)`);
process.exit(bad ? 1 : 0);
