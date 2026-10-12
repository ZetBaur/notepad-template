// Windows system-audio capture via WASAPI loopback — the counterpart to the
// macOS CoreAudio process tap in native/macos/audiotap.swift.
//
// Unlike macOS, this needs no sidecar process. The macOS helper exists because
// the Process Tap API is Swift-only with no usable Rust bindings; WASAPI is
// plain COM and is reachable from this process directly, so there is no second
// executable to build, bundle or code-sign on Windows (which is also why
// tauri.windows.conf.json overrides `externalBin` to an empty list).
//
// Two WASAPI details drive the shape of this file:
//
//   * **Loopback is a render endpoint opened for capture.** There is no
//     "what's playing" device to enumerate — you take the default *playback*
//     device and initialise its client in the Capture direction, which is what
//     sets AUDCLNT_STREAMFLAGS_LOOPBACK. Nothing here asks for a microphone.
//
//   * **Polling, not event-driven.** The event handle is only signalled while
//     the render endpoint is actually pulling buffers; on an idle endpoint it
//     never fires, so an event-driven loop would block forever the moment the
//     call went quiet. Polling also gives us somewhere to synthesise the
//     silence the endpoint stops delivering — see the idle branch in
//     capture_loop.
//
// Windows needs no permission for loopback capture, so unlike the macOS path
// there is no permission_denied status to report here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use wasapi::{
    deinitialize, initialize_mta, Direction, SampleType, StreamMode, WaveFormat,
};

use crate::audio::Vad;

/// What whisper requires, and — thanks to `autoconvert` below — what we ask
/// the audio engine to hand us directly. The shared-mode engine resamples and
/// downmixes from whatever the device mix format happens to be (typically
/// 48 kHz stereo), so this file contains no resampling code of its own and the
/// byte stream reaching `Vad` is bit-for-bit the same shape the macOS sidecar
/// produces: little-endian f32, mono, 16 kHz.
const TARGET_RATE: usize = 16000;
const TARGET_CHANNELS: usize = 1;
const TARGET_BITS: usize = 32;
const BYTES_PER_FRAME: usize = TARGET_CHANNELS * TARGET_BITS / 8;

/// Requested engine buffer, in 100 ns units (200 ms). Only an upper bound on
/// how much audio may queue up before it is dropped — generous here because a
/// whisper inference on another thread can briefly starve this loop, and
/// losing a chunk of the interviewer's question is far worse than the few
/// hundred kilobytes this costs.
const BUFFER_DURATION_HNS: i64 = 2_000_000;

/// Starts the capture thread. It runs until `stop` is set; the handle is kept
/// by AudioState so stop_audio_capture_sync can join it.
pub fn spawn_capture(app: AppHandle, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        // COM has to be initialised on the thread that talks to it, so this
        // belongs here rather than in the app's setup hook.
        if let Err(e) = initialize_mta().ok() {
            eprintln!("wasapi: COM initialisation failed: {e}");
            let _ = app.emit("audio-status", format!("error:{e}"));
            return;
        }

        let result = capture_loop(&app, &stop);
        deinitialize();

        if let Err(e) = result {
            eprintln!("wasapi: capture failed: {e}");
            let _ = app.emit("audio-status", format!("error:{e}"));
        }
    })
}

fn capture_loop(app: &AppHandle, stop: &AtomicBool) -> Result<(), String> {
    let enumerator = wasapi::DeviceEnumerator::new()
        .map_err(|e| format!("could not open the audio device list: {e}"))?;

    // Direction::Render — the device whose *output* we want to listen to.
    let device = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| format!("no default playback device to capture from: {e}"))?;

    let mut client = device
        .get_iaudioclient()
        .map_err(|e| format!("could not open the audio client: {e}"))?;

    let format = WaveFormat::new(
        TARGET_BITS,
        TARGET_BITS,
        &SampleType::Float,
        TARGET_RATE,
        TARGET_CHANNELS,
        None,
    );

    let (default_period, _min_period) = client
        .get_device_period()
        .map_err(|e| format!("could not read the device period: {e}"))?;

    // Direction::Capture on a render device is what turns this into a loopback
    // stream. `autoconvert` is what lets us ask for 16 kHz mono instead of
    // having to resample the device mix format ourselves.
    client
        .initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: BUFFER_DURATION_HNS,
            },
        )
        .map_err(|e| format!("could not start loopback capture: {e}"))?;

    let capture = client
        .get_audiocaptureclient()
        .map_err(|e| format!("could not open the capture client: {e}"))?;

    client
        .start_stream()
        .map_err(|e| format!("could not start the audio stream: {e}"))?;

    let _ = app.emit("audio-status", "ready");

    // Poll at half the device period so a packet is picked up in the same
    // period it appeared, with a floor so a very small period can't turn this
    // into a busy loop.
    let poll_interval = Duration::from_nanos((default_period as u64).saturating_mul(100) / 2)
        .clamp(Duration::from_millis(5), Duration::from_millis(20));

    let mut vad = Vad::new();
    let mut idle_since: Option<Instant> = None;

    while !stop.load(Ordering::SeqCst) {
        let frames = capture
            .get_next_packet_size()
            .map_err(|e| format!("audio capture stopped: {e}"))?
            .unwrap_or(0);

        if frames == 0 {
            // An idle render endpoint delivers *nothing* — not even buffers
            // flagged silent. The VAD only advances its silence counter on
            // data it receives, so without this it would never close out a
            // segment that ended just as the other side stopped talking: the
            // utterance would sit in the buffer until the next one arrived
            // and the two would be transcribed as one. Feeding it the silence
            // that really elapsed keeps segmentation identical to the macOS
            // path, where the tap keeps delivering zeroes.
            let now = Instant::now();
            if let Some(since) = idle_since.replace(now) {
                let elapsed = now.duration_since(since).as_secs_f64();
                let samples = (elapsed * TARGET_RATE as f64) as usize;
                if samples > 0 {
                    vad.push(app, &vec![0u8; samples * BYTES_PER_FRAME]);
                }
            }
            std::thread::sleep(poll_interval);
            continue;
        }
        idle_since = None;

        let mut buffer = vec![0u8; frames as usize * BYTES_PER_FRAME];
        let (frames_read, info) = capture
            .read_from_device(&mut buffer)
            .map_err(|e| format!("audio capture stopped: {e}"))?;

        // With AUDCLNT_BUFFERFLAGS_SILENT the buffer's contents are undefined
        // rather than zeroed — the engine is telling us "treat this as
        // silence", not "here is silence". Feeding the raw bytes through would
        // hand the VAD whatever was left in that memory.
        if info.flags.silent {
            buffer.fill(0);
        }

        buffer.truncate(frames_read as usize * BYTES_PER_FRAME);
        vad.push(app, &buffer);
    }

    let _ = client.stop_stream();
    Ok(())
}
