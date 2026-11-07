import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  getCurrentWindow,
  PhysicalPosition,
  LogicalSize,
  currentMonitor,
} from "@tauri-apps/api/window";

// Codes are whisper's own ISO-639-1 language identifiers; the names are what
// the user sees and what gets sent to the LLM as the response language — an
// LLM understands "Portuguese" directly, whereas whisper needs "pt".
type Language = { name: string; code: string };

// Top of the list, per docs/PRODUCT-BRIEF.md §5 ("Язык") — ordered by rough
// estimate of non-English-speaking developer market size, not SEO data (the
// site/UI stay English-only; this is a quiet per-conversation setting).
const TOP_LANGUAGES: Language[] = [
  { name: "English", code: "en" },
  { name: "Spanish", code: "es" },
  { name: "Portuguese", code: "pt" },
  { name: "German", code: "de" },
  { name: "French", code: "fr" },
  { name: "Mandarin Chinese", code: "zh" },
];

const MORE_LANGUAGES: Language[] = [
  { name: "Arabic", code: "ar" },
  { name: "Bengali", code: "bn" },
  { name: "Bulgarian", code: "bg" },
  { name: "Catalan", code: "ca" },
  { name: "Croatian", code: "hr" },
  { name: "Czech", code: "cs" },
  { name: "Danish", code: "da" },
  { name: "Dutch", code: "nl" },
  { name: "Finnish", code: "fi" },
  { name: "Greek", code: "el" },
  { name: "Hebrew", code: "he" },
  { name: "Hindi", code: "hi" },
  { name: "Hungarian", code: "hu" },
  { name: "Indonesian", code: "id" },
  { name: "Italian", code: "it" },
  { name: "Japanese", code: "ja" },
  { name: "Korean", code: "ko" },
  { name: "Malay", code: "ms" },
  { name: "Norwegian", code: "no" },
  { name: "Polish", code: "pl" },
  { name: "Romanian", code: "ro" },
  { name: "Russian", code: "ru" },
  { name: "Slovak", code: "sk" },
  { name: "Swedish", code: "sv" },
  { name: "Thai", code: "th" },
  { name: "Turkish", code: "tr" },
  { name: "Ukrainian", code: "uk" },
  { name: "Urdu", code: "ur" },
  { name: "Vietnamese", code: "vi" },
].sort((a, b) => a.name.localeCompare(b.name));

const DEFAULT_MODEL = "claude-haiku-4-5";

// Mirrors Settings in src-tauri/src/settings.rs (serde camelCase). Rust also
// has provider/hasUsedApp fields nothing here uses anymore; serde fills them
// with defaults on save.
type PersistedSettings = {
  model: string;
  interviewContext: string;
  company: string;
  profile: string;
  sttLanguage: string;
  captureExclusion: boolean;
  zoomLevel: number;
  audioPermissionGranted: boolean | null;
};

const EMPTY_SETTINGS: PersistedSettings = {
  model: "",
  interviewContext: "",
  company: "",
  profile: "",
  sttLanguage: "",
  captureExclusion: true,
  zoomLevel: 1,
  audioPermissionGranted: null,
};

async function loadPersistedSettings(): Promise<PersistedSettings> {
  try {
    return await invoke<PersistedSettings>("load_settings");
  } catch (err) {
    console.error("failed to load saved settings", err);
    return EMPTY_SETTINGS;
  }
}

// macOS-only deep link straight to the "Screen & System Audio Recording"
// pane, so the user doesn't have to hunt for it in System Settings — see
// docs/PRODUCT-BRIEF.md §3 (Variant A, macOS permission note).
const MACOS_SCREEN_RECORDING_SETTINGS_URL =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

// Only English and Russian are supported (owner decision, 2026-09-19).
const SUPPORTED_LANGUAGE_CODES = ["en", "ru"];
const ALL_LANGUAGES = [...TOP_LANGUAGES, ...MORE_LANGUAGES].filter((l) => SUPPORTED_LANGUAGE_CODES.includes(l.code));
const AUTO_DETECT_NAME = "Auto-detect";

/// STT language is stored as an ISO code (what whisper takes); the LLM
/// prompt's response language wants the plain name instead — this bridges
/// the two. Falls back to "English" for "auto" (no single name applies to
/// auto-detect).
function languageNameForCode(code: string): string {
  return ALL_LANGUAGES.find((lang) => lang.code === code)?.name ?? "English";
}

/// Reverse of languageNameForCode, for reading back what the user typed into
/// a language combobox. Case-insensitive since typed text shouldn't have to
/// match the canonical casing exactly.
function codeForLanguageName(name: string): string {
  if (name.trim().toLowerCase() === AUTO_DETECT_NAME.toLowerCase()) return "auto";
  return (
    ALL_LANGUAGES.find((lang) => lang.name.toLowerCase() === name.trim().toLowerCase())?.code ?? ""
  );
}

// ---------- custom dropdown (replaces native <select> and <datalist>) ----------
// Both are OS-controlled popup chrome that CSS can never reach — no hover
// state, and datalist additionally only filters against whatever text is
// *already* in the input (so a pre-filled field shows just one self-matching
// suggestion the moment it's opened) and renders value/label as two stacked
// lines (repeating text in both duplicated it on screen). This is a plain
// div we fully own instead. See the .dropdown CSS comment for the same
// rationale from the styling side.

type DropdownOption = { value: string; label: string; hint?: string };

/// Low-level menu behavior shared by every dropdown on the settings screen:
/// positioning (via the .dropdown-menu CSS, absolutely positioned under
/// `container`), rendering, hover/keyboard highlight, and outside-click/
/// Escape to close. `anchorEl` is whatever should open/filter the menu — a
/// button for fixed-choice fields, a text input for searchable ones.
function setupDropdownMenu(
  container: HTMLElement,
  anchorEl: HTMLElement,
  menuEl: HTMLElement,
  getOptions: () => DropdownOption[],
  onPick: (value: string) => void,
) {
  let activeIndex = -1;
  let visibleOptions: DropdownOption[] = [];

  function render(filterQuery: string | null) {
    const all = getOptions();
    const query = filterQuery?.trim().toLowerCase();
    visibleOptions = query
      ? all.filter(
          (o) => o.label.toLowerCase().includes(query) || o.value.toLowerCase().includes(query),
        )
      : all;

    menuEl.innerHTML = "";
    if (visibleOptions.length === 0) {
      const empty = document.createElement("div");
      empty.className = "dropdown-option-empty";
      empty.textContent = "No matches";
      menuEl.appendChild(empty);
      activeIndex = -1;
      return;
    }
    visibleOptions.forEach((opt, i) => {
      const row = document.createElement("div");
      row.className = "dropdown-option";
      row.textContent = opt.label;
      if (opt.hint) {
        const hint = document.createElement("span");
        hint.className = "dropdown-option-hint";
        hint.textContent = opt.hint;
        row.appendChild(hint);
      }
      // mousedown + preventDefault (not click) so selecting an option never
      // blurs the input first — a plain click would fire the input's blur
      // (closing the menu and, for the language combobox, reverting an
      // in-progress typed value) before this handler ever ran.
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        onPick(opt.value);
        close();
      });
      row.addEventListener("mouseenter", () => setActive(i));
      menuEl.appendChild(row);
    });
    activeIndex = -1;
  }

  function setActive(index: number) {
    const rows = menuEl.querySelectorAll<HTMLElement>(".dropdown-option");
    rows.forEach((r) => r.classList.remove("active"));
    activeIndex = index;
    if (index >= 0 && rows[index]) {
      rows[index].classList.add("active");
      rows[index].scrollIntoView({ block: "nearest" });
    }
  }

  function open(filterQuery: string | null) {
    render(filterQuery);
    menuEl.toggleAttribute("hidden", false);
  }

  function close() {
    menuEl.toggleAttribute("hidden", true);
    activeIndex = -1;
  }

  function isOpen() {
    return !menuEl.hasAttribute("hidden");
  }

  anchorEl.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!isOpen()) open(null);
      else setActive(Math.min(activeIndex + 1, visibleOptions.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (isOpen()) setActive(Math.max(activeIndex - 1, 0));
    } else if (e.key === "Enter") {
      if (isOpen() && activeIndex >= 0) {
        e.preventDefault();
        onPick(visibleOptions[activeIndex].value);
        close();
      }
    } else if (e.key === "Escape" && isOpen()) {
      close();
    }
  });

  anchorEl.addEventListener("blur", () => close());

  document.addEventListener("mousedown", (e) => {
    if (!container.contains(e.target as Node)) close();
  });

  return { open, close, isOpen };
}

/// Fixed-choice replacement for a native <select> (Model, Answer style,
/// Speech recognition model): the real <select> stays in the DOM (`hidden`)
/// so every existing `.value` read/write and `"change"` listener elsewhere
/// in this file keeps working untouched — this only adds a styled trigger
/// button in front of it that stays in sync by going *through* the select
/// (set .value, dispatch "change") instead of beside it.
/// Visual-only chevron for the typeable comboboxes (Model, Interview
/// language) — see the `.dropdown-chevron` CSS comment for why this exists
/// alongside `.dropdown-trigger`'s own chevron rather than reusing it: those
/// two dropdown flavors render a real visible `<input>` vs. a hidden
/// `<select>` behind a button, so the chevron has to be its own element here.
function addDropdownChevron(container: HTMLElement) {
  const chevron = document.createElement("span");
  chevron.className = "dropdown-chevron";
  container.appendChild(chevron);
}

function enhanceSelectDropdown(selectEl: HTMLSelectElement) {
  const container = selectEl.closest<HTMLElement>(".dropdown")!;

  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "dropdown-trigger";
  container.insertBefore(trigger, selectEl);

  const menuEl = document.createElement("div");
  menuEl.className = "dropdown-menu";
  menuEl.hidden = true;
  container.appendChild(menuEl);

  const getOptions = (): DropdownOption[] =>
    Array.from(selectEl.options).map((o) => ({ value: o.value, label: o.textContent ?? o.value }));

  function refreshTriggerLabel() {
    trigger.textContent = getOptions().find((o) => o.value === selectEl.value)?.label ?? selectEl.value;
  }
  refreshTriggerLabel();
  selectEl.addEventListener("change", refreshTriggerLabel);

  const menu = setupDropdownMenu(container, trigger, menuEl, getOptions, (value) => {
    selectEl.value = value;
    refreshTriggerLabel();
    selectEl.dispatchEvent(new Event("change", { bubbles: true }));
  });

  trigger.addEventListener("click", () => {
    if (menu.isOpen()) menu.close();
    else menu.open(null);
  });
}

