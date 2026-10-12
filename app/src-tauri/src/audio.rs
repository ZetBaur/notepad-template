// Turns captured system audio into pause-delimited speech segments, using the
// same energy/silence VAD approach as the ~/dev/ipr/transcriber.py reference
// (adapted to an event-driven stream instead of a blocking read loop).
//
// The capture source is platform-specific, but deliberately produces the same
// stream in both cases — little-endian f32, mono, 16 kHz — so everything below
// `Vad` is shared:
//   * macOS spawns native/macos/audiotap.swift as a Tauri sidecar process and
//     reads its raw PCM stdout (the CoreAudio Process Tap API is Swift-only).
//   * Windows captures in-process via WASAPI loopback — see wasapi.rs.
//
// Alongside the live events the frontend shows ("listening", live level,
// segment count), each closed-out segment is handed to whisper off this
// thread — see transcribe_in_background.

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandChild;
#[cfg(target_os = "macos")]
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

/// PID of the running macOS capture sidecar, mirrored outside `AudioState` so
/// the process-wide exit guard in lib.rs can reach it. That guard runs from an
/// `atexit` handler, which has no `AppHandle` and must not take a lock that
/// another thread might already hold at exit time — a plain atomic is the only
/// safe thing to read there. 0 means "not running".
#[cfg(target_os = "macos")]
static SIDECAR_PID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Best-effort SIGTERM to the capture sidecar, called from the `atexit` guard.
/// The Swift side handles SIGTERM by releasing the CoreAudio tap and aggregate
/// device before exiting, so this is what keeps a menu-quit from leaving an
/// orphan holding the system-audio tap.
#[cfg(target_os = "macos")]
pub(crate) fn kill_sidecar_on_exit() {
    let pid = SIDECAR_PID.swap(0, std::sync::atomic::Ordering::SeqCst);
    if pid > 0 {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        const SIGTERM: i32 = 15;
        unsafe { kill(pid, SIGTERM) };
    }
}

const SAMPLE_RATE: f64 = 16000.0;
const SILENCE_RMS: f32 = 0.006;
const SILENCE_CUT_SEC: f64 = 0.7;
const MIN_SEGMENT_SEC: f64 = 1.0;
const MAX_SEGMENT_SEC: f64 = 20.0;
/// A segment cut by MAX_SEGMENT_SEC (not by a pause) is usually the middle of a
/// sentence; its text is held and glued to the next segment so the bot answers
/// the whole question once. If no speech follows within this much silence, the
/// held text is released on its own.
const HELD_FLUSH_SILENCE_SEC: f64 = 1.5;
const LEVEL_EMIT_INTERVAL_SEC: f64 = 0.1;

#[derive(Default)]
pub struct AudioState {
    /// macOS only: the running audiotap sidecar.
    child: Mutex<Option<CommandChild>>,
    /// Speech-recognition language for the current listening session, chosen
    /// in settings ("auto" or a whisper language code).
    language: Mutex<String>,
    /// Text of a segment cut by the length limit, waiting for the rest of the sentence.
    held: Mutex<Option<String>>,
    /// Windows only: the running WASAPI capture thread. There is no process to
    /// kill — capture runs in-process — so stopping is a flag plus a join
    /// rather than a `child.kill()`. Both halves live behind one lock so
    /// there is no order to get wrong between them.
    #[cfg(target_os = "windows")]
    capture: Mutex<Option<WindowsCapture>>,
}

#[cfg(target_os = "windows")]
struct WindowsCapture {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn rms(bytes: &[u8]) -> f32 {
    let n = bytes.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let mut sum_sq = 0.0f64;
    for i in 0..n {
        let b = [bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]];
        let v = f32::from_le_bytes(b) as f64;
        sum_sq += v * v;
    }
    ((sum_sq / n as f64).sqrt()) as f32
}

pub(crate) struct Vad {
    leftover: Vec<u8>,
    segment: Vec<u8>,
    silence_run_sec: f64,
    voiced: bool,
    last_level_emit: std::time::Instant,
}

impl Vad {
    pub(crate) fn new() -> Self {
        Self {
            leftover: Vec::new(),
            segment: Vec::new(),
            silence_run_sec: 0.0,
            voiced: false,
            last_level_emit: std::time::Instant::now(),
        }
    }

