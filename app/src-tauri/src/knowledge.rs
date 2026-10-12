// The candidate's personal knowledge base: `profiles/<profile>/bot/knowledge.md`
// (git-ignored, never bundled), one file per profile, plus the company cards in
// `profiles/companies/`. Read once and cached, so a question never
// pays for disk access and the system prompt stays byte-identical between
// requests (a changed prompt means a new session, ~2s). `reload_knowledge`
// drops the cache after the files were edited.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Everything the bot knows about the candidate for one profile lives in this
/// single file; the other files in `<profile>/` (bot/reference, linkedin, resume, analytics) and
/// the shared `bot/FORBIDDEN.md` / `bot/GAPS.md` never reach the prompt.
const KNOWLEDGE_FILE: &str = "knowledge.md";

/// Where the personal files live: `NOTEPAD_PROFILES` if set, otherwise the
/// app data folder (so the project folder can be moved freely), falling back to
/// `<repo>/profiles` for a fresh checkout.
fn profiles_dir() -> PathBuf {
    if let Ok(p) = std::env::var("NOTEPAD_PROFILES") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let app_data = if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library/Application Support/dev.notepad.assistant/profiles"))
    } else {
        std::env::var("APPDATA")
            .ok()
            .map(|a| PathBuf::from(a).join("dev.notepad.assistant/profiles"))
    };
    match app_data {
        Some(dir) if dir.is_dir() => dir,
        _ => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles")),
    }
}

fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.contains("..")
}

/// Words the bot must not say about the candidate's own experience, per profile:
/// `profiles/<profile>/bot/BANNED-WORDS.txt`, one word or phrase per line (`#` = comment).
/// `None` = the profile has no such file (the built-in default list applies); an existing
/// but empty file means "no banned words".
pub fn banned_words(profile: &str) -> Option<Vec<String>> {
    if !is_safe_name(profile) {
        return None;
    }
    let text = std::fs::read_to_string(profiles_dir().join(profile).join("bot").join("BANNED-WORDS.txt")).ok()?;
    Some(
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect(),
    )
}

/// Profile names = folders in `profiles/` that hold a `bot/knowledge.md`.
fn profile_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().join("bot").join(KNOWLEDGE_FILE).is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    names
}

