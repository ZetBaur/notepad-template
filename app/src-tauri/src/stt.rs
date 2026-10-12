// Local speech-to-text over the segments the VAD produces.
//
// Everything runs on-device: the model file lives in the app's data
// directory and audio never leaves the machine, which is what makes the
// privacy claim in docs/PRODUCT-BRIEF.md §6 true rather than aspirational.
//
// The model is downloaded on first use with progress reporting rather than
// bundled in the archive (explicitly allowed by brief §3). Bundling even the
// smallest useful multilingual model would add hundreds of megabytes to a
// download whose conversion rate is the whole business — all traffic is
// organic, so a scared-off download can't be re-bought.

use std::path::PathBuf;
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Multilingual models only — `.en` variants would be faster, but the STT
/// language setting in brief §5 has to be able to reach ~90 languages.
const MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// The size that ships inside the app bundle. Chosen over the smaller ones
/// because a mangled transcript produces a wrong hint, and technical
/// interview vocabulary ("idempotent", "sharding", "Kubernetes") is exactly
/// where the smaller models degrade.
const BUNDLED_MODEL: &str = "small";

#[derive(Default)]
pub struct SttState {
    context: Mutex<Option<WhisperContext>>,
    loaded_model: Mutex<Option<String>>,
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
}

// rename_all is required here: unlike #[tauri::command] arguments (which
// Tauri auto-converts to camelCase), an event payload struct is serialized
// by plain serde with no such magic — without this, the field ships as
// `is_question` and the frontend's `event.payload.isQuestion` silently reads
// undefined, so every transcript looks like a non-question.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub text: String,
    /// True when the text looks like a question directed at the user, which
    /// is what triggers an automatic hint (brief §1, step 3).
    pub is_question: bool,
}

fn model_filename(model: &str) -> Result<String, String> {
    match model {
        "tiny" | "base" | "small" | "medium" => Ok(format!("ggml-{model}.bin")),
        other => Err(format!("unknown whisper model: {other}")),
    }
}

/// Where a downloaded model lives. The bundled model is not here — see
/// `bundled_model_path`.
fn downloaded_model_path(app: &AppHandle, model: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve the app data directory: {e}"))?
        .join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(model_filename(model)?))
}

/// The model that ships inside the app bundle, so the default configuration
/// works offline the moment the app is unzipped. Only one size is bundled;
/// the others are opt-in downloads.
fn bundled_model_path(app: &AppHandle, model: &str) -> Option<PathBuf> {
    if model != BUNDLED_MODEL {
        return None;
    }
    let path = app
        .path()
        .resolve(
            format!("resources/{}", model_filename(model).ok()?),
            tauri::path::BaseDirectory::Resource,
        )
        .ok()?;
    path.exists().then_some(path)
}

/// Resolves a model to a usable file, preferring the bundled copy.
fn resolve_model(app: &AppHandle, model: &str) -> Result<PathBuf, String> {
    if let Some(path) = bundled_model_path(app, model) {
        return Ok(path);
    }
    let downloaded = downloaded_model_path(app, model)?;
    if downloaded.exists() {
        return Ok(downloaded);
    }
    Err(format!("model '{model}' is not available yet"))
}

/// True when the model can be used right now — either bundled or already
/// downloaded.
#[tauri::command]
pub fn is_model_downloaded(app: AppHandle, model: String) -> Result<bool, String> {
    Ok(resolve_model(&app, &model).is_ok())
}

/// True when this model ships with the app and never needs downloading.
#[tauri::command]
pub fn is_model_bundled(app: AppHandle, model: String) -> bool {
    bundled_model_path(&app, &model).is_some()
}

/// Downloads the model with progress events. Writes to a temporary path and
/// renames on completion, so an interrupted download can't leave a truncated
/// file that later fails to load with a confusing error.
#[tauri::command]
pub async fn download_model(app: AppHandle, model: String) -> Result<(), String> {
    if resolve_model(&app, &model).is_ok() {
        return Ok(()); // bundled, or already downloaded
    }
    let path = downloaded_model_path(&app, &model)?;

    let url = format!("{MODEL_BASE_URL}/{}", model_filename(&model)?);
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("could not start the model download: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "model download failed with HTTP {}",
            response.status()
        ));
    }

    let total = response.content_length().unwrap_or(0);
    let temp_path = path.with_extension("partial");
    let mut file = std::fs::File::create(&temp_path).map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("model download interrupted: {e}"))?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let _ = app.emit("model-download-progress", DownloadProgress { downloaded, total });
    }

    drop(file);
    std::fs::rename(&temp_path, &path).map_err(|e| e.to_string())?;
    let _ = app.emit("model-download-done", ());
    Ok(())
}

