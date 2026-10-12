// Static check: no forbidden statement in the READY-TO-SPEAK lines (`> ...`) of the knowledge base.
// Notes like «500k убрано, не называть» are not answers, so only quoted answer lines are scanned.
import fs from "node:fs";
import path from "node:path";
import { forbiddenRules, profilesDir } from "./forbidden.mjs";

const root = profilesDir();
let ruleCount = 0;
const files = []; // [relative path, profile or ""]
// Only what reaches the prompt: <profile>/bot/knowledge.md of every profile + company cards.
for (const p of fs.readdirSync(root)) {
  if (fs.existsSync(path.join(root, p, "bot", "knowledge.md"))) files.push([`${p}/bot/knowledge.md`, p]);
}
const cards = path.join(root, "companies");
if (fs.existsSync(cards)) for (const f of fs.readdirSync(cards)) if (f.endsWith(".md")) files.push([`companies/${f}`, ""]);

let hits = 0;
for (const [f, profile] of files) {
  const rules = forbiddenRules(profile);
  ruleCount = Math.max(ruleCount, rules.length);
  fs.readFileSync(path.join(root, f), "utf8").split("\n").forEach((line, i) => {
    if (!line.startsWith(">")) return;
    for (const r of rules) if (r.re.test(line)) { hits++; console.log(`${f}:${i + 1}: ${r.reason}\n   ${line.slice(0, 140)}`); }
  });
}
console.log(hits ? `\nFOUND ${hits} forbidden statement(s) in the knowledge base` : `knowledge base clean (${files.length} files, ${ruleCount} rules)`);
process.exit(hits ? 1 : 0);