/// A plain <select> forces scanning a long alphabetical list to find one
/// language — replaced with a free-text input backed by the same custom
/// dropdown menu as enhanceModelDropdown. Only the *names* are ever put in
/// the option list; STT's ISO code is derived via codeForLanguageName() at
/// the point of use, not stored as the input's value, so typing "Fren…"
/// actually matches "French" instead of relying on a hidden code the user
/// never sees.
function setupLanguageCombobox(options: {
  inputEl: HTMLInputElement;
  includeAutoDetect: boolean;
  initialValue: string;
  onChange?: (name: string) => void;
}) {
  const { inputEl, includeAutoDetect, initialValue, onChange } = options;
  const container = inputEl.closest<HTMLElement>(".dropdown")!;
  addDropdownChevron(container);
  const menuEl = document.createElement("div");
  menuEl.className = "dropdown-menu";
  menuEl.hidden = true;
  container.appendChild(menuEl);

  const names = [
    ...(includeAutoDetect ? [AUTO_DETECT_NAME] : []),
    ...ALL_LANGUAGES.map((l) => l.name),
  ];
  const dropdownOptions: DropdownOption[] = names.map((name) => ({ value: name, label: name }));

  const isValid = (value: string) => names.some((n) => n.toLowerCase() === value.trim().toLowerCase());
  let lastValid = isValid(initialValue) ? initialValue : names[0];
  inputEl.value = lastValid;

  function commit(name: string) {
    lastValid = name;
    inputEl.value = name;
    onChange?.(name);
  }

  const menu = setupDropdownMenu(container, inputEl, menuEl, () => dropdownOptions, commit);

  inputEl.addEventListener("focus", () => menu.open(null));
  inputEl.addEventListener("input", () => menu.open(inputEl.value));

  // Fires on blur/Enter — typing something that isn't in the list (typo,
  // incomplete word) snaps back to the last real selection instead of
  // silently persisting free text that whisper or the LLM prompt wouldn't
  // recognize. A pick via click/menu-Enter already called commit() above;
  // this only needs to handle "typed something then left without picking".
  inputEl.addEventListener("change", () => {
    if (isValid(inputEl.value)) {
      if (inputEl.value.trim() !== lastValid) commit(inputEl.value.trim());
    } else {
      inputEl.value = lastValid;
    }
  });

  return {
    getValue: () => lastValid,
    setValue: (name: string) => {
      lastValid = name;
      inputEl.value = name;
    },
  };
}

type Mode = "setup" | "compact" | "permission";

function setMode(mode: Mode) {
  document.body.dataset.mode = mode;
  document
    .querySelector<HTMLElement>("#setup-view")!
    .toggleAttribute("hidden", mode !== "setup");
  document
    .querySelector<HTMLElement>("#compact-view")!
    .toggleAttribute("hidden", mode !== "compact");
  document
    .querySelector<HTMLElement>("#permission-view")!
    .toggleAttribute("hidden", mode !== "permission");
}

// Global text-scale control (Cmd/Ctrl +/-/0) instead of a Small/Medium/Large
// settings-screen select — see docs/PROMPT-FONT-SIZE-OPTION.md. Everything
// in styles.css is sized in rem off :root's font-size, so scaling that one
// value scales the whole UI (both the setup screen and the compact panel)
// consistently, the same way a browser's page zoom would.
const ZOOM_BASE_PX = 14;
const ZOOM_MIN = 0.7;
const ZOOM_MAX = 1.8;
const ZOOM_STEP = 0.1;

function applyZoom(level: number) {
  document.documentElement.style.fontSize = `${ZOOM_BASE_PX * level}px`;
}

/// Brief on-screen "120%" pill so a zoom keystroke has visible feedback —
/// fades on its own rather than staying as permanent UI chrome, since this
/// is an occasional adjustment, not something to dedicate screen space to.
function flashZoomIndicator(level: number) {
  let el = document.querySelector<HTMLElement>("#zoom-indicator");
  if (!el) {
    el = document.createElement("div");
    el.id = "zoom-indicator";
    document.body.appendChild(el);
  }
  el.textContent = `${Math.round(level * 100)}%`;
  el.classList.remove("visible");
  // Force a reflow so re-adding the class restarts the fade transition even
  // if the previous flash is still fading out (rapid repeated key presses).
  void el.offsetWidth;
  el.classList.add("visible");
  window.clearTimeout(Number(el.dataset.fadeTimeout));
  el.dataset.fadeTimeout = String(
    window.setTimeout(() => el!.classList.remove("visible"), 900),
  );
}

const isMac = navigator.platform.toLowerCase().includes("mac");

// Show/Hide (⌘B/Ctrl+B) and Screenshot (⌘H/Ctrl+H) are fixed, non-
// configurable global hotkeys as of docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md
// (2026-09-15) — registered once in Rust at startup (see lib.rs), never via
// invoke() from here. This just formats a spec for display in the toolbar
// button badges.
function formatHotkeyForDisplay(spec: string): string {
  if (!spec) return "";
  if (!isMac) return spec.replace(/\bCmd\b/g, "Ctrl");
  return spec
    .replace(/\bCmd\b/g, "⌘")
    .replace(/\bCtrl\b/g, "⌃")
    .replace(/\bShift\b/g, "⇧")
    .replace(/\bAlt\b/g, "⌥")
    .replace(/\+/g, "");
}

// Mirrors the fixed specs lib.rs registers at startup (default_visibility_
// shortcut_spec/default_screenshot_shortcut_spec/default_start_over_
// shortcut_spec) — written once here in the same "Cmd+X" form
// formatHotkeyForDisplay already knows how to render per platform, purely
// for the toolbar badges/tip text. Not sent anywhere; Rust registers its own
// copy independently.
const FIXED_HOTKEYS = {
  showHide: "Cmd+B",
  screenshot: "Cmd+H",
  startOver: "Cmd+R",
};