    /// Feeds one chunk of captured audio (little-endian f32, mono, 16 kHz),
    /// emitting a level update and, when a pause closes an utterance out, a
    /// segment for transcription. Chunk boundaries need not align to samples.
    pub(crate) fn push(&mut self, app: &AppHandle, chunk: &[u8]) {
        self.leftover.extend_from_slice(chunk);
        let usable_len = self.leftover.len() - (self.leftover.len() % 4);
        if usable_len == 0 {
            return;
        }
        let samples: Vec<u8> = self.leftover.drain(..usable_len).collect();

        let level = rms(&samples);
        let chunk_sec = (samples.len() / 4) as f64 / SAMPLE_RATE;

        if level >= SILENCE_RMS {
            self.voiced = true;
            self.silence_run_sec = 0.0;
        } else {
            self.silence_run_sec += chunk_sec;
        }

        if self.voiced {
            self.segment.extend_from_slice(&samples);
        }

        if self.last_level_emit.elapsed().as_secs_f64() >= LEVEL_EMIT_INTERVAL_SEC {
            let _ = app.emit("audio-level", level);
            self.last_level_emit = std::time::Instant::now();
        }

        if !self.voiced && self.silence_run_sec >= HELD_FLUSH_SILENCE_SEC {
            flush_held(app);
        }

        let seg_sec = (self.segment.len() / 4) as f64 / SAMPLE_RATE;
        let should_cut = self.voiced
            && ((self.silence_run_sec >= SILENCE_CUT_SEC && seg_sec >= MIN_SEGMENT_SEC)
                || seg_sec >= MAX_SEGMENT_SEC);

        if should_cut {
            let duration_ms = (seg_sec * 1000.0) as u64;
            let segment = std::mem::take(&mut self.segment);
            self.silence_run_sec = 0.0;
            self.voiced = false;
            let continued = seg_sec >= MAX_SEGMENT_SEC;
            let _ = app.emit("audio-segment", duration_ms);
            transcribe_in_background(app.clone(), segment, continued);
        }
    }
}

/// Hands a finished utterance to whisper off the audio thread. Inference is
/// GPU/CPU-bound and takes long enough that running it inline would stall the
/// capture loop and drop the next utterance.
fn transcribe_in_background(app: AppHandle, segment: Vec<u8>, continued: bool) {
    let language = {
        let state = app.state::<AudioState>();
        let lang = state.language.lock().unwrap();
        if lang.is_empty() { "auto".to_string() } else { lang.clone() }
    };

    tauri::async_runtime::spawn_blocking(move || {
        let samples = bytes_to_f32(&segment);
        match crate::stt::transcribe_segment(&app, &samples, &language) {
            Ok(Some(mut transcript)) => {
                let state = app.state::<AudioState>();
                let mut held = state.held.lock().unwrap();
                if let Some(prev) = held.take() {
                    transcript.text = format!("{prev} {}", transcript.text);
                    transcript.is_question = crate::stt::looks_like_question(&transcript.text);
                }
                if continued {
                    *held = Some(transcript.text);
                } else {
                    drop(held);
                    let _ = app.emit("transcript", transcript);
                }
            }
            Ok(None) => {
                // Silence after a length-limit cut: nothing to glue, release what was held.
                if !continued {
                    flush_held(&app);
                }
            }
            Err(e) => {
                eprintln!("transcription failed: {e}");
                let _ = app.emit("transcript-error", e);
            }
        }
    });
}

/// Emits the held text (see `HELD_FLUSH_SILENCE_SEC`) as a normal transcript.
fn flush_held(app: &AppHandle) {
    let text = app.state::<AudioState>().held.lock().unwrap().take();
    if let Some(text) = text {
        let is_question = crate::stt::looks_like_question(&text);
        let _ = app.emit("transcript", crate::stt::Transcript { text, is_question });
    }
}