/// Loads the model into memory once and keeps it there — reloading per
/// utterance would add seconds to every hint.
pub fn ensure_model_loaded(app: &AppHandle, model: &str) -> Result<(), String> {
    let state = app.state::<SttState>();

    {
        let loaded = state.loaded_model.lock().unwrap();
        if loaded.as_deref() == Some(model) && state.context.lock().unwrap().is_some() {
            return Ok(());
        }
    }

    let path = resolve_model(app, model)?;

    let context = WhisperContext::new_with_params(
        path.to_str().ok_or("model path is not valid UTF-8")?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("could not load the speech model: {e}"))?;

    // Must come *after* the context above exists: loading the model is what
    // creates ggml's Metal device, and its broken teardown handler is only
    // registered at that moment. atexit runs handlers LIFO, so registering
    // ours here — and not at startup — is what puts us in front of it. See
    // lib.rs `install_exit_guard` for the crash this prevents.
    crate::install_exit_guard();

    *state.context.lock().unwrap() = Some(context);
    *state.loaded_model.lock().unwrap() = Some(model.to_string());
    Ok(())
}

/// Cheap heuristic for "the interviewer just asked something". Whisper
/// punctuates, so a `?` anywhere in the segment is the strongest signal — not
/// just trailing, since a leading filler or a second sentence can push the
/// mark off the end. Beyond that: WH-questions, yes/no questions, imperative
/// coding-task prompts ("Reverse this array"), and short bare follow-ups
/// ("Complexity?" said without the mark, "Edge cases") all show up in real
/// interviews without a literal "?". Scenario setups ("Suppose you have...")
/// are the one category this can't catch — that needs actual language
/// understanding, not a word list. This only decides whether to spend an API
/// call automatically — the user can always click to answer manually, so a
/// false negative costs less than firing on every sentence.
pub(crate) fn looks_like_question(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.contains('?') {
        return true;
    }
    let lowered = trimmed.to_lowercase();

    // Greetings and small talk often lead a segment ("Hello, tell me about
    // yourself."), so every clause is checked on its own, not only the start.
    if lowered
        .split(['.', '!', ',', ';', '—'])
        .any(|clause| clause_opens_like_question(clause))
    {
        return true;
    }

    // Short bare follow-ups asked without a "?": "Complexity?", "Edge cases",
    // "Any trade-offs". Bounded to a handful of words and checked on the whole
    // segment only, so "complexity" inside a long unrelated explanation (or one
    // of its clauses) doesn't fire.
    let rest = strip_fillers(&lowered);
    if rest.split_whitespace().count() <= 5 {
        const FOLLOWUP_TOPICS: [&str; 6] = [
            "complexity", "edge case", "trade-off", "tradeoff", "assumption", "scale",
        ];
        if FOLLOWUP_TOPICS.iter().any(|topic| rest.contains(topic)) {
            return true;
        }
    }

    false
}

/// Strips leading filler words, repeatedly — "so, um, what is..." should still
/// hit the opener check after both are gone.
fn strip_fillers(lowered: &str) -> &str {
    const FILLERS: [&str; 16] = [
        "so", "um", "uh", "okay", "alright", "now", "hello", "hi", "hey", "well", "great",
        "thanks", "thank you", "good", "nice", "ладно",
    ];
    let mut rest = lowered.trim();
    while let Some(stripped) = FILLERS
        .iter()
        .find_map(|filler| rest.strip_prefix(filler).filter(|r| r.is_empty() || !r.starts_with(|c: char| c.is_alphanumeric())))
    {
        rest = stripped.trim_start_matches(|c: char| c == ',' || c == '.' || c.is_whitespace());
    }
    rest
}

fn clause_opens_like_question(clause: &str) -> bool {
    let rest = strip_fillers(clause);

    const OPENERS: [&str; 30] = [
        "what", "why", "how", "when", "where", "which", "who", "can you", "could you",
        "would you", "tell me", "tell us", "walk me", "walk us", "explain", "describe",
        "give me", "share", "talk me through", "talk about", "i'd like to hear",
        "i would like to hear", "let's talk about", "let's start with", "i want to hear",
        "i'm curious", "i'd be curious", "i am curious", "i was wondering", "i wonder",
    ];
    // Same idea in Russian, for interviews held in Russian.
    const RU_OPENERS: [&str; 26] = [
        "расскажите", "расскажи", "почему", "зачем", "как ", "что ", "какой", "какая",
        "какие", "опишите", "приведите", "поделитесь", "объясните", "назовите",
        "когда", "где", "кто", "сколько", "куда", "какое", "какую", "можете", "готовы ли",
        "есть ли", "были ли", "чем ",
    ];
    // Yes/no questions rarely lead with a WH-word — two-word phrases only,
    // a bare "is"/"does"/"should" is too ambiguous with ordinary statements.
    const YES_NO_OPENERS: [&str; 13] = [
        "is there", "are there", "do you", "does this", "does it", "would this",
        "would that", "should we", "should i", "will this", "did you", "have you",
        "has this",
    ];
    // Coding-interview task prompts are often imperative, not interrogative.
    const TASK_OPENERS: [&str; 24] = [
        "implement", "write a", "write an", "design a", "design an", "given a",
        "given an", "reverse", "sort", "find", "merge", "remove", "delete", "insert",
        "return", "print", "build", "create", "optimize", "solve", "calculate",
        "count", "check", "traverse",
    ];

    OPENERS
        .iter()
        .chain(RU_OPENERS.iter())
        .chain(YES_NO_OPENERS.iter())
        .chain(TASK_OPENERS.iter())
        .any(|opener| rest.starts_with(opener))
}