/// Empty profile name = the first profile on disk.
fn read_profile(dir: &Path, profile: &str) -> String {
    let name = if profile.is_empty() {
        profile_names(dir).into_iter().next().unwrap_or_default()
    } else {
        profile.to_string()
    };
    if !is_safe_name(&name) {
        return String::new();
    }
    std::fs::read_to_string(dir.join(&name).join("bot").join(KNOWLEDGE_FILE))
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

fn read_company(dir: &Path, company: &str) -> Option<String> {
    if !is_safe_name(company) {
        return None;
    }
    let text = std::fs::read_to_string(dir.join("companies").join(format!("{company}.md"))).ok()?;
    // The recruiter marker only feeds the dropdown label, not the prompt.
    let text: String = text.lines().filter(|l| recruiter_marker(l).is_none()).collect::<Vec<_>>().join("\n");
    Some(format!("=== COMPANY CARD: {company} ===\n{}", text.trim()))
}

fn build(dir: &Path, profile: &str, company: &str) -> String {
    let mut out = read_profile(dir, profile);
    if let Some(card) = read_company(dir, company) {
        out.push_str("\n\n");
        out.push_str(&card);
    }
    out
}

/// The chosen profile's knowledge plus the chosen company card (empty string = none).
pub fn load(profile: &str, company: &str) -> String {
    let (profile, company) = (profile.trim(), company.trim());
    let mut guard = cache().lock().unwrap();
    guard
        .entry(format!("{profile}\0{company}"))
        .or_insert_with(|| build(&profiles_dir(), profile, company))
        .clone()
}

#[tauri::command]
pub fn list_profiles() -> Vec<String> {
    profile_names(&profiles_dir())
}

/// `<!-- recruiter: Name -->` line in a company card.
fn recruiter_marker(line: &str) -> Option<&str> {
    let name = line.trim().strip_prefix("<!-- recruiter:")?.strip_suffix("-->")?.trim();
    (!name.is_empty()).then_some(name)
}

#[derive(serde::Serialize)]
pub struct CompanyItem {
    /// File stem, used to load the card.
    name: String,
    /// Dropdown text: "Recruiter - Company", or just the company without a marker.
    label: String,
}

#[tauri::command]
pub fn list_companies() -> Vec<CompanyItem> {
    let dir = profiles_dir().join("companies");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "md"))
                .then(|| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
                .flatten()
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(format!("{name}.md"))).unwrap_or_default();
            let label = match text.lines().find_map(recruiter_marker) {
                Some(r) => format!("{r} \u{2014} {name}"),
                None => name.clone(),
            };
            CompanyItem { name, label }
        })
        .collect()
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Appends a question the knowledge base could not answer to `GAPS.md`, to be
/// brought to Claude after the call so the base can be extended.
/// Shared by all profiles: `bot/GAPS.md`.
#[tauri::command]
pub fn log_gap(question: String) -> Result<(), String> {
    use std::io::Write;
    let question = question.trim().replace('\n', " ");
    if question.is_empty() {
        return Ok(());
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(profiles_dir().join("bot").join("GAPS.md"))
        .map_err(|e| e.to_string())?;
    writeln!(file, "- [{y:04}-{m:02}-{d:02}] {question}").map_err(|e| e.to_string())
}

/// Appends one question and the bot's answer to this interview's log in
/// `interviews/<file>.md` (created with a header on first write). Only bot
/// questions and answers go here; raw heard speech goes to `transcripts/`.
fn append_interview_entry(
    dir: &Path,
    file: &str,
    header: &str,
    time: &str,
    question: &str,
    answer: &str,
    first_word_ms: Option<u32>,
) -> Result<(), String> {
    use std::io::Write;
    if !is_safe_name(file) {
        return Err("bad interview file name".into());
    }
    let folder = dir.join("interviews");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("{file}.md"));
    let is_new = !path.exists();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    if is_new {
        writeln!(f, "# Интервью: {header}\n\n- Итог: (заполнить после звонка)\n").map_err(|e| e.to_string())?;
    }
    let timing = match first_word_ms {
        Some(ms) => format!(" (до первого слова: {ms} мс)"),
        None => String::new(),
    };
    writeln!(
        f,
        "## {time}{timing}\n\n**Вопрос:** {}\n\n**Ответ бота:**\n{}\n\n**Что сказал реально / оценка (ок / слабо / пробел):** \n",
        question.trim(),
        answer.trim()
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn log_interview_entry(
    file: String,
    header: String,
    time: String,
    question: String,
    answer: String,
    first_word_ms: Option<u32>,
) -> Result<(), String> {
    append_interview_entry(&profiles_dir(), &file, &header, &time, &question, &answer, first_word_ms)
}

/// Appends one heard utterance to `interviews/transcripts/<file>.md`. Kept in a
/// subfolder so `/after-call` (which lists `interviews/2026-*.md`) never picks
/// transcripts up by mistake. Only what the app heard (system audio, i.e. the
/// interviewer); the owner's own voice is not captured.
fn append_transcript_line(dir: &Path, file: &str, header: &str, time: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    if !is_safe_name(file) {
        return Err("bad interview file name".into());
    }
    let folder = dir.join("interviews").join("transcripts");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("{file}.md"));
    let is_new = !path.exists();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    if is_new {
        writeln!(f, "# Транскрипт (только речь интервьюера): {header}\n").map_err(|e| e.to_string())?;
    }
    writeln!(f, "- {time} {}", text.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn log_transcript_line(file: String, header: String, time: String, text: String) -> Result<(), String> {
    append_transcript_line(&profiles_dir(), &file, &header, &time, &text)
}

/// Runs the same FORBIDDEN.md check as `/eval` (eval/check-kb.mjs), reusing its
/// regexes so there's no risk of the Rust and JS checks drifting apart.
/// Empty string = clean or the check itself couldn't run (missing Node.js);
/// reload must never fail just because this bonus check did.
fn check_forbidden(dir: &Path) -> String {
    let node = crate::llm::find_node();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../agent-sidecar/eval/check-kb.mjs");
    let output = std::process::Command::new(&node).arg(script).env("NOTEPAD_PROFILES", dir).output();
    match output {
        Ok(out) if !out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

#[tauri::command]
pub fn reload_knowledge() -> String {
    cache().lock().unwrap().clear();
    check_forbidden(&profiles_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_profiles(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jh-kb-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("companies")).unwrap();
        dir
    }

    #[test]
    fn profile_loads_only_its_knowledge_file_and_the_company_card() {
        let dir = temp_profiles("a");
        let full = dir.join("fullstack/bot");
        std::fs::create_dir_all(full.join("reference")).unwrap();
        std::fs::write(full.join("knowledge.md"), "fullstack facts").unwrap();
        std::fs::write(full.join("claims.md"), "unverified claims").unwrap();
        std::fs::write(full.join("reference/old.md"), "old history").unwrap();
        std::fs::create_dir_all(dir.join("bot")).unwrap();
        std::fs::write(dir.join("bot/FORBIDDEN.md"), "rules").unwrap();
        std::fs::create_dir_all(dir.join("mobile/bot")).unwrap();
        std::fs::write(dir.join("mobile/bot/knowledge.md"), "mobile facts").unwrap();
        std::fs::write(dir.join("companies").join("acme.md"), "acme card").unwrap();
        let kb = build(&dir, "fullstack", "acme");
        assert!(kb.contains("fullstack facts") && kb.contains("acme card"));
        assert!(!kb.contains("mobile facts") && !kb.contains("unverified claims"));
        assert!(!kb.contains("old history") && !kb.contains("rules"));
        assert!(build(&dir, "mobile", "").contains("mobile facts"));
        // empty profile name = first one on disk
        assert!(build(&dir, "", "").contains("fullstack facts"));
        assert_eq!(profile_names(&dir), vec!["fullstack", "mobile"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_names_cannot_escape_the_bot_dir() {
        let dir = temp_profiles("p");
        std::fs::create_dir_all(dir.join("bot")).unwrap();
        std::fs::write(dir.join("secret.md"), "secret").unwrap();
        assert_eq!(read_profile(&dir, "../secret"), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn company_names_cannot_escape_the_companies_dir() {
        let dir = temp_profiles("b");
        std::fs::write(dir.join("secret.md"), "secret").unwrap();
        assert!(read_company(&dir, "../secret").is_none());
        assert!(read_company(&dir, "").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn banned_words_come_from_the_profile_file_only() {
        let dir = temp_profiles("banned");
        std::fs::create_dir_all(dir.join("a/bot")).unwrap();
        std::fs::write(dir.join("a/bot/BANNED-WORDS.txt"), "# c\nfoo\n\n bar \n").unwrap();
        std::fs::create_dir_all(dir.join("b/bot")).unwrap();
        std::fs::write(dir.join("b/bot/BANNED-WORDS.txt"), "# only a comment\n").unwrap();
        std::env::set_var("NOTEPAD_PROFILES", &dir);
        assert_eq!(banned_words("a"), Some(vec!["foo".to_string(), "bar".to_string()]));
        assert_eq!(banned_words("b"), Some(vec![]));
        assert_eq!(banned_words("missing"), None);
        assert_eq!(banned_words("../a"), None);
        std::env::remove_var("NOTEPAD_PROFILES");
    }

    #[test]
    fn civil_date_conversion_is_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_365), (2025, 10, 4));
        assert_eq!(civil_from_days(20_715), (2026, 9, 19));
    }

    #[test]
    fn interview_entries_append_to_one_file_with_one_header() {
        let dir = temp_profiles("interview");
        append_interview_entry(&dir, "2026-09-19-1030-acme", "Acme", "10:31", "Q1?", "A1", Some(1234)).unwrap();
        append_interview_entry(&dir, "2026-09-19-1030-acme", "Acme", "10:32", "Q2?", "A2", None).unwrap();
        let text = std::fs::read_to_string(dir.join("interviews/2026-09-19-1030-acme.md")).unwrap();
        assert_eq!(text.matches("# Интервью").count(), 1);
        assert!(text.contains("Q1?") && text.contains("A2"));
        assert!(text.contains("до первого слова: 1234 мс"));
        assert!(append_interview_entry(&dir, "../x", "h", "t", "q", "a", None).is_err());
        append_transcript_line(&dir, "2026-09-19-1030-acme", "Acme", "10:31", " hello ").unwrap();
        append_transcript_line(&dir, "2026-09-19-1030-acme", "Acme", "10:32", "bye").unwrap();
        let tr = std::fs::read_to_string(dir.join("interviews/transcripts/2026-09-19-1030-acme.md")).unwrap();
        assert!(tr.contains("- 10:31 hello") && tr.contains("- 10:32 bye"));
        assert!(append_transcript_line(&dir, "../x", "h", "t", "x").is_err());
    }

    #[test]
    fn missing_directory_gives_empty_knowledge() {
        assert_eq!(build(Path::new("/definitely/not/here"), "", ""), "");
    }
}
