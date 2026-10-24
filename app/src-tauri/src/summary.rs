// Short version of a long heard question for the panel (owner idea 2026-10-12).
//
// Display only: the main model still receives the full text. Runs in its own
// sidecar process (a second Haiku session with a tiny prompt) PARALLEL to the
// main answer, so the answer is not delayed. A line written by the main model
// before the answer (old TRANSLATED_Q) cost +0.5..1.7 s, measured 2026-10-04;
// the parallel variant was measured 2026-10-12 (see TODO.md): ~0.9 s for the
// whole short version, main first word unchanged within noise.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

const SUMMARY_MODEL: &str = "claude-haiku-4-5";
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(10);

// Measured 2026-10-12 (6 long monologues x 3 rounds, Haiku 4.5): the first, short
// prompt sometimes ANSWERED instead of shortening (say-test: 2 of 6). This one
// plus the `Transcript: "..." / Short:` wrapper gave a question in 18 of 18.
const SUMMARY_SYSTEM: &str = r#"You are a text shortener, not an interviewer's assistant. The user message is a transcript of what an interviewer said to a job candidate. Your only job: rewrite it as the short question the candidate has to answer.

Rules:
- Output 1-3 short sentences, max 30 words in total, in the language of the transcript.
- Write it as the interviewer's question or request ("How would you...", "Why did you...", "Tell me about..."), second person, never as a solution or advice.
- Keep the topic words the interviewer used. Do not add details, criteria or technologies they did not say.
- Drop the background story and filler.
- NEVER answer, solve, advise or list steps. Do not write imperatives like "Kill", "Design", "Use", "Add". If you are about to give a solution, write the question instead.
- No preamble, no quotes, no labels.

Examples:
Transcript: "We run a small online shop, about ten engineers. Last month our checkout page got really slow during a sale and we had to restart servers by hand. I'd like to know what you would do about it."
Short: How would you deal with a checkout page that gets slow during a sale?

Transcript: "So I see you worked at a bank and then at a startup. Walk me through that, I'm curious what you liked and didn't like."
Short: Why did you move from a bank to a startup, and what did you like and dislike?"#;

type Waiting = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>>;

struct Session {
    /// Set when replaced, so its reader thread doesn't fail requests of the next session.
    retired: Arc<AtomicBool>,
    child: Child,
    stdin: ChildStdin,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.retired.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
    }
}

#[derive(Default)]
pub struct SummaryState {
    session: Mutex<Option<Session>>,
    waiting: Waiting,
    next_id: AtomicU64,
}

fn read_events(stdout: std::process::ChildStdout, waiting: Waiting, retired: Arc<AtomicBool>) {
    let mut buffers: HashMap<u64, String> = HashMap::new();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = msg["id"].as_u64();
        match (msg["type"].as_str(), id) {
            (Some("delta"), Some(id)) => {
                if let Some(text) = msg["text"].as_str() {
                    buffers.entry(id).or_default().push_str(text);
                }
            }
            (Some("done"), Some(id)) => {
                let text = buffers.remove(&id).unwrap_or_default().trim().to_string();
                if let Some(tx) = waiting.lock().unwrap().remove(&id) {
                    let _ = tx.send(Ok(text));
                }
            }
            (Some("error"), Some(id)) => {
                buffers.remove(&id);
                let message = msg["message"].as_str().unwrap_or("summary error").to_string();
                if let Some(tx) = waiting.lock().unwrap().remove(&id) {
                    let _ = tx.send(Err(message));
                }
            }
            _ => {}
        }
    }
    if retired.load(Ordering::SeqCst) {
        return;
    }
    let stuck: Vec<_> = waiting.lock().unwrap().drain().collect();
    for (_, tx) in stuck {
        let _ = tx.send(Err("summary engine stopped".to_string()));
    }
}

fn spawn_session(waiting: &Waiting) -> Result<Session, String> {
    let node = crate::llm::find_node();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../agent-sidecar/sidecar.mjs");
    if !std::path::Path::new(script).exists() {
        return Err(format!("sidecar script not found at {script}"));
    }
    let mut command = Command::new(&node);
    if let Some(dir) = std::path::Path::new(&node).parent().filter(|d| !d.as_os_str().is_empty()) {
        let path = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{path}", dir.display()));
    }
    let mut child = command
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("could not start the summary engine: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin on summary process")?;
    let stdout = child.stdout.take().ok_or("no stdout on summary process")?;

    let retired = Arc::new(AtomicBool::new(false));
    let (waiting2, retired2) = (waiting.clone(), retired.clone());
    std::thread::spawn(move || read_events(stdout, waiting2, retired2));

    write_line(&mut stdin, &serde_json::json!({ "type": "start", "system": SUMMARY_SYSTEM, "model": SUMMARY_MODEL }))?;
    Ok(Session { retired, child, stdin })
}

fn write_line(stdin: &mut ChildStdin, value: &serde_json::Value) -> Result<(), String> {
    let mut line = value.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.flush())
        .map_err(|e| format!("summary process is not accepting input: {e}"))
}

fn ensure_session(state: &SummaryState) -> Result<(), String> {
    let mut guard = state.session.lock().unwrap();
    if let Some(session) = guard.as_mut() {
        if matches!(session.child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }
    *guard = None;
    *guard = Some(spawn_session(&state.waiting)?);
    Ok(())
}

/// Starts the summary session at app launch so the first long question doesn't pay the cold start.
pub fn prewarm(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<SummaryState>();
        if let Err(e) = ensure_session(&state) {
            eprintln!("summary prewarm failed: {e}");
        }
    });
}

/// Short version of a long question. An error only means "keep showing the full text".
#[tauri::command]
pub async fn summarize_question(state: tauri::State<'_, SummaryState>, text: String) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(String::new());
    }
    ensure_session(&state)?;
    let (tx, rx) = oneshot::channel();
    let id = state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    state.waiting.lock().unwrap().insert(id, tx);
    {
        let mut guard = state.session.lock().unwrap();
        let session = guard.as_mut().ok_or("summary engine is not running")?;
        let wrapped = format!("Transcript: \"{text}\"\nShort:");
        let ask = serde_json::json!({ "type": "ask", "id": id, "text": wrapped });
        if let Err(message) = write_line(&mut session.stdin, &ask) {
            state.waiting.lock().unwrap().remove(&id);
            return Err(message);
        }
    }
    let result = tokio::time::timeout(SUMMARY_TIMEOUT, rx).await;
    match result {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err("summary engine stopped".to_string()),
        Err(_) => {
            state.waiting.lock().unwrap().remove(&id);
            Err("summary timed out".to_string())
        }
    }
}