/// Runs one VAD segment through whisper. Called from a blocking context —
/// inference is CPU/GPU-bound and must not block the async runtime.
pub fn transcribe_segment(
    app: &AppHandle,
    samples: &[f32],
    language: &str,
) -> Result<Option<Transcript>, String> {
    let state = app.state::<SttState>();
    let guard = state.context.lock().unwrap();
    let context = guard
        .as_ref()
        .ok_or_else(|| "speech model is not loaded".to_string())?;

    let mut session = context
        .create_state()
        .map_err(|e| format!("could not start transcription: {e}"))?;

    // Greedy sampling: beam search is more accurate but materially slower,
    // and a hint that lands after the moment has passed is worthless.
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    // whisper.cpp's own default is a hardcoded 4 threads regardless of the
    // machine — on CPU-only builds (no Metal/CUDA) that leaves most cores
    // idle during inference. Use up to 8: more helps throughput less past
    // that on whisper.cpp's encoder/decoder split, and it leaves headroom for
    // whatever else is running instead of saturating every logical core.
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8) as i32;
    params.set_n_threads(n_threads);
    // "auto" lets whisper detect per segment; anything else pins the language.
    if language != "auto" {
        params.set_language(Some(language));
    }

    // Logged deliberately: this pipeline had zero visibility before
    // 2026-07-27, which is why a ~25s-per-segment CPU-only regression (fixed
    // by the Vulkan backend below) took this long to pin down. Doesn't touch
    // transcript content, just timing.
    let t0 = std::time::Instant::now();
    session
        .full(params, samples)
        .map_err(|e| format!("transcription failed: {e}"))?;
    eprintln!(
        "stt: transcribed {:.1}s of audio in {:?}",
        samples.len() as f64 / 16000.0,
        t0.elapsed()
    );

    // Whisper occasionally hallucinates a plausible-sounding but fabricated
    // sentence on a quiet/ambiguous segment instead of returning nothing —
    // unlike the [BLANK_AUDIO]-style markers below, this text isn't bracketed
    // and would otherwise reach the UI as if it were really heard. Per-segment
    // no_speech_probability is whisper.cpp's own signal for this; 0.6 is
    // whisper.cpp's own documented default threshold for it (see
    // whisper_rs::FullParams::set_no_speech_thold), not a value invented here.
    const NO_SPEECH_PROB_THRESHOLD: f32 = 0.6;
    let segment_count = session.full_n_segments();
    let mut text = String::new();
    for i in 0..segment_count {
        if let Some(segment) = session.get_segment(i) {
            if segment.no_speech_probability() > NO_SPEECH_PROB_THRESHOLD {
                continue;
            }
            if let Ok(chunk) = segment.to_str() {
                text.push_str(chunk);
            }
        }
    }

    let text = text.trim().to_string();
    // Whisper emits bracketed/parenthesized markers for non-speech sound —
    // [BLANK_AUDIO], [MUSIC], (chimes), (typing), etc. — depending on model
    // and version. Those aren't utterances and shouldn't reach the UI or
    // count as something to answer.
    let is_non_speech_marker = (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'));
    if text.is_empty() || is_non_speech_marker {
        return Ok(None);
    }

    let is_question = looks_like_question(&text);
    Ok(Some(Transcript { text, is_question }))
}

#[cfg(test)]
mod question_tests {
    use super::looks_like_question;

    #[test]
    fn greeting_before_a_request_still_counts() {
        assert!(looks_like_question("Hello, tell me about yourself."));
        assert!(looks_like_question("Thanks. Walk me through your resume."));
        assert!(looks_like_question("Great, so why do you want to work here."));
    }

    #[test]
    fn behavioral_and_russian_prompts_count() {
        assert!(looks_like_question("Give me an example of a conflict."));
        assert!(looks_like_question("Расскажите о себе."));
        assert!(looks_like_question("Хорошо, почему вы хотите уйти с текущей работы."));
        assert!(looks_like_question("Когда вы можете приступить к работе."));
        assert!(looks_like_question("Сколько лет у вас опыта."));
    }

    #[test]
    fn small_talk_and_long_explanations_do_not_count() {
        assert!(!looks_like_question("Hello, nice to meet you."));
        assert!(!looks_like_question("The complexity of this approach, well, it grows quickly with the input size."));
        assert!(!looks_like_question("Thanks for joining us today."));
    }

    #[test]
    fn existing_behaviour_is_kept() {
        assert!(looks_like_question("What is a hash map"));
        assert!(looks_like_question("Complexity"));
        assert!(looks_like_question("Is this right?"));
    }

    #[test]
    fn indirect_curiosity_phrasing_counts() {
        assert!(looks_like_question("I'd be curious about your salary expectations."));
        assert!(looks_like_question("I was wondering about your notice period."));
        assert!(looks_like_question("Walk us through a recent project."));
    }
}
