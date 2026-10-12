// Reads profiles/<profile>/bot/FORBIDDEN.md if the profile has its own, else the shared profiles/bot/FORBIDDEN.md: lines `- /regex/flags — reason`.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function profilesDir() {
  if (process.env.NOTEPAD_PROFILES) return process.env.NOTEPAD_PROFILES;
  const appData = path.join(os.homedir(), "Library/Application Support/dev.notepad.assistant/profiles");
  if (fs.existsSync(appData)) return appData;
  return path.join(path.dirname(fileURLToPath(import.meta.url)), "../../../profiles");
}

export function forbiddenRules(profile) {
  const own = profile ? path.join(profilesDir(), profile, "bot", "FORBIDDEN.md") : "";
  const file = own && fs.existsSync(own) ? own : path.join(profilesDir(), "bot", "FORBIDDEN.md");
  if (!fs.existsSync(file)) return [];
  const rules = [];
  for (const line of fs.readFileSync(file, "utf8").split("\n")) {
    const m = line.match(/^- \/(.+)\/([a-z]*) — (.+)$/);
    if (m) rules.push({ re: new RegExp(m[1], m[2]), reason: m[3].replace(/\s*\{о себе\}/, ""), aboutCandidate: m[3].includes("{о себе}") });
  }
  return rules;
}