/// Parses the `STATUS:` lines the macOS audiotap sidecar writes to stderr.
/// Windows has no equivalent — WASAPI capture reports its own state directly
/// from wasapi.rs, and needs no permission, so there is nothing to parse.
#[cfg(target_os = "macos")]
fn handle_status_line(app: &AppHandle, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if let Some(rest) = line.strip_prefix("STATUS:") {
        if rest.starts_with("ready") {
            eprintln!("audio: got STATUS:ready, emitting audio-status=ready");
            match app.emit("audio-status", "ready") {
                Ok(()) => eprintln!("audio: emit(audio-status, ready) succeeded"),
                Err(e) => eprintln!("audio: emit(audio-status, ready) FAILED: {e}"),
            }
        } else if rest.starts_with("unsupported_os") {
            eprintln!("audio: got STATUS:unsupported_os");
            let _ = app.emit("audio-status", "unsupported_os");
        } else if rest.starts_with("permission_denied") {
            eprintln!("audio: got STATUS:permission_denied");
            let _ = app.emit("audio-status", "permission_denied");
        } else if rest.starts_with("error") {
            eprintln!("audio: got STATUS:error: {rest}");
            let _ = app.emit("audio-status", format!("error:{rest}"));
        } else {
            eprintln!("audiotap: {line}");
        }
    } else {
        eprintln!("audiotap: {line}");
    }
}

// `async` + `spawn_blocking` rather than a synchronous `#[tauri::command]`: a
// sync command runs on Tauri's main thread, and `ensure_model_loaded` below
// is not quick. Measured on this Windows machine (CPU, no Metal) at
// 650-870ms — well past the ~500ms line docs/WINDOWS-BUILD.md sets for
// converting, the same class of stall (there, ~15s) that made "Start
// listening" freeze until secrets.rs got this same treatment. The blocking
// body has no `await` in it, so the move is mechanical: unchanged logic,
// just called from off the main thread.
#[tauri::command]
pub async fn start_audio_capture(app: AppHandle, model: String, language: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || start_audio_capture_blocking(app, model, language))
        .await
        .map_err(|e| e.to_string())?
}

fn start_audio_capture_blocking(app: AppHandle, model: String, language: String) -> Result<(), String> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (&app, &model, &language);
        return Err(
            "System audio capture isn't implemented for this OS yet (macOS and Windows only so far)."
                .to_string(),
        );
    }

    // NOTE: the two platform branches below deliberately repeat the model-load
    // and language lines instead of sharing one hoisted copy. Hoisting them
    // would restructure code macOS compiles, and this file can only be built
    // and tested on one platform at a time — so the macOS branch is kept
    // byte-identical to what was there before Windows capture existed, and a
    // Windows change cannot alter macOS behaviour even by accident. Two
    // duplicated lines are a cheap price for that.
    #[cfg(target_os = "windows")]
    {
        // Load the speech model before opening the capture stream: a failure
        // here should surface as a plain error, not as a stream of utterances
        // that silently never get transcribed.
        crate::stt::ensure_model_loaded(&app, &model)?;

        let state = app.state::<AudioState>();
        *state.language.lock().unwrap() = language;
        let mut capture = state.capture.lock().unwrap();

        if let Some(running) = capture.take() {
            if running.thread.is_finished() {
                // A previous run already ended — stopped, or failed with an
                // error the user has since seen. Clearing it lets "Start
                // listening" actually restart, instead of reporting a capture
                // that is no longer running as if it were live.
                let _ = running.thread.join();
            } else {
                *capture = Some(running);
                // Already capturing. "ready" is only ever emitted once, right
                // after the stream starts, so a caller that missed it (e.g.
                // the user pressed "Start listening" again) would otherwise
                // wait forever for a signal that isn't coming again.
                let _ = app.emit("audio-status", "ready");
                return Ok(());
            }
        }

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = crate::wasapi::spawn_capture(app.clone(), stop.clone());
        *capture = Some(WindowsCapture { stop, thread });
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        // Load the speech model before opening the audio tap: a failure here
        // should surface as a plain error, not as a stream of utterances that
        // silently never get transcribed.
        crate::stt::ensure_model_loaded(&app, &model)?;

        let state = app.state::<AudioState>();
        *state.language.lock().unwrap() = language;
        if state.child.lock().unwrap().is_some() {
            // Already running — the helper only ever sends STATUS:ready once,
            // right after it starts, so a caller that missed that first
            // event (e.g. the user clicked "Start listening" again) would
            // otherwise wait forever for a signal that's never coming again.
            let _ = app.emit("audio-status", "ready");
            return Ok(());
        }

        // Must run in raw mode: stdout is a continuous binary PCM stream, not
        // text. The plugin's default line-buffered reader scans for `\n`
        // (0x0A) bytes to split "lines" — a byte value that shows up by
        // chance inside arbitrary float samples, and *never* shows up during
        // silence (all-zero bytes), so the reader can block indefinitely
        // waiting for a "line" that isn't coming. That stalls the pipe,
        // which backs up the child's stdout writes, which was hanging the
        // helper before it ever reached its first real audio write. Our own
        // Stderr handling already does its own manual line-splitting on raw
        // chunks (see the loop below), so raw mode doesn't change that.
        let (mut rx, child) = app
            .shell()
            .sidecar("notepad-audiotap")
            .map_err(|e| e.to_string())?
            .set_raw_out(true)
            .spawn()
            .map_err(|e| e.to_string())?;

        SIDECAR_PID.store(child.pid() as i32, std::sync::atomic::Ordering::SeqCst);
        *state.child.lock().unwrap() = Some(child);

        let app_for_task = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut vad = Vad::new();
            let mut stderr_leftover: Vec<u8> = Vec::new();

            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => {
                        vad.push(&app_for_task, &bytes);
                    }
                    CommandEvent::Stderr(bytes) => {
                        stderr_leftover.extend_from_slice(&bytes);
                        while let Some(pos) = stderr_leftover.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = stderr_leftover.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line);
                            handle_status_line(&app_for_task, &line);
                        }
                    }
                    CommandEvent::Error(msg) => {
                        let _ = app_for_task.emit("audio-status", format!("error:{msg}"));
                    }
                    CommandEvent::Terminated(payload) => {
                        SIDECAR_PID.store(0, std::sync::atomic::Ordering::SeqCst);
                        let state = app_for_task.state::<AudioState>();
                        *state.child.lock().unwrap() = None;
                        if let Some(code) = payload.code {
                            if code != 0 {
                                eprintln!("audiotap exited with code {code}");
                            }
                        }
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }
}