// ---------- answer rendering (fenced code blocks + light syntax highlighting) ----------
// The LLM answer (llm.rs's system prompt) always fences code in a ``` block
// and writes prose (the complexity line) around it — this turns that into
// real HTML instead of rendering literal backticks and asterisks as plain
// text, since a raw fenced block is much harder to visually separate from
// the surrounding prose at a glance.
//
// Note the explanation now lives *inside* the block as code comments, so
// `.tok-comment` styling carries real content rather than decoration — see
// the note on that rule in styles.css before dimming it.

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// Deliberately not a real per-language parser — just enough token
// recognition (comments, strings, keywords across the common C-like/Python/
// Go/Rust keyword sets, numbers) to break up a monotone block of code
// visually. Good enough for a short interview-answer snippet; not meant to
// be exactly correct for every language.
const CODE_TOKEN_RE =
  /(\/\/[^\n]*|#[^\n]*|\/\*[\s\S]*?\*\/)|('(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*"|`(?:[^`\\]|\\.)*`)|\b(if|else|elif|for|while|do|def|function|fn|func|return|class|struct|enum|interface|trait|impl|extends|implements|const|let|var|mut|pub|static|public|private|protected|final|abstract|new|delete|this|self|super|try|catch|except|finally|throw|raise|async|await|break|continue|switch|match|case|default|yield|lambda|with|as|pass|and|or|not|in|of|is|typeof|instanceof|void|null|None|nil|undefined|true|false|True|False|import|from|export|package|module|use|mod|where|int|long|float|double|bool|boolean|string|String|char|byte)\b|\b(\d+\.?\d*)\b/g;

function highlightCode(code: string): string {
  return escapeHtml(code).replace(
    CODE_TOKEN_RE,
    (match, comment, string, keyword, number) => {
      if (comment) return `<span class="tok-comment">${comment}</span>`;
      if (string) return `<span class="tok-string">${string}</span>`;
      if (keyword) return `<span class="tok-keyword">${keyword}</span>`;
      if (number) return `<span class="tok-number">${number}</span>`;
      return match;
    },
  );
}

/// Escapes and lightly formats a non-code stretch of the answer: **bold**
/// and `inline code` become real markup, everything else is left to the
/// parent's `white-space: pre-wrap` (see .feed-item-answer) to preserve
/// line breaks — no manual newline-to-<br> conversion needed.
function renderInlineMarkdown(segment: string): string {
  return escapeHtml(segment)
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
    .replace(/`([^`]+)`/g, '<code class="inline-code">$1</code>');
}

const BULLET_LINE_RE = /^[-*•]\s+/;

/// Plain (non-code) answer as bullets: every non-empty line becomes one <li>
/// (owner, mock 2026-10-25). A single line stays plain text so a one-phrase
/// reply (or the first streamed line) isn't wrapped in a lone bullet.
function renderAnswerBullets(text: string): string | null {
  const lines = text
    .split("\n")
    .map((l) => l.trim().replace(BULLET_LINE_RE, ""))
    .filter((l) => l.length > 0);
  if (lines.length < 2) return null;
  const items = lines.map((l) => `<li>${renderInlineMarkdown(l)}</li>`).join("");
  return `<ul class="answer-bullets">${items}</ul>`;
}

/// Renders a leading "My Thoughts" bullet segment (see
/// docs/PROPOSAL-ANSWER-FORMAT-MY-THOUGHTS.md) as a labeled list instead of
/// plain text — only called on the stretch of text before the first code
/// fence, and only when that whole stretch is bullet lines, so a normal
/// (non-coding) bulleted answer that never reaches a fence isn't mislabeled.
function renderMyThoughts(segment: string): string | null {
  const lines = segment.split("\n").map((l) => l.trim());
  const bulletLines = lines.filter((l) => l.length > 0);
  if (bulletLines.length === 0 || !bulletLines.every((l) => BULLET_LINE_RE.test(l))) {
    return null;
  }
  const items = bulletLines
    .map((l) => `<li>${renderInlineMarkdown(l.replace(BULLET_LINE_RE, ""))}</li>`)
    .join("");
  return `<div class="my-thoughts"><div class="my-thoughts-label">Reasoning</div><ul>${items}</ul></div>`;
}

const ANSWER_HEADLINE_RE = /^Answer:\s*(.+?)\n\n([\s\S]*)$/;

/// Renders the leading "Answer: <headline>\n\n<explanation>" format llm.rs
/// asks for on non-coding questions (see the conceptual/factual branch) as a
/// labeled headline above the rest of the prose — only fires once the blank
/// line after the headline has actually streamed in, so a partial "Answer: "
/// prefix mid-stream just renders as plain text until then.
function renderAnswerHeadline(segment: string): string | null {
  const match = ANSWER_HEADLINE_RE.exec(segment);
  if (!match) return null;
  const [, headline, rest] = match;
  return (
    `<div class="answer-headline">${renderInlineMarkdown(headline)}</div>` +
    renderInlineMarkdown(rest)
  );
}

const COMPLEXITY_RE = /^Time:\s*(.+)\nSpace:\s*(.+)$/;

/// Renders the trailing "Time: ...\nSpace: ..." lines llm.rs asks for after
/// the code fence as a labeled block instead of plain prose — mirrors
/// renderMyThoughts below. Only matches when the ENTIRE remaining segment is
/// just those two lines, so it doesn't misfire on an answer that happens to
/// mention "Time:" partway through some other explanation.
function renderComplexity(segment: string): string | null {
  const match = COMPLEXITY_RE.exec(segment.trim());
  if (!match) return null;
  const [, time, space] = match;
  return (
    `<div class="complexity"><div class="complexity-label">Complexity</div>` +
    `<div class="complexity-row"><span class="complexity-key">Time</span>${renderInlineMarkdown(time)}</div>` +
    `<div class="complexity-row"><span class="complexity-key">Space</span>${renderInlineMarkdown(space)}</div>` +
    `</div>`
  );
}

/// Scans `text` for ```-fenced code blocks, rendering each as a highlighted
/// <pre><code> and everything else via renderInlineMarkdown. Runs on the
/// full accumulated answer on every streamed delta (see the llm-delta
/// listener), so it has to tolerate a fence that hasn't closed yet —
/// treated as "still-open code block," which lets the code area render
/// (and grow) live while the model is mid-stream instead of flashing raw
/// backticks until the closing fence finally arrives.
function renderAnswerHtml(text: string): string {
  let html = "";
  let i = 0;
  while (i < text.length) {
    const fenceIdx = text.indexOf("```", i);
    if (fenceIdx === -1) {
      const trailing = text.slice(i);
      // i === 0 and no fence anywhere: a non-coding answer, try the
      // "Answer: " headline. i > 0: text after a code block, try the
      // complexity format. Either mismatching (e.g. a non-coding answer
      // that happens to mention "Time:") just falls through to plain text.
      const rendered =
        i === 0 ? renderAnswerHeadline(trailing) : renderComplexity(trailing);
      html += rendered ?? (i === 0 ? renderAnswerBullets(trailing) : null) ?? renderInlineMarkdown(trailing);
      break;
    }
    const preFence = text.slice(i, fenceIdx);
    html += (i === 0 ? renderMyThoughts(preFence) : null) ?? renderInlineMarkdown(preFence);

    const afterFence = fenceIdx + 3;
    const newlineIdx = text.indexOf("\n", afterFence);
    let lang = "";
    let codeStart = afterFence;
    if (newlineIdx !== -1) {
      const langCandidate = text.slice(afterFence, newlineIdx).trim();
      // A real language tag is a short bare word (e.g. "python", "cpp") —
      // anything else on that line means there was no language tag at all
      // and this is already the first line of code.
      if (/^[a-zA-Z0-9+#]{0,16}$/.test(langCandidate)) {
        lang = langCandidate;
        codeStart = newlineIdx + 1;
      }
    }

    const closeIdx = text.indexOf("```", codeStart);
    const codeEnd = closeIdx === -1 ? text.length : closeIdx;
    const code = text.slice(codeStart, codeEnd);
    html += `<pre class="code-block"><code class="language-${escapeHtml(lang)}">${highlightCode(code)}</code></pre>`;
    i = closeIdx === -1 ? text.length : closeIdx + 3;
  }
  return html;
}

function showPermissionScreen(kind: "permission_denied" | "unsupported_os" | "error", detail?: string) {
  const title = document.querySelector<HTMLElement>("#permission-title")!;
  const message = document.querySelector<HTMLElement>("#permission-message")!;
  const openSettingsButton = document.querySelector<HTMLButtonElement>(
    "#open-system-settings-button",
  )!;

  if (kind === "permission_denied") {
    title.textContent = "Enable system audio access";
    message.textContent = isMac
      ? "Notepad needs permission to hear your call's audio. macOS requires you to turn this on yourself — it can't be granted automatically.\n\n1. Click \"Open System Settings\" below.\n2. Turn on the toggle for Notepad under Screen & System Audio Recording.\n3. Come back here and click \"Try again\" (you may need to restart Notepad)."
      : "Notepad couldn't access system audio. Check your OS's privacy/sound settings and try again.";
    openSettingsButton.toggleAttribute("hidden", !isMac);
  } else if (kind === "unsupported_os") {
    title.textContent = "macOS update required";
    message.textContent =
      "System audio capture needs macOS 14.2 (Sonoma) or later. Please update macOS and try again.";
    openSettingsButton.toggleAttribute("hidden", true);
  } else {
    title.textContent = "Couldn't start audio capture";
    message.textContent = detail || "Something went wrong starting system audio capture.";
    openSettingsButton.toggleAttribute("hidden", true);
  }

  setMode("permission");
}

window.addEventListener("DOMContentLoaded", async () => {
  const modelSelect = document.querySelector<HTMLSelectElement>("#model-select")!;
  const interviewContext = document.querySelector<HTMLSelectElement>("#interview-context")!;
  const companySelect = document.querySelector<HTMLSelectElement>("#company-select")!;
  const reloadKnowledgeButton = document.querySelector<HTMLButtonElement>("#reload-knowledge-button")!;
  const reloadKnowledgeNote = document.querySelector<HTMLElement>("#reload-knowledge-note")!;
  const sttLanguageInput = document.querySelector<HTMLInputElement>("#stt-language-input")!;
  const captureExclusionCheckbox = document.querySelector<HTMLInputElement>("#capture-exclusion-checkbox")!;
  const tipHotkeyClause = document.querySelector<HTMLElement>("#tip-hotkey-clause")!;
  // Toolbar (docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md, 2026-09-15) — Show/
  // Hide, Screenshot and Start Over, each fixed to a hotkey no longer shown
  // via #quick-screenshot-button's old footer position; the hotkey badges
  // are filled in from the same fixed specs lib.rs registers at startup
  // (see FIXED_HOTKEYS below), not read from settings.
  const showHideButton = document.querySelector<HTMLButtonElement>("#show-hide-button")!;
  const showHideHotkeyBadge = document.querySelector<HTMLElement>("#show-hide-hotkey-badge")!;
  const screenshotHotkeyBadge = document.querySelector<HTMLElement>("#screenshot-hotkey-badge")!;
  const startOverButton = document.querySelector<HTMLButtonElement>("#start-over-button")!;
  const startOverHotkeyBadge = document.querySelector<HTMLElement>("#start-over-hotkey-badge")!;
  const startListeningButton = document.querySelector<HTMLButtonElement>("#start-listening-button")!;
  const settingsButton = document.querySelector<HTMLButtonElement>("#settings-button")!;
  const collapseAllButton = document.querySelector<HTMLButtonElement>("#collapse-all-button")!;
  const setupView = document.querySelector<HTMLElement>("#setup-view")!;
  // #add-screenshot-button commented out in index.html — see the matching
  // commented-out captureRegionScreenshot()/click-listener below.
  const quickScreenshotButton = document.querySelector<HTMLButtonElement>("#quick-screenshot-button")!;
  const audioMeterFill = document.querySelector<HTMLElement>("#audio-meter-fill")!;
  const segmentCountText = document.querySelector<HTMLElement>("#segment-count-text")!;
  const statusText = document.querySelector<HTMLElement>("#status-text")!;
  const openSystemSettingsButton = document.querySelector<HTMLButtonElement>(
    "#open-system-settings-button",
  )!;
  const permissionRetryButton = document.querySelector<HTMLButtonElement>("#permission-retry-button")!;
  const permissionBackButton = document.querySelector<HTMLButtonElement>("#permission-back-button")!;
  const appVersionEl = document.querySelector<HTMLElement>("#app-version")!;
  const permissionsBlock = document.querySelector<HTMLElement>("#permissions-block")!;
  const permissionsReadyRow = document.querySelector<HTMLElement>("#permissions-ready-row")!;
  const permissionsDetail = document.querySelector<HTMLElement>("#permissions-detail")!;
  const audioPermissionStatus = document.querySelector<HTMLElement>("#audio-permission-status")!;
  const checkAudioPermissionButton = document.querySelector<HTMLButtonElement>(
    "#check-audio-permission-button",
  )!;
  const screenPermissionRow = document.querySelector<HTMLElement>("#screen-permission-row")!;
  const screenPermissionStatus = document.querySelector<HTMLElement>("#screen-permission-status")!;
  const screenPermissionNote = document.querySelector<HTMLElement>("#screen-permission-note")!;
  const screenPermissionManualHelp = document.querySelector<HTMLElement>(
    "#screen-permission-manual-help",
  )!;
  const screenPermissionManualSteps = document.querySelector<HTMLElement>(
    "#screen-permission-manual-steps",
  )!;
  const screenPermissionManualWarning = document.querySelector<HTMLElement>(
    "#screen-permission-manual-warning",
  )!;
  const requestScreenPermissionButton = document.querySelector<HTMLButtonElement>(
    "#request-screen-permission-button",
  )!;
  const startListeningReasonNote = document.querySelector<HTMLElement>(
    "#start-listening-reason-note",
  )!;

  // Fire-and-forget: shows which version is currently running next to the
  // app name, so it's clear at a glance alongside the "a new version is
  // available" note above (added 2026-08-30, owner request while testing
  // that same banner).
  void (async () => {
    try {
      appVersionEl.textContent = `v${await getVersion()}`;
    } catch (err) {
      console.error("failed to read app version", err);
    }
  })();

  // Load everything the user configured last time before any
  // of the hardcoded defaults below run, so a returning user never has to
  // redo their setup.
  const saved = await loadPersistedSettings();
  interviewContext.value = saved.interviewContext;

  // Falls back to 1 (100%) for both a genuinely first-ever run and an
  // older settings.json saved before this field existed (deserializes to
  // 0 there, not a usable scale).
  let currentZoom = saved.zoomLevel > 0 ? saved.zoomLevel : 1;
  applyZoom(currentZoom);

  // Last known result of the audio-permission probe, persisted across
  // restarts. Painted immediately below so the row isn't blank while
  // refreshAudioPermissionStatus() re-checks live (see its own comment for
  // why that re-check is safe to run automatically once a decision already
  // exists, unlike the very first-ever check).
  let audioPermissionGranted: boolean | null = saved.audioPermissionGranted;
  // Mirrors the Rust side's own copy, which is what actually flips this to
  if (audioPermissionGranted !== null) {
    audioPermissionStatus.textContent = audioPermissionGranted
      ? "System audio access — granted"
      : "System audio access — not granted";
    audioPermissionStatus.className = `permission-status ${audioPermissionGranted ? "granted" : "denied"}`;
    checkAudioPermissionButton.hidden = audioPermissionGranted;
  }

  function setZoom(level: number) {
    currentZoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(level * 10) / 10));
    applyZoom(currentZoom);
    flashZoomIndicator(currentZoom);
    void persistSettings();
  }

  // Cmd/Ctrl +/-/0, the same combo every browser and code editor already
  // uses for page/text zoom — no settings-screen control needed, see
  // docs/PROMPT-FONT-SIZE-OPTION.md. Bubble-phase window listener: harmless
  // to also have active while a hotkey-capture field has focus, since that
  // capture path calls stopPropagation() on the keys it actually consumes.
  window.addEventListener("keydown", (e) => {
    const modifierHeld = isMac ? e.metaKey : e.ctrlKey;
    if (!modifierHeld) return;
    if (e.key === "=" || e.key === "+") {
      e.preventDefault();
      setZoom(currentZoom + ZOOM_STEP);
    } else if (e.key === "-") {
      e.preventDefault();
      setZoom(currentZoom - ZOOM_STEP);
    } else if (e.key === "0") {
      e.preventDefault();
      setZoom(1);
    }
  });

  const sttLanguageCombobox = setupLanguageCombobox({
    inputEl: sttLanguageInput,
    includeAutoDetect: true,
    initialValue: saved.sttLanguage ? languageNameForCode(saved.sttLanguage) : "English",
    onChange: () => void persistSettings(),
  });

  // The LLM prompt can't be told to "respond in Auto-detect" — falls back to
  // English the same way the STT side falls back to whisper's own detection.
  function currentResponseLanguage(): string {
    const name = sttLanguageCombobox.getValue();
    return name === AUTO_DETECT_NAME ? "English" : name;
  }

  captureExclusionCheckbox.checked = saved.captureExclusion;
  // Declared before the first check below: the async check can resolve before
  // later code runs, and assigning to a `let` still in its temporal dead zone throws.
  let screenPermissionGranted = !isMac;
  void refreshScreenPermissionStatus();
  // Only once a decision already exists — see refreshAudioPermissionStatus's
  // own comment for why probing before any decision has ever been made is
  // left to the explicit button click instead.
  if (audioPermissionGranted !== null) {
    void refreshAudioPermissionStatus();
  }

  // Toolbar hotkey badges + the hint-feed tip clause — all three are fixed
  // (registered once in Rust at startup, see lib.rs), so this just paints
  // the display strings; nothing here can fail or need re-registering.
  showHideHotkeyBadge.textContent = formatHotkeyForDisplay(FIXED_HOTKEYS.showHide);
  screenshotHotkeyBadge.textContent = formatHotkeyForDisplay(FIXED_HOTKEYS.screenshot);
  startOverHotkeyBadge.textContent = formatHotkeyForDisplay(FIXED_HOTKEYS.startOver);
  tipHotkeyClause.textContent = `, or press ${formatHotkeyForDisplay(FIXED_HOTKEYS.screenshot)}`;

  function collectSettings(): PersistedSettings {
    return {
      model: modelSelect.value,
      interviewContext: interviewContext.value,
      company: companySelect.value,
      profile: saved.profile,
      sttLanguage: codeForLanguageName(sttLanguageCombobox.getValue()) || "en",
      captureExclusion: captureExclusionCheckbox.checked,
      zoomLevel: currentZoom,
      audioPermissionGranted,
    };
  }

  async function persistSettings() {
    try {
      await invoke("save_settings", { settings: collectSettings() });
    } catch (err) {
      console.error("failed to save settings", err);
    }
  }

  for (const field of [modelSelect, interviewContext, companySelect, captureExclusionCheckbox]) {
    field.addEventListener("change", () => void persistSettings());
  }
  // The language combobox persists via its own onChange (passed to
  // setupLanguageCombobox above) instead of this generic loop.

  // True between the click on "Start listening" and the window actually
  // becoming the hint panel. Startup is not instant (the speech model loads
  // and the capture helper has to report itself ready), so without visible
  // feedback the button looks like it ignored the click.
  let starting = false;
  let startingTimeout: number | undefined;

  function setStarting(on: boolean) {
    starting = on;
    startListeningButton.textContent = on ? "Starting…" : "Start listening";
    window.clearTimeout(startingTimeout);
    if (on) {
      startListeningButton.disabled = true;
      startListeningButton.title = "";
      // The transition is driven by an "audio-status" event, which a wedged
      // capture helper might never send. Without this the button would sit
      // disabled on "Starting…" forever, with no way to retry short of
      // restarting the app — worse than the pre-disable behaviour.
      startingTimeout = window.setTimeout(() => {
        setStarting(false);
        showPermissionScreen(
          "error",
          "System audio capture didn't report itself ready. Please try again.",
        );
      }, 30_000);
    } else {
      refreshStartButtonState();
    }
  }

  // Start is gated on the speech model being ready and (macOS) screen
  // recording being granted. One priority-ordered reason is shown both as the
  // button's tooltip and as visible text under it.
  function refreshStartButtonState() {
    // Never re-enable mid-startup — this also runs from unrelated events
    // (a model download finishing) that can land while a start is in flight.
    if (starting) {
      return;
    }
    startListeningButton.disabled = !sttModelReady || !screenPermissionGranted;
    const reason = !sttModelReady
      ? "Speech model not found — run `pnpm model` in app/ (downloads ggml-small.bin)"
      : !screenPermissionGranted
        ? "Grant screen recording access above to start listening"
        : "";
    startListeningButton.title = reason;
    startListeningReasonNote.textContent = reason;
    startListeningReasonNote.hidden = reason === "";
  }

  // A model saved before this list existed (or not in it) falls back to the
  // fast default.
  const knownModels = Array.from(modelSelect.options).map((o) => o.value);
  modelSelect.value = knownModels.includes(saved.model) ? saved.model : DEFAULT_MODEL;
  enhanceSelectDropdown(modelSelect);
  enhanceSelectDropdown(interviewContext);

  // Company cards are files in profiles/companies/; the list is read from
  // disk, so it is filled before the dropdown is enhanced.
  async function fillCompanies() {
    let items: { name: string; label: string }[] = [];
    try {
      items = await invoke<{ name: string; label: string }[]>("list_companies");
    } catch (err) {
      console.error("failed to list company cards", err);
    }
    const keep = companySelect.value || saved.company;
    companySelect.replaceChildren(new Option("None", ""));
    for (const c of items) companySelect.appendChild(new Option(c.label, c.name));
    companySelect.value = items.some((c) => c.name === keep) ? keep : "";
  }
  await fillCompanies();
  enhanceSelectDropdown(companySelect);

  // The knowledge profile has no setting: the value saved in settings.json is passed through
  // unchanged (empty = first profile on disk, see knowledge::read_profile).

  reloadKnowledgeButton.addEventListener("click", () => {
    void (async () => {
      try {
        const warning = await invoke<string>("reload_knowledge");
        await fillCompanies();
        companySelect.dispatchEvent(new Event("change"));
        reloadKnowledgeNote.classList.toggle("field-note-error", !!warning);
        reloadKnowledgeNote.textContent = warning ? `Reloaded, but found a problem:\n${warning}` : "Knowledge reloaded.";
      } catch (err) {
        reloadKnowledgeNote.classList.add("field-note-error");
        reloadKnowledgeNote.textContent = `Could not reload: ${String(err)}`;
      }
      reloadKnowledgeNote.hidden = false;
    })();
  });
  void invoke<string>("reload_knowledge")
    .then((warning) => {
      if (!warning) return;
      reloadKnowledgeNote.classList.add("field-note-error");
      reloadKnowledgeNote.textContent = `Knowledge base warning:\n${warning}`;
      reloadKnowledgeNote.hidden = false;
    })
    .catch((err) => console.error("reload_knowledge failed", err));


  // Show/Hide and Start Over toolbar buttons — Screenshot's own click
  // handler (captureFullScreenshot) is wired further down where that
  // function is defined. Both hotkeys themselves are registered once in
  // Rust at startup (lib.rs); these buttons just give a mouse-driven way to
  // trigger the exact same actions the fixed hotkeys do.
  showHideButton.addEventListener("click", () => {
    void invoke("toggle_window_visibility").catch((err) => {
      console.error("failed to toggle window visibility", err);
    });
  });

  function startOver() {
    hintFeed.innerHTML = hintFeedEmptyHtml;
    activeAnswerEl = null;
    activeAnswerText = "";
  }

  startOverButton.addEventListener("click", startOver);
  void listen("start-over-triggered", startOver);

  // Permissions section (docs/TODO.md, "Разрешения macOS запрашиваются не
  // вовремя") — lets the user grant both up front, from the setup screen,
  // instead of hitting an unexplained system dialog mid-call.

  // Whole block only exists on macOS — WASAPI loopback and GDI capture need
  // no OS permission on Windows, so there's nothing to check or explain
  // there (docs/TODO.md "Редизайн экрана настроек").
  permissionsBlock.hidden = !isMac;

  // macOS 26 (Tahoe) merged "Screen Recording" and "System Audio Recording"
  // into one System Settings category; older macOS still lists them
  // separately. Picked once at startup (it can't change mid-session) and
  // used only to word the manual recovery instructions correctly — has no
  // bearing on the permission checks themselves.
  if (isMac) {
    void (async () => {
      let tahoeOrLater = false;
      try {
        tahoeOrLater = await invoke<boolean>("is_macos_tahoe_or_later");
      } catch (err) {
        console.error("failed to detect macOS version", err);
      }
      screenPermissionManualSteps.innerHTML = tahoeOrLater
        ? `<li>Open System Settings.</li>
           <li>Go to Privacy &amp; Security.</li>
           <li>Click "Screen &amp; System Audio Recording".</li>
           <li>Turn on the switch next to Notepad.</li>
           <li>Restart Notepad.</li>`
        : `<li>Open System Settings.</li>
           <li>Go to Privacy &amp; Security.</li>
           <li>Turn on the switch next to Notepad under "Screen Recording".</li>
           <li>Also turn it on under "System Audio Recording".</li>
           <li>Restart Notepad.</li>`;
      screenPermissionManualWarning.hidden = !tahoeOrLater;
    })();
  }

  // See docs/TODO.md "Редизайн экрана настроек": replaces always showing
  // both permission rows with a single "Ready to listen" line once both are
  // granted — they're both required for every user (not one optional, one
  // required), so in the common case one line says everything there is to
  // say. The per-permission detail underneath takes back over automatically
  // the moment either one isn't granted, so nothing about the diagnostic is
  // lost. `audioPermissionGranted === null` (never checked yet) counts as
  // not ready, same as an explicit denial.
  function refreshPermissionsSummary() {
    const ready = screenPermissionGranted && audioPermissionGranted === true;
    permissionsReadyRow.hidden = !ready;
    permissionsDetail.hidden = ready;
  }

  // Gates "Start listening" below, same as hasKey/hasModel — added
  // 2026-08-16 (docs/TODO.md, "'Start listening' не гейтится на Screen
  // Recording permission") after the row above turned out to be purely
  // informational: nothing stopped a user from starting a call without
  // granting it, so the system dialog could still ambush them mid-interview
  // the first time they clicked a screenshot button, defeating the whole
  // point of asking ahead of time on this screen. Defaults to `true` off
  // macOS, where `check_screen_recording_permission` always reports granted
  // (see the comment below) and the button must never block on it.

  // Shown on macOS whenever screenshots are possible at all — which is
  // always, since the "Screenshot your code" toolbar button (and its fixed
  // ⌘H/Ctrl+H hotkey) is unconditionally present and shells out to
  // `screencapture`, which macOS gates behind this permission.
  //
  // This row used to be tied to the "Region screenshot hotkey" checkbox, on
  // the theory that people who leave it off never need the permission. That
  // was wrong: they still have both buttons, so the row was hidden from
  // exactly the users most likely to be surprised by the system dialog
  // mid-call — the thing this whole section exists to prevent. Found in the
  // 2026-08-06 clean-Mac run (docs/TODO.md, finding #2).
  //
  // Hidden off macOS: `check_screen_recording_permission` returns `true`
  // unconditionally there (no such permission exists), so the row would be a
  // permanently-green line for something the OS never asks about.
  // `check_screen_recording_permission` is a silent preflight (no dialog) —
  // safe to call automatically on load.
  async function refreshScreenPermissionStatus(options: { autoRequest?: boolean } = {}) {
    if (!isMac) {
      screenPermissionRow.hidden = true;
      screenPermissionNote.hidden = true;
      screenPermissionManualHelp.hidden = true;
      return;
    }
    screenPermissionRow.hidden = false;
    screenPermissionNote.hidden = false;
    screenPermissionManualHelp.hidden = false;
    try {
      const granted = await invoke<boolean>("check_screen_recording_permission");
      screenPermissionStatus.textContent = granted
        ? "Screen recording access — granted"
        : "Screen recording access — not granted";
      screenPermissionStatus.className = `permission-status ${granted ? "granted" : "denied"}`;
      requestScreenPermissionButton.hidden = granted;
      screenPermissionGranted = granted;
      refreshStartButtonState();
      refreshPermissionsSummary();
      if (!granted && options.autoRequest) {
        await requestScreenRecordingPermission();
      }
    } catch (err) {
      console.error("failed to check screen recording permission", err);
    }
  }

  async function requestScreenRecordingPermission() {
    requestScreenPermissionButton.disabled = true;
    try {
      // May show the system prompt (only if the decision isn't made yet).
      await invoke("request_screen_recording_permission");
    } catch (err) {
      console.error("failed to request screen recording permission", err);
    } finally {
      requestScreenPermissionButton.disabled = false;
      // A freshly granted decision often only takes effect for this process
      // after a relaunch (a macOS quirk) — this may still show "not
      // granted" right after allowing it, which is expected, not a bug.
      await refreshScreenPermissionStatus();
    }
  }

  requestScreenPermissionButton.addEventListener("click", () => void requestScreenRecordingPermission());

  // System audio has no preflight API (see permissions.rs/audio.rs) —
  // checking it and requesting it are the same action (creating the tap),
  // which is also what triggers macOS's system dialog, but *only* when the
  // decision is still undetermined. Once a decision exists — granted or
  // denied, doesn't matter — macOS never re-shows that dialog, so re-probing
  // then is side-effect-free and safe to run automatically (see the call
  // near `refreshScreenPermissionStatus()` above). The very first-ever check
  // stays behind the explicit button click, same as before.
  async function refreshAudioPermissionStatus(options: { showDeniedScreen?: boolean } = {}) {
    checkAudioPermissionButton.disabled = true;
    audioPermissionStatus.textContent = "System audio access — checking…";
    audioPermissionStatus.className = "permission-status";
    try {
      const granted = await invoke<boolean>("check_audio_permission");
      audioPermissionGranted = granted;
      await persistSettings();
      audioPermissionStatus.textContent = granted
        ? "System audio access — granted"
        : "System audio access — not granted";
      audioPermissionStatus.className = `permission-status ${granted ? "granted" : "denied"}`;
      checkAudioPermissionButton.hidden = granted;
      if (!granted && options.showDeniedScreen) {
        // Reuses the same guided recovery screen "Start listening" falls
        // back to on a real capture failure — same copy, same Open System
        // Settings / Try again / Back buttons, just reached a step earlier.
        // Only shown for an explicit click — an automatic background
        // re-check on load must never redirect the screen the user is
        // looking at out from under them.
        showPermissionScreen("permission_denied");
      }
    } catch (err) {
      audioPermissionStatus.textContent = "System audio access — couldn't check";
      audioPermissionStatus.className = "permission-status denied";
      checkAudioPermissionButton.hidden = false;
      console.error("failed to check audio permission", err);
    } finally {
      checkAudioPermissionButton.disabled = false;
      refreshPermissionsSummary();
    }
  }

  checkAudioPermissionButton.addEventListener("click", () => {
    void refreshAudioPermissionStatus({ showDeniedScreen: true });
  });

  let segmentCount = 0;

  // The speech model is fixed (the one bundled with the app, no setting). "Start
  // listening" stays disabled until it is found on disk.
  const STT_MODEL = "small";
  let sttModelReady = false;

  async function refreshModelStatus() {
    try {
      sttModelReady = await invoke<boolean>("is_model_downloaded", { model: STT_MODEL });
    } catch (err) {
      console.error("failed to check model", err);
      sttModelReady = false;
    }
    refreshStartButtonState();
  }
  void refreshModelStatus();

  async function beginListening() {
    if (starting) return;
    setStarting(true);

    interviewFile = null;
    for (const k of Object.keys(fillerBags)) delete fillerBags[k];
    segmentCount = 0;
    segmentCountText.textContent = "0 speech segments";
    audioMeterFill.style.width = "0%";
    statusText.textContent = "Starting…";

    try {
      await invoke("start_audio_capture", {
        model: STT_MODEL,
        language: codeForLanguageName(sttLanguageCombobox.getValue()) || "en",
      });
    } catch (err) {
      setStarting(false);
      showPermissionScreen("error", String(err));
      return;
    }
    // Actual transition to compact mode happens once the "audio-status"
    // event confirms capture is ready (see listener below) — starting the
    // helper process is not instant, and it may fail asynchronously (e.g.
    // permission denied) after this call has already resolved.
  }

  startListeningButton.addEventListener("click", () => {
    void beginListening();
  });

  async function returnToSetup() {
    try {
      await invoke("stop_audio_capture");
    } catch (err) {
      console.error("failed to stop audio capture", err);
    }
    try {
      await invoke("exit_listening_mode");
    } catch (err) {
      console.error("failed to exit listening mode", err);
    }
    setMode("setup");
    // exit_listening_mode (lib.rs) always resizes the window to the fixed
    // SETUP_SIZE constant, overwriting whatever height
    // resizeSetupWindowToContent() had fit it to before Start listening was
    // clicked — found live-testing 2026-08-22, same empty-space bug as
    // before but on the return path, which nothing was re-triggering it on.
    void resizeSetupWindowToContent();
  }

  // Compact panel dragging — macOS gets a hand-rolled implementation,
  // Windows keeps Tauri's native one. Reason: `data-tauri-drag-region`
  // invokes `performWindowDragWithEvent:` on macOS (a *live* WindowServer
  // move), and that live move turned out to silently strip the panel's
  // cross-Space visibility set up for the fullscreen-Space fix — found
  // 2026-08-08 (docs/TODO.md): dragging only worked on the panel's "home"
  // Space, and doing it there made the panel disappear from every other
  // fullscreen Space afterwards. Same root cause as that fix's `orderOut`
  // problem (docs/PRODUCT-BRIEF.md §5), just a second, independent trigger
  // for it that wasn't covered at the time. Plain `setPosition` calls (a
  // single `setFrameOrigin:`, not a live move loop) don't trigger it.
  // Windows has no equivalent cross-Space feature, so it keeps the native,
  // smoother drag.
  // Extracted 2026-09-16 (owner: dragging only worked on #compact-header,
  // not on the empty space in the toolbar row above it — the toolbar is now
  // the visual top of the panel, so that's the spot a user tries first) so
  // the same drag mechanism can attach to both rows. The existing
  // interactive-descendant exclusion below already keeps this from
  // hijacking clicks on the toolbar's own buttons.
  function attachDragRegion(el: HTMLElement) {
    if (isMac) {
      el.addEventListener("mousedown", (e) => {
        if (e.button !== 0) return;
        // Matches Tauri's own drag-region exclusion — a click that started
        // on an interactive descendant (a toolbar button, the settings
        // gear) must not also start a drag.
        if ((e.target as HTMLElement).closest("button, a, input, select, textarea")) {
          return;
        }
        e.preventDefault();
        const win = getCurrentWindow();
        const startScreenX = e.screenX;
        const startScreenY = e.screenY;
        const scale = window.devicePixelRatio || 1;
        void win.outerPosition().then((start) => {
          const onMove = (moveEvent: MouseEvent) => {
            const dx = Math.round((moveEvent.screenX - startScreenX) * scale);
            const dy = Math.round((moveEvent.screenY - startScreenY) * scale);
            void win.setPosition(new PhysicalPosition(start.x + dx, start.y + dy));
          };
          const onUp = () => {
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", onUp);
            // The live setPosition() loop above breaks cross-Space visibility
            // the same way performWindowDragWithEvent used to (docs/TODO.md,
            // 2026-08-19) — re-assert it once the drag has actually ended.
            void invoke("reassert_cross_space");
          };
          document.addEventListener("mousemove", onMove);
          document.addEventListener("mouseup", onUp);
        });
      });
    } else {
      el.setAttribute("data-tauri-drag-region", "deep");
    }
  }

  const compactHeader = document.querySelector<HTMLElement>("#compact-header")!;
  const compactToolbarEl = document.querySelector<HTMLElement>("#compact-toolbar")!;
  attachDragRegion(compactHeader);
  attachDragRegion(compactToolbarEl);

  // Bottom-right resize handle. The window has no native frame, so the edges
  // are hard to hit. Resizing is done by hand (setSize), same idea as
  // attachDragRegion; floor matches COMPACT_MIN_SIZE in lib.rs.
  const resizeGrip = document.querySelector<HTMLElement>("#resize-grip")!;
  resizeGrip.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const win = getCurrentWindow();
    const startX = e.screenX;
    const startY = e.screenY;
    void Promise.all([win.innerSize(), win.scaleFactor()]).then(([size, scale]) => {
      const startW = size.width / scale;
      const startH = size.height / scale;
      const onMove = (m: MouseEvent) => {
        const w = Math.max(280, startW + (m.screenX - startX));
        const h = Math.max(200, startH + (m.screenY - startY));
        void win.setSize(new LogicalSize(w, h));
      };
      const onUp = () => {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    });
  });

  // Normal <-> tall window toggle (owner, mock 2026-10-25): tall = full
  // screen height of the current monitor, narrow column, same left edge.
  // Remembers the normal size/position to come back to.
  const TALL_WIDTH = 380;
  const TALL_TOP_GAP = 30; // menu bar
  const TALL_BOTTOM_GAP = 10;
  let normalWindow: { w: number; h: number; x: number; y: number } | null = null;
  const sizeToggleButton = document.querySelector<HTMLButtonElement>("#size-toggle-button")!;
  sizeToggleButton.addEventListener("click", async () => {
    const win = getCurrentWindow();
    try {
      if (normalWindow) {
        const prev = normalWindow;
        normalWindow = null;
        await win.setSize(new LogicalSize(prev.w, prev.h));
        await win.setPosition(new PhysicalPosition(prev.x, prev.y));
      } else {
        const [size, scale, pos, monitor] = await Promise.all([
          win.innerSize(),
          win.scaleFactor(),
          win.outerPosition(),
          currentMonitor(),
        ]);
        if (!monitor) return;
        normalWindow = { w: size.width / scale, h: size.height / scale, x: pos.x, y: pos.y };
        const monitorH = monitor.size.height / scale;
        const maxX = monitor.position.x + monitor.size.width - TALL_WIDTH * scale;
        const x = Math.min(pos.x, Math.round(maxX));
        await win.setSize(new LogicalSize(TALL_WIDTH, monitorH - TALL_TOP_GAP - TALL_BOTTOM_GAP));
        await win.setPosition(
          new PhysicalPosition(x, monitor.position.y + Math.round(TALL_TOP_GAP * scale)),
        );
      }
      void invoke("reassert_cross_space");
    } catch (err) {
      console.error("failed to toggle window size", err);
    }
  });

  settingsButton.addEventListener("click", () => {
    normalWindow = null;
    void returnToSetup();
  });

  // The setup window has no content-based sizing of its own — it keeps
  // whatever height tauri.conf.json set initially (720px) or wherever the
  // user last dragged it, regardless of how many accordions are open. With
  // everything collapsed that leaves a large empty strip below "Start
  // listening" (found live-testing 2026-08-21, docs/TODO.md "Редизайн экрана
  // настроек"). Reruns on every accordion toggle so the window tracks actual
  // content height both ways — shrinking when sections collapse, growing
  // back when they reopen. Width is left untouched; `minHeight` in
  // tauri.conf.json still applies as a floor since Tauri enforces it on
  // `setSize` the same as on interactive resize. Only meaningful in setup
  // mode — the compact panel has its own remembered size (see
  // `enter_listening_mode` in `lib.rs`), this must never run there.
  //
  // Bug fixed 2026-08-22, found live-testing right after the first version
  // shipped: `#setup-view` itself is `height: 100vh` (styles.css) — always
  // exactly the window's current content height by definition, regardless
  // of how little is actually in it. Measuring *its own* rect against the
  // window's current size was comparing a value to itself, an unconditional
  // no-op; the window never actually shrank. Fixed by measuring the last
  // *visible* direct child's bottom edge instead (real content extent),
  // skipping ones hidden via the `hidden` attribute (e.g.
  // `#start-listening-reason-note` when there's no blocking reason) rather
  // than blindly taking `lastElementChild`, which could be a zero-size
  // hidden node.
  function lastVisibleChild(container: HTMLElement): HTMLElement | null {
    for (let i = container.children.length - 1; i >= 0; i--) {
      const el = container.children[i] as HTMLElement;
      if (!el.hidden) return el;
    }
    return null;
  }

  async function resizeSetupWindowToContent() {
    if (document.body.dataset.mode !== "setup") return;
    const last = lastVisibleChild(setupView);
    if (!last) return;
    const setupTop = setupView.getBoundingClientRect().top;
    const lastBottom = last.getBoundingClientRect().bottom;
    // `.view`'s own bottom padding isn't included in the last child's rect.
    const paddingBottom = parseFloat(getComputedStyle(setupView).paddingBottom) || 0;
    const contentHeight = Math.ceil(lastBottom - setupTop + paddingBottom);

    const win = getCurrentWindow();
    const [scaleFactor, current, outer, outerPos, monitor] = await Promise.all([
      win.scaleFactor(),
      win.innerSize(),
      win.outerSize(),
      win.outerPosition(),
      currentMonitor(),
    ]);
    const currentLogicalHeight = current.height / scaleFactor;
    const currentLogicalWidth = current.width / scaleFactor;

    // Never grow past the bottom of the monitor the window is currently on —
    // #setup-view already has `overflow-y: auto` (styles.css), so capping
    // the target height here just lets that existing scrollbar take over for
    // whatever doesn't fit, instead of pushing the window (and "Start
    // listening" with it) off the bottom of the screen with no way back
    // short of manually dragging the title bar (found live-testing on
    // Windows 2026-08-23, docs/TODO.md). Tauri doesn't expose the
    // taskbar-excluded work area, only the full monitor resolution, so this
    // can still end up partly under a docked taskbar — strictly better than
    // being fully unreachable off-screen.
    //
    // `setSize` sets the *inner* (content) size, but the monitor bound has
    // to hold the *outer* window (title bar included) — chromeHeight below
    // is that title-bar/border overhead, measured rather than guessed since
    // it varies with OS theming. A small extra margin absorbs the couple of
    // pixels a paragraph can re-wrap by once the clamp itself introduces the
    // scrollbar (narrower content width from the scrollbar can push a
    // borderline line of text onto an extra line).
    let targetHeight = contentHeight;
    if (monitor) {
      const chromeHeightLogical = (outer.height - current.height) / scaleFactor;
      const reflowMargin = 24;
      const monitorBottomLogical = (monitor.position.y + monitor.size.height) / scaleFactor;
      const topLogical = outerPos.y / scaleFactor;
      const maxInnerHeight =
        monitorBottomLogical - topLogical - chromeHeightLogical - reflowMargin;
      targetHeight = Math.min(contentHeight, maxInnerHeight);
    }

    if (Math.abs(targetHeight - currentLogicalHeight) < 2) return;
    try {
      await win.setSize(new LogicalSize(currentLogicalWidth, targetHeight));
    } catch (err) {
      console.error("failed to resize setup window to content", err);
    }
  }

  document
    .querySelectorAll<HTMLDetailsElement>("#setup-view > details.field-group")
    .forEach((details) => {
      details.addEventListener("toggle", () => void resizeSetupWindowToContent());
    });
  // Correct the size once on startup too — every accordion starts closed
  // (no `open` attribute in index.html, nothing here opens one by default),
  // so a fresh launch needs this exactly as much as a later manual toggle.
  void resizeSetupWindowToContent();

  collapseAllButton.addEventListener("click", () => {
    document
      .querySelectorAll<HTMLDetailsElement>("#setup-view > details.field-group")
      .forEach((details) => {
        details.open = false;
      });
    // Each individual "toggle" event above already fires and resizes once
    // per section as `.open = false` is set here — no separate call needed.
  });
  permissionBackButton.addEventListener("click", () => {
    void returnToSetup();
  });

  permissionRetryButton.addEventListener("click", () => {
    void beginListening();
  });

  openSystemSettingsButton.addEventListener("click", () => {
    void openUrl(MACOS_SCREEN_RECORDING_SETTINGS_URL);
  });

  const hintFeed = document.querySelector<HTMLElement>("#hint-feed")!;
  // Captured before anything is ever appended — Start Over (see
  // showHideButton/startOverButton wiring above) restores exactly this,
  // including the "See a coding problem on screen?" tip block.
  const hintFeedEmptyHtml = hintFeed.innerHTML;

  // A coding problem submitted by the user, from any of the input sources in
  // docs/PRODUCT-BRIEF.md §5 (pasted text, pasted image, region screenshot).
  // Kept as one abstraction on purpose so a future input source can be added
  // without reworking the pipeline.
  type ProblemInput =
    | { kind: "text"; text: string }
    | { kind: "image"; dataUri: string }
    // Transcribed speech — the only source the user didn't explicitly submit,
    // so it's labelled as "heard" to make clear where it came from.
    | { kind: "heard"; text: string };

  function addProblemToFeed(input: ProblemInput) {
    hintFeed.querySelector(".hint-feed-empty")?.remove();

    const item = document.createElement("div");
    item.className = "feed-item";

    const header = document.createElement("div");
    header.className = "feed-item-header";

    const label = document.createElement("span");
    label.className = "feed-item-label";
    // Deliberately neutral for user-submitted input: nothing here has checked
    // that it actually is a coding problem — that judgement lives in the
    // model's off-topic guard (see the system prompt in llm.rs). Claiming
    // "problem submitted" up front would assert something unverified.
    label.textContent =
      input.kind === "image"
        ? "Screenshot added"
        : input.kind === "heard"
          ? "Heard"
          : "Text added";
    header.appendChild(label);


    // A mis-paste during a live interview needs a fast, obvious undo — both
    // to clear the clutter and to keep the wrong thing out of the context
    // sent with the next question.
    const removeButton = document.createElement("button");
    removeButton.className = "feed-item-remove";
    removeButton.type = "button";
    removeButton.title = "Remove";
    removeButton.textContent = "✕";
    removeButton.addEventListener("click", () => {
      item.remove();
    });
    header.appendChild(removeButton);

    item.appendChild(header);

    let heardEl: HTMLElement | null = null;
    if (input.kind === "image") {
      // No thumbnail — removed 2026-09-16 (owner: showing the user's own
      // screen back to them adds no information, they already see it).
      // The "Screenshot added" label above is enough context; the image
      // itself still goes to the LLM via `input.dataUri`, this only drops
      // the in-feed preview.
    } else if (input.kind === "heard") {
      const p = document.createElement("p");
      p.className = "feed-item-heard";
      // Starts as the raw (interview-language) transcript; swapped for a
      // Russian translation by the "translated-question" listener below once
      // it arrives, folded into the same answer call (see llm.rs
      // TRANSLATION_MARKER) — the interview itself stays in its own language
      // end to end (CLAUDE.md "Язык"), this is display only. Ported from
      // interview-app (owner decision 2026-10-01).
      p.textContent = `“${input.text}”`;
      item.appendChild(p);
      activeHeardEl = p;
      heardEl = p;
    } else {
      const pre = document.createElement("pre");
      pre.className = "feed-item-text";
      pre.textContent = input.text;
      item.appendChild(pre);
    }

    // Where the streamed answer lands, directly under the input it answers.
    const answer = document.createElement("div");
    answer.className = "feed-item-answer";
    answer.textContent = "Thinking…";
    item.appendChild(answer);

    hintFeed.appendChild(item);
    hintFeed.scrollTop = hintFeed.scrollHeight;

    void requestAnswer(input, answer);
    if (heardEl && input.kind === "heard") void shortenHeard(heardEl, input.text);
  }

  // Long heard questions (a monologue before the question, an implicit ask) are
  // shown shortened in the panel (owner idea 2026-10-12). Display only: the
  // answer model gets the full text. A separate Haiku session works in
  // PARALLEL to the answer, so the first word is not delayed (measured, see
  // TODO.md). Click on the text toggles short/full. Any failure — the full
  // text simply stays.
  const SUMMARY_MIN_WORDS = 20;
  async function shortenHeard(el: HTMLElement, full: string) {
    if (full.trim().split(/\s+/).length < SUMMARY_MIN_WORDS) return;
    const t0 = performance.now();
    try {
      const short = (await invoke<string>("summarize_question", { text: full })).trim();
      if (!short || !el.isConnected) return;
      console.debug(`[summary] ${Math.round(performance.now() - t0)} ms, ${full.split(/\s+/).length} -> ${short.split(/\s+/).length} words`);
      const shortText = `“${short}”`;
      const fullText = `“${full}”`;
      let showingShort = true;
      el.textContent = shortText;
      el.title = "Click: full question / short version";
      el.style.cursor = "pointer";
      el.addEventListener("click", () => {
        showingShort = !showingShort;
        el.textContent = showingShort ? shortText : fullText;
      });
    } catch (err) {
      console.debug("[summary] failed, full text stays:", err);
    }
  }

  // Only one answer streams at a time — the Rust side emits deltas globally,
  // so this points at whichever element should receive them.
  let activeAnswerEl: HTMLElement | null = null;
  let activeAnswerText = "";
  // The prompt ends an answer that has no matching fact in the base with `[[GAP]]`
  // (see llm.rs); hide it, including a half-streamed `[[GA`, and log the gap.
  const stripGapMarker = (t: string) => t.replace(/\s*\[\[?G?A?P?\]?\]?$/, "");
  // Same one-at-a-time assumption as activeAnswerEl, for the
  // "translated-question" event (only set for input.kind === "heard").
  // Ported from interview-app (owner decision 2026-10-01).
  let activeHeardEl: HTMLElement | null = null;
  let activeQuestion = "";
  let activeIsImage = false;
  let activeStartedAt = 0;
  let activeFirstWordMs: number | null = null;
  let interviewFile: string | null = null;
  let interviewHeader = "";

  const pad2 = (n: number) => String(n).padStart(2, "0");

  // The first thing heard in a session (utterance or bot question) fixes the
  // file name, so the transcript and the bot log share one name.
  function ensureInterviewFile(now: Date, time: string) {
    if (interviewFile) return;
    const company = companySelect.value.trim();
    const day = `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`;
    const slug = company.toLowerCase().replace(/[^a-z0-9а-яё]+/gi, "-").replace(/^-|-$/g, "") || "no-company";
    interviewFile = `${day}-${pad2(now.getHours())}${pad2(now.getMinutes())}-${slug}`;
    interviewHeader = `${company || "без карточки компании"} — ${day} ${time}`;
  }

  // Fire-and-forget: a file write must never slow the hint down.
  function saveTranscriptLine(text: string) {
    const now = new Date();
    const time = `${pad2(now.getHours())}:${pad2(now.getMinutes())}`;
    ensureInterviewFile(now, time);
    void invoke("log_transcript_line", { file: interviewFile, header: interviewHeader, time, text }).catch((err) =>
      console.error("log_transcript_line failed", err),
    );
  }

  function saveInterviewEntry(question: string, answer: string, firstWordMs: number | null) {
    const now = new Date();
    const time = `${pad2(now.getHours())}:${pad2(now.getMinutes())}`;
    ensureInterviewFile(now, time);
    void invoke("log_interview_entry", {
      file: interviewFile,
      header: interviewHeader,
      time,
      question,
      answer,
      firstWordMs,
    }).catch((err) => console.error("log_interview_entry failed", err));
  }

  // Stall fillers: if the first word hasn't arrived FILLER_DELAY_MS after the
  // question was sent, show a neutral stock phrase above the answer so the
  // owner has something to say. Shuffle-bag per language: no phrase repeats
  // until the whole deck has been used, and the deck resets on a new interview.
  const FILLER_DELAY_MS = 2000;
  const FILLERS: Record<string, string[]> = {
    English: [
      "That's a great question. Let me take a second to structure my thoughts.",
      "Let me think about that for a second.",
      "Good one, let me put my answer together.",
      "Let me walk you through how I see it.",
      "Give me a moment, I want to answer this properly.",
      "Let me think of the best way to explain this.",
      "That's worth a careful answer, one second.",
      "Let me gather my thoughts on that.",
      "Okay, let me think about how to put this clearly.",
      "Let me recall the details so I give you an accurate answer.",
      "Let me think about which example fits best here.",
      "Interesting, let me think about where to start.",
      "Let me take a moment to answer that well.",
      "Let me think about it from a practical side.",
      "I want to give you a clear answer, so let me think for a moment.",
    ],
    Russian: [
      "Хороший вопрос, дайте секунду, соберусь с мыслями.",
      "Минутку, подумаю, как лучше объяснить.",
      "Давайте я сначала соберу мысли.",
      "Секунду, хочу ответить по делу.",
      "Дайте мне момент, чтобы ответить чётко.",
      "Хм, интересный вопрос, подумаю, с чего начать.",
      "Сейчас вспомню детали, чтобы ответить точно.",
      "Давайте я объясню по порядку, только соберусь.",
      "Одну секунду, подберу подходящий пример.",
      "Подумаю, как лучше это сформулировать.",
      "Секунду, хочу ответить аккуратно.",
      "Давайте подумаю, как это выглядит на практике.",
    ],
  };
  const fillerBags: Record<string, string[]> = {};
  let fillerTimer: number | null = null;

  function nextFiller(language: string): string | null {
    const deck = FILLERS[language] ?? (language === "Russian" ? null : FILLERS.English);
    if (!deck) return null;
    let bag = fillerBags[language];
    if (!bag || bag.length === 0) {
      bag = [...deck];
      for (let i = bag.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [bag[i], bag[j]] = [bag[j], bag[i]];
      }
      fillerBags[language] = bag;
    }
    return bag.pop() ?? null;
  }

  function clearFillerTimer() {
    if (fillerTimer !== null) {
      window.clearTimeout(fillerTimer);
      fillerTimer = null;
    }
  }

  function scheduleFiller(target: HTMLElement) {
    clearFillerTimer();
    fillerTimer = window.setTimeout(() => {
      fillerTimer = null;
      // Answer (or error) already arrived, or a newer question took over.
      if (activeAnswerEl !== target || activeFirstWordMs !== null || target.classList.contains("feed-item-answer-error")) return;
      const phrase = nextFiller(currentResponseLanguage());
      if (!phrase) return;
      const el = document.createElement("div");
      el.className = "feed-item-filler";
      el.textContent = phrase;
      target.before(el);
      hintFeed.scrollTop = hintFeed.scrollHeight;
      requestAnimationFrame(() => void invoke("nudge_repaint"));
    }, FILLER_DELAY_MS);
  }

  // Last transcribed lines, so a screenshot is answered in the context of what
  // the interviewer just said (owner request 2026-11-02): "what is wrong with
  // this architecture?" + a diagram on screen. Only fresh lines are used.
  const recentHeard: { text: string; at: number }[] = [];
  const SCREENSHOT_SPEECH_WINDOW_MS = 60_000;
  function speechBeforeScreenshot(): string {
    const cutoff = Date.now() - SCREENSHOT_SPEECH_WINDOW_MS;
    return recentHeard
      .filter((h) => h.at >= cutoff)
      .slice(-3)
      .map((h) => h.text)
      .join(" ")
      .trim();
  }

  async function requestAnswer(input: ProblemInput, target: HTMLElement) {
    activeAnswerEl = target;
    activeAnswerText = "";
    activeQuestion = input.kind === "image" ? "" : input.text;
    activeIsImage = input.kind === "image";
    activeStartedAt = performance.now();
    activeFirstWordMs = null;
    target.textContent = "Thinking…";
    target.classList.remove("feed-item-answer-error");
    if (input.kind !== "image") scheduleFiller(target);

    try {
      await invoke("generate_answer", {
        request: {
          model: modelSelect.value,
          interviewContext: interviewContext.value,
          company: companySelect.value,
          profile: saved.profile,
          responseLanguage: currentResponseLanguage(),
          text:
            input.kind === "image"
              ? speechBeforeScreenshot()
                ? `[Interviewer said just before this screenshot: "${speechBeforeScreenshot()}"]`
                : ""
              : input.text,
          imageDataUri: input.kind === "image" ? input.dataUri : null,
          // Off by owner decision 2026-10-04: the TRANSLATED_Q line the model writes
          // before the answer delayed the first word by 0.7-1.4 s on long questions
          // (A/B measured, see TODO.md). Rust/prompt support is kept; set to
          // `input.kind === "heard" && currentResponseLanguage() !== "Russian"` to bring it back.
          translateQuestion: false,
        },
      });
    } catch (err) {
      // The Rust side also emits llm-error; only show this if nothing arrived,
      // so a mid-stream failure doesn't wipe a partial answer.
      clearFillerTimer();
      if (activeAnswerText.length === 0) {
        target.textContent = String(err);
        target.classList.add("feed-item-answer-error");
        // Rust also emits llm-error; nudge a repaint for this synchronous path too
        // (see nudge_repaint in llm.rs).
        requestAnimationFrame(() => void invoke("nudge_repaint"));
      }
    }
  }

  // The nudge_repaint() invoke inside each rAF callback works around a real
  // macOS repaint bug — see nudge_repaint's doc comment in llm.rs and
  // docs/TODO-DONE.md "repaint-on-inactive-window" for the full
  // investigation. Firing it immediately from the Rust side (right after
  // emitting these same events) was tried first and was inconsistent; rAF
  // ensures WebKit has actually painted the DOM write before the nudge
  // fires, which is what made it reliable.
  void listen<string>("llm-delta", (event) => {
    if (!activeAnswerEl) return;
    clearFillerTimer();
    if (activeFirstWordMs === null) activeFirstWordMs = Math.round(performance.now() - activeStartedAt);
    activeAnswerText += event.payload;
    activeAnswerEl.innerHTML = renderAnswerHtml(stripGapMarker(activeAnswerText));
    hintFeed.scrollTop = hintFeed.scrollHeight;
    requestAnimationFrame(() => void invoke("nudge_repaint"));
  });

  // Replaces the raw transcript already shown with a Russian translation (see
  // llm.rs TRANSLATION_MARKER) — only fires when the question was sent with
  // translateQuestion: true. Ported from interview-app (owner decision
  // 2026-10-01).
  void listen<string>("translated-question", (event) => {
    if (activeHeardEl) {
      activeHeardEl.textContent = `“${event.payload}”`;
    }
  });

  void listen<string>("llm-error", (event) => {
    clearFillerTimer();
    if (!activeAnswerEl) return;
    if (activeAnswerText.length === 0) {
      activeAnswerEl.textContent = event.payload;
      activeAnswerEl.classList.add("feed-item-answer-error");
    } else {
      // A failure that lands after some of the answer already streamed
      // (e.g. the connection dropped mid-answer) must never erase what's
      // already on screen — the candidate may already be reading or typing
      // from it. Shown as a small separate line below instead
      // (docs/PRODUCT-BRIEF.md scenario 5).
      const note = document.createElement("p");
      note.className = "feed-item-answer-error-note";
      note.textContent = event.payload;
      activeAnswerEl.appendChild(note);
    }
    activeAnswerEl = null;
    requestAnimationFrame(() => void invoke("nudge_repaint"));
  });

  void listen("llm-done", () => {
    clearFillerTimer();
    if (activeAnswerEl && activeAnswerText.length === 0) {
      activeAnswerEl.textContent = "(empty response)";
    }
    // A "no data" reply means the knowledge base has a gap: remember the question.
    const head = activeAnswerText.trimStart().slice(0, 60).toLowerCase();
    const hasGapMarker = /\[\[GAP\]\]/.test(activeAnswerText);
    if (activeAnswerEl && hasGapMarker) {
      activeAnswerEl.innerHTML = renderAnswerHtml(stripGapMarker(activeAnswerText));
    }
    if (activeQuestion && (hasGapMarker || head.startsWith("no data") || head.startsWith("no company card") || head.startsWith("нет данных"))) {
      void invoke("log_gap", { question: activeQuestion });
    }
    if (activeAnswerText.length > 0) {
      saveInterviewEntry(activeIsImage ? "[скриншот]" : activeQuestion, stripGapMarker(activeAnswerText), activeFirstWordMs);
    }
    activeAnswerEl = null;
    requestAnimationFrame(() => void invoke("nudge_repaint"));
  });

  let capturing = false;

  // Region-select capture commented out, not deleted — docs/TODO.md
  // 2026-08-16, "Убрать region-select скриншот". Kept intact (including the
  // Rust-side capture_screen_region command, still registered) so it's
  // cheap to bring back if this decision reverses.
  //
  // async function captureRegionScreenshot() {
  //   // The OS tool is modal; a second concurrent invocation would fight the
  //   // first over the window hide/show and the clipboard.
  //   if (capturing) return;
  //   capturing = true;
  //   try {
  //     const dataUri = await invoke<string | null>("capture_screen_region");
  //     // null means the user pressed Esc instead of selecting a region.
  //     if (dataUri) {
  //       addProblemToFeed({ kind: "image", dataUri });
  //     }
  //   } catch (err) {
  //     console.error("region screenshot failed", err);
  //   } finally {
  //     capturing = false;
  //   }
  // }
  //
  // addScreenshotButton.addEventListener("click", () => {
  //   void captureRegionScreenshot();
  // });

  async function captureFullScreenshot() {
    // Shares the `capturing` guard with the region variant — both ultimately
    // run the OS's modal screenshot tool on the Rust side, so only one
    // should ever be in flight at a time.
    if (capturing) return;
    capturing = true;
    try {
      const dataUri = await invoke<string | null>("capture_full_screen");
      if (dataUri) {
        addProblemToFeed({ kind: "image", dataUri });
      }
    } catch (err) {
      console.error("full-screen screenshot failed", err);
    } finally {
      capturing = false;
    }
  }

  quickScreenshotButton.addEventListener("click", () => {
    void captureFullScreenshot();
  });

  // The Screenshot *hotkey* (still keyed "region_screenshot_*" internally —
  // see the settings.json-compat comment on the checkbox in index.html) runs
  // the capture entirely on the Rust side (it has to work even when this
  // window isn't focused), so its result arrives as an event instead of an
  // invoke() return value — same underlying capture_full_screen call as the
  // button, just triggered and awaited in Rust rather than from here. Event
  // names themselves were renamed from "region-screenshot-*" to
  // "screenshot-*" (docs/TODO.md, 2026-08-16) since they're purely internal
  // IPC channel names, not persisted anywhere — unlike the Rust command/
  // settings-field names, renaming these carries no settings.json risk.
  void listen<string>("screenshot-captured", (event) => {
    addProblemToFeed({ kind: "image", dataUri: event.payload });
  });
  void listen<string>("screenshot-error", (event) => {
    console.error("screenshot hotkey failed", event.payload);
  });

  // Source-agnostic paste: whatever put an image on the clipboard (macOS's
  // own tool, Lightshot, Snagit, ShareX, …) pastes the same way, per
  // docs/PRODUCT-BRIEF.md §5. Text pastes are accepted too, for platforms
  // that allow copying the problem statement.
  document.addEventListener("paste", (event) => {
    if (document.body.dataset.mode !== "compact") return;

    const clipboard = event.clipboardData;
    if (!clipboard) return;

    const imageItem = Array.from(clipboard.items).find((item) =>
      item.type.startsWith("image/"),
    );
    if (imageItem) {
      const file = imageItem.getAsFile();
      if (file) {
        event.preventDefault();
        const reader = new FileReader();
        reader.onload = () => {
          addProblemToFeed({ kind: "image", dataUri: String(reader.result) });
        };
        reader.readAsDataURL(file);
        return;
      }
    }

    const text = clipboard.getData("text/plain").trim();
    if (text) {
      event.preventDefault();
      addProblemToFeed({ kind: "text", text });
    }
  });

  void listen<string>("audio-status", async (event) => {
    const status = event.payload;
    if (status === "ready") {
      try {
        await invoke("enter_listening_mode", {
          excludeFromCapture: captureExclusionCheckbox.checked,
        });
        statusText.textContent = "Listening";
        setMode("compact");
      } catch (err) {
        console.error("failed to enter listening mode", err);
      }
      // Reset regardless of outcome: on success so the button reads normally
      // again when the user comes back to setup, on failure so they aren't
      // left with a permanently disabled "Starting…" button.
      setStarting(false);
    } else if (status === "permission_denied") {
      setStarting(false);
      showPermissionScreen("permission_denied");
    } else if (status === "unsupported_os") {
      setStarting(false);
      showPermissionScreen("unsupported_os");
    } else if (status.startsWith("error:")) {
      setStarting(false);
      showPermissionScreen("error", status.slice("error:".length));
    }
  });

  void listen<number>("audio-level", (event) => {
    // event.payload is an RMS value on a roughly 0-0.3 practical scale for
    // speech; clamp/scale it into a 0-100% meter width.
    const pct = Math.min(100, Math.max(0, event.payload * 100 * 3));
    audioMeterFill.style.width = `${pct}%`;
  });

  void listen<number>("audio-segment", () => {
    segmentCount += 1;
    segmentCountText.textContent = `${segmentCount} speech segment${segmentCount === 1 ? "" : "s"}`;
  });

  // Transcribed speech. Only questions trigger an automatic hint — answering
  // every utterance would burn the user's Claude usage on the interviewer's
  // small talk and bury the hint that matters under noise. Non-questions are
  // still shown, so the user can see the app is hearing correctly and can
  // ask for an answer on anything by clicking it.
  void listen<{ text: string; isQuestion: boolean }>("transcript", (event) => {
    const { text, isQuestion } = event.payload;
    recentHeard.push({ text, at: Date.now() });
    if (recentHeard.length > 10) recentHeard.shift();
    if (isQuestion) {
      addProblemToFeed({ kind: "heard", text });
      saveTranscriptLine(text);
      return;
    }
    saveTranscriptLine(text);

    hintFeed.querySelector(".hint-feed-empty")?.remove();
    const heard = document.createElement("p");
    heard.className = "feed-item-heard";
    heard.textContent = `“${text}”`;
    heard.title = "Click to answer this";
    heard.style.cursor = "pointer";
    heard.addEventListener("click", () => {
      heard.remove();
      addProblemToFeed({ kind: "heard", text });
    });
    hintFeed.appendChild(heard);
    hintFeed.scrollTop = hintFeed.scrollHeight;
  });

  void listen<string>("transcript-error", (event) => {
    console.error("transcription failed", event.payload);
    statusText.textContent = "Transcription error";
  });
});