/// For the setup screen's "Permissions" section (docs/TODO.md, added
/// 2026-07-30): lets the user grant system-audio access ahead of time,
/// instead of discovering the system prompt mid-way into "Start listening".
///
/// There is no public preflight API for this TCC entry (unlike Screen
/// Recording, see `permissions.rs`) — creating the tap *is* the check, and
/// is also what triggers the system prompt on an undetermined decision. So
/// this spawns the sidecar in a one-shot `--probe` mode (creates the tap,
/// tears it down immediately, never reads or streams any audio) and reports
/// whether that succeeded. Must only be called from an explicit user action,
/// same reasoning as everywhere else in this file: an automatic call here
/// would pop the system dialog with no warning, the exact problem this
/// section exists to avoid.
#[tauri::command]
pub async fn check_audio_permission(app: AppHandle) -> Result<bool, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &app;
        // WASAPI loopback needs no OS permission — nothing to probe.
        return Ok(true);
    }

    #[cfg(target_os = "macos")]
    {
        let output = app
            .shell()
            .sidecar("notepad-audiotap")
            .map_err(|e| e.to_string())?
            .args(["--probe"])
            .output()
            .await
            .map_err(|e| e.to_string())?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("STATUS:probe_ok") {
            Ok(true)
        } else if stderr.contains("STATUS:permission_denied") {
            Ok(false)
        } else if stderr.contains("STATUS:unsupported_os") {
            Err("System audio capture needs macOS 14.2 (Sonoma) or later.".to_string())
        } else {
            Err(format!("Couldn't check system audio access: {stderr}"))
        }
    }
}

#[tauri::command]
pub fn stop_audio_capture(app: AppHandle) -> Result<(), String> {
    stop_audio_capture_sync(&app)
}

pub fn stop_audio_capture_sync(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AudioState>();
    if let Some(child) = state.child.lock().unwrap().take() {
        #[cfg(target_os = "macos")]
        SIDECAR_PID.store(0, std::sync::atomic::Ordering::SeqCst);
        child.kill().map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        // Joining rather than detaching: the thread holds an open WASAPI
        // stream, and letting it outlive a "Stop" the user just pressed would
        // leave the app capturing system audio with nothing on screen saying
        // so. It notices the flag within one poll interval (≤20 ms).
        if let Some(running) = state.capture.lock().unwrap().take() {
            running
                .stop
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = running.thread.join();
        }
    }

    Ok(())
}
