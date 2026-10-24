// Window-mode + global-hotkey shell for SyntaxCue.
//
// The app is a single native window that transforms in place between two
// states instead of opening a second window or a browser tab — a browser
// tab can't react to a global hotkey while focus is in Zoom/Teams, which is
// why this whole app is a Tauri window rather than "open localhost in the
// default browser" (see docs/PRODUCT-BRIEF.md §5, "Живой экран во время
// звонка"). No audio/whisper/LLM wiring yet — this is the app shell only.

mod audio;
mod knowledge;
mod llm;
mod permissions;
mod screenshot;
mod settings;
mod stt;
mod summary;
#[cfg(target_os = "windows")]
mod wasapi;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// whisper.cpp's ggml Metal backend crashes the app on *every* clean exit: a
/// static C++ destructor for its Metal residency-set collection runs during
/// `exit()` and hits `ggml_abort()`. Confirmed from a real crash report —
/// `ggml_abort` <- `ggml_metal_rsets_free` <- `ggml_metal_device_free` <-
/// `~unique_ptr<ggml_metal_device>()` <- `__cxa_finalize_ranges` <- `exit`.
///
/// Doing the bypass in Tauri's `RunEvent::ExitRequested` is not enough, and
/// that was the first (failed) attempt: quitting from the macOS app menu goes
/// `NSMenuItem` -> `-[NSApplication terminate:]` -> `exit()` directly, so the
/// Tauri event never fires and the crash still happened. An `atexit` hook
/// catches every exit path instead, because `exit()` itself is what runs it.
///
/// Ordering is the whole trick: `atexit`/`__cxa_atexit` handlers run LIFO, and
/// ggml registers its destructor when the Metal device is first created (model
/// load). Registering *after* that point therefore puts us ahead of it — we
/// run first, and `_exit` ends the process without touching the remaining
/// handlers or any static destructor. Hence the call site in `stt.rs`
/// immediately after the model loads, not at startup.
///
/// macOS-only on purpose, and both halves of that matter for the Windows
/// build: the crash is in ggml's *Metal* backend (Windows builds run the CPU
/// / Vulkan path, which has no such destructor), and the cleanup it performs
/// targets the macOS capture sidecar, which does not exist on Windows —
/// WASAPI capture runs in-process. Leaving it enabled there would also break
/// the build outright, since `kill(2)` is POSIX and does not link on MSVC.
#[cfg(target_os = "macos")]
pub(crate) fn install_exit_guard() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        extern "C" fn on_exit() {
            // The sidecar is a separate process, so it would otherwise survive
            // a menu quit and keep holding the system-audio tap — which then
            // makes the *next* launch capture silence (see audiotap.swift).
            audio::kill_sidecar_on_exit();
            unsafe { _exit(0) }
        }
        extern "C" {
            fn atexit(cb: extern "C" fn()) -> i32;
        }
        unsafe { atexit(on_exit) };
    });
}

/// No-op off macOS — see the macOS version above for why there is nothing to
/// guard against here.
#[cfg(not(target_os = "macos"))]
pub(crate) fn install_exit_guard() {}

#[cfg(target_os = "macos")]
extern "C" {
    fn _exit(code: i32) -> !;
}

// All sizes are logical (points), not physical pixels — on a Retina display
// physical sizes would render the window at half the intended size.
const SETUP_SIZE: (f64, f64) = (480.0, 720.0);
const SETUP_MIN_SIZE: (f64, f64) = (420.0, 560.0);
// Width doubled from the original 360 (added 2026-07-30, docs/TODO.md) — the
// panel was cramped for anything past a one-line answer. Height untouched;
// the user is free to resize either dimension afterward, this is just the
// starting point.
const COMPACT_SIZE: (f64, f64) = (720.0, 460.0);
// The compact panel stays user-resizable — hint length varies a lot between
// a one-line answer and a code block, so a fixed height would be limiting.
// This floor just keeps it from being shrunk into uselessness.
const COMPACT_MIN_SIZE: (f64, f64) = (280.0, 200.0);

#[derive(Default)]
struct AppState {
    /// Mirrors the "Keep hint window out of screen share" setting (off by default).
    /// Re-applied every time the window enters compact/listening mode.
    capture_excluded: Mutex<bool>,
    /// Currently registered global show/hide shortcut. Fixed at ⌘B/Ctrl+B
    /// (docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md, 2026-09-15) — no longer
    /// user-changeable, so this only ever holds the one default combo. Kept
    /// as a slot (not a bare registered-or-not bool) because register_shortcut
    /// is shared with the other two fixed hotkeys below and needs somewhere
    /// to remember what it registered.
    visibility_shortcut: Mutex<Option<Shortcut>>,
    /// Currently registered screenshot shortcut. Fixed at ⌘H/Ctrl+H and
    /// always registered at startup as of 2026-09-15 — previously an
    /// optional, off-by-default, user-configurable "Region screenshot
    /// hotkey" setting (docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md). Still
    /// named `region_*` internally (settings.json/event-name compat
    /// reasoning predates this change and still applies to whatever's left
    /// referencing it).
    region_screenshot_shortcut: Mutex<Option<Shortcut>>,
    /// Currently registered "Start Over" shortcut (⌘R/Ctrl+R, fixed) — clears
    /// the hint feed back to its empty state. Added 2026-09-15,
    /// docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md.
    start_over_shortcut: Mutex<Option<Shortcut>>,
    /// Guards screenshot::capture_screen_region against overlapping calls —
    /// e.g. the region-screenshot hotkey firing again while the OS's
    /// interactive selector from the first press is still waiting. A second
    /// `screencapture -i -c` can't run alongside the first (macOS rejects it
    /// immediately), and without this guard that instant failure would still
    /// run the function's own "show the window again" step, flashing the
    /// hint window back into view before the user finished selecting.
    pub(crate) screenshot_capturing: AtomicBool,
    /// The compact panel's last user-resized size, remembered only for the
    /// current run (not persisted to disk). Without this, going back to
    /// setup and re-entering listening mode always snapped the panel back
    /// to COMPACT_SIZE, silently discarding a resize the user had just
    /// made — this makes enter_listening_mode restore it instead.
    compact_size: Mutex<Option<(f64, f64)>>,
    /// The compact panel's last position, remembered for the same reason and
    /// with the same lifetime as `compact_size` — without it, re-entering
    /// listening mode always snapped the panel back to the top-right corner,
    /// discarding wherever the user had dragged it.
    ///
    /// Stored in *physical* coordinates on purpose: they're global across
    /// monitors, so restoring and bounds-checking need no per-monitor scale
    /// conversion (each display can have its own scale factor).
    compact_position: Mutex<Option<(i32, i32)>>,
}

fn window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())
}

/// Whether the panel is currently faded out (see set_panel_hidden_via_alpha).
/// Needed because an alpha-0 window still reports `isVisible() == true`, so the
/// window itself can no longer be asked what state the toggle is in.
#[cfg(target_os = "macos")]
static PANEL_ALPHA_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Hides/reveals the compact panel *without* ordering the NSWindow out.
///
/// Ordering a window out and back in makes macOS re-evaluate which Space it
/// belongs to, and for a Regular (Dock-icon) app that has been activated, that
/// re-evaluation re-homes the window to the app's own Space and quietly drops
/// its all-Spaces membership: `collectionBehavior` still reads `0x1`
/// afterwards while `isOnActiveSpace` has gone false and stays there. The
/// panel then only appears on the Space SyntaxCue itself lives on — i.e. never
/// over the fullscreen video call, which is the whole point of the feature.
///
/// Measured, not guessed, and the alternatives were tried first and rejected
/// on evidence: re-asserting the collection behavior after ordering back in
/// (no effect), `orderFrontRegardless` instead of `show()` so the app is never
/// activated (no effect), and dropping the explicit `set_focus()` (no effect —
/// `makeKeyAndOrderFront:` activates the app on its own). Only never ordering
/// out at all holds. Fading to alpha 0 also stops mouse events reaching an
/// invisible window, and the window stays continuously ordered in, so the
/// membership is never re-evaluated in the first place.
///
/// Full investigation: docs/PRODUCT-BRIEF.md §5, "Панель не видна поверх
/// чужого полноэкранного (fullscreen) Space на macOS".
#[cfg(target_os = "macos")]
fn set_panel_hidden_via_alpha(win: &WebviewWindow, hidden: bool) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let Ok(ptr) = win.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    unsafe {
        let ns_window = ptr as *mut AnyObject;
        let alpha: f64 = if hidden { 0.0 } else { 1.0 };
        let _: () = msg_send![ns_window, setAlphaValue: alpha];
        let _: () = msg_send![ns_window, setIgnoresMouseEvents: hidden];
    }
}

/// For `screenshot.rs`'s self-exclusion during a full-screen/region capture
/// (docs/TODO.md "Полноэкранный скриншот может захватывать саму панель") —
/// must use the same alpha-fade as the hotkey toggle above, never
/// `WebviewWindow::hide()`/`show()`. A first version of that fix used plain
/// `hide()`/`show()` and reintroduced exactly the bug `set_panel_hidden_via_alpha`
/// exists to avoid (found live-testing 2026-08-21/22 — screenshot button alone,
/// no drag, no resize, reliably dropped cross-Space visibility afterward).
///
/// Returns whether the panel was *already* alpha-hidden (via the hotkey)
/// before this call — if so, does nothing, and the caller must not un-hide it
/// afterward either. Otherwise a screenshot taken while the user has
/// deliberately hidden the panel via hotkey would un-hide it as a side
/// effect, which is not this function's job to decide.
#[cfg(target_os = "macos")]
fn hide_panel_for_capture(win: &WebviewWindow) -> bool {
    let already_hidden = PANEL_ALPHA_HIDDEN.load(std::sync::atomic::Ordering::SeqCst);
    if !already_hidden {
        set_panel_hidden_via_alpha(win, true);
    }
    already_hidden
}

/// Pairs with `hide_panel_for_capture` — pass back exactly what it returned.
#[cfg(target_os = "macos")]
fn restore_panel_after_capture(win: &WebviewWindow, was_already_hidden: bool) {
    if !was_already_hidden {
        set_panel_hidden_via_alpha(win, false);
    }
}

/// Async, cross-thread-safe wrappers around the two functions above — for
/// `screenshot.rs`, whose `capture_full`/`capture_region` are `async fn`
/// Tauri commands and so run on a tokio worker thread, not the main thread.
/// `set_panel_hidden_via_alpha` calls raw AppKit (`objc2::msg_send!` on an
/// `NSWindow`), which crashes (`EXC_BREAKPOINT`, "Must only be used from the
/// main thread") if called off it — confirmed live 2026-08-22, the first
/// version of this fix called `hide_panel_for_capture`/
/// `restore_panel_after_capture` directly from the async command body.
/// `run_on_main_thread` only dispatches (fire-and-forget) — the `oneshot`
/// round-trip is what lets the caller actually wait for the fade to have
/// taken effect before proceeding to capture (or the clipboard read after
/// restoring), same reasoning as the existing SPACEDBG debug driver's use of
/// the same dispatch pattern above.
#[cfg(target_os = "macos")]
pub(crate) async fn hide_panel_for_capture_async(app: &AppHandle) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app2 = app.clone();
    if app
        .run_on_main_thread(move || {
            let hidden = window(&app2).map(|win| hide_panel_for_capture(&win)).unwrap_or(false);
            let _ = tx.send(hidden);
        })
        .is_err()
    {
        return false;
    }
    rx.await.unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub(crate) async fn restore_panel_after_capture_async(app: &AppHandle, was_already_hidden: bool) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app2 = app.clone();
    if app
        .run_on_main_thread(move || {
            if let Ok(win) = window(&app2) {
                restore_panel_after_capture(&win, was_already_hidden);
            }
            let _ = tx.send(());
        })
        .is_err()
    {
        return;
    }
    let _ = rx.await;
}

// ---------------------------------------------------------------------------
// TEMPORARY DEBUG INSTRUMENTATION (added 2026-08-18, docs/TODO.md "Регресс:
// панель снова теряет видимость поверх чужого fullscreen").
//
// Reads the window's *real* AppKit state rather than what our own code
// believes it set. The whole class of bug being chased here is "the flag
// still reads correct while WindowServer has silently stopped honouring it",
// so `collection_behavior` alone proves nothing — `is_on_active_space` is the
// value that actually goes false when the panel stops compositing over
// another Space. Remove this block (and its call sites, all tagged SPACEDBG)
// once the trigger is identified and fixed.
// ---------------------------------------------------------------------------

/// Snapshot of everything that matters for cross-Space compositing.
#[cfg(target_os = "macos")]
#[derive(PartialEq, Clone, Copy)]
struct SpaceState {
    collection_behavior: usize,
    is_on_active_space: bool,
    level: isize,
    sharing_type: usize,
    style_mask: usize,
    alpha: f64,
    visible: bool,
}

#[cfg(target_os = "macos")]
fn read_space_state(win: &WebviewWindow) -> Option<SpaceState> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let ptr = win.ns_window().ok()?;
    if ptr.is_null() {
        return None;
    }
    unsafe {
        let w = ptr as *mut AnyObject;
        Some(SpaceState {
            collection_behavior: msg_send![w, collectionBehavior],
            is_on_active_space: msg_send![w, isOnActiveSpace],
            level: msg_send![w, level],
            sharing_type: msg_send![w, sharingType],
            style_mask: msg_send![w, styleMask],
            alpha: msg_send![w, alphaValue],
            visible: msg_send![w, isVisible],
        })
    }
}

/// `CanJoinAllSpaces` is bit 0 of NSWindowCollectionBehavior — the single bit
/// this whole feature depends on.
#[cfg(target_os = "macos")]
const CAN_JOIN_ALL_SPACES: usize = 1 << 0;

#[cfg(target_os = "macos")]
fn log_space_state(win: &WebviewWindow, label: &str) {
    let Some(s) = read_space_state(win) else {
        eprintln!("SPACEDBG [{label}] <no ns_window>");
        return;
    };
    eprintln!(
        "SPACEDBG [{label}] canJoinAllSpaces={} behavior=0x{:x} onActiveSpace={} level={} sharing={} styleMask=0x{:x} alpha={:.2} visible={}",
        s.collection_behavior & CAN_JOIN_ALL_SPACES != 0,
        s.collection_behavior,
        s.is_on_active_space,
        s.level,
        s.sharing_type,
        s.style_mask,
        s.alpha,
        s.visible,
    );
}

#[cfg(not(target_os = "macos"))]
fn log_space_state(_win: &WebviewWindow, _label: &str) {}

/// Drives the candidate operations from a file so a repro can be scripted
/// step by step without UI clicks — writes one command per line into
/// `/tmp/spacedbg-cmd`, which this thread executes and then clears. Only
/// active when `SPACEDBG_AUTO=1`, so it cannot fire in a real build.
#[cfg(target_os = "macos")]
fn spawn_space_debug_driver(app: AppHandle) {
    if std::env::var("SPACEDBG_AUTO").as_deref() != Ok("1") {
        return;
    }
    std::thread::spawn(move || {
        let path = std::path::Path::new("/tmp/spacedbg-cmd");
        loop {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let Ok(cmd) = std::fs::read_to_string(path) else {
                continue;
            };
            let _ = std::fs::remove_file(path);
            let cmd = cmd.trim().to_string();
            if cmd.is_empty() {
                continue;
            }
            eprintln!("SPACEDBG >>> command: {cmd}");
            let app2 = app.clone();
            // Window work must happen on the main thread — AppKit rejects
            // most of it from a background thread, silently or otherwise.
            let _ = app.run_on_main_thread(move || {
                let Ok(win) = window(&app2) else { return };
                match cmd.as_str() {
                    "enter" => {
                        let _ = enter_listening_mode(app2.clone(), false);
                    }
                    "exit" => {
                        let _ = exit_listening_mode(app2.clone());
                    }
                    "protect-on" => {
                        let _ = set_capture_exclusion(app2.clone(), true);
                    }
                    "protect-off" => {
                        let _ = set_capture_exclusion(app2.clone(), false);
                    }
                    "resize-a" => {
                        apply("set_size", win.set_size(LogicalSize::new(520.0, 300.0)));
                        log_space_state(&win, "driver/after-resize-a");
                    }
                    "resize-b" => {
                        apply("set_size", win.set_size(LogicalSize::new(420.0, 220.0)));
                        log_space_state(&win, "driver/after-resize-b");
                    }
                    "move-a" => {
                        apply(
                            "set_position",
                            win.set_position(PhysicalPosition::new(200, 200)),
                        );
                        log_space_state(&win, "driver/after-move-a");
                    }
                    "toggle-hotkey" => {
                        let _ = toggle_window_visibility(app2.clone());
                    }
                    "state" => log_space_state(&win, "driver/state"),
                    other => eprintln!("SPACEDBG >>> unknown command: {other}"),
                }
            });
        }
    });
}

/// Polls the window state on a timer and logs only when something *changes*.
/// This is the half that catches a flip happening with none of our own code
/// running — if the state degrades between two ticks with no operation logged
/// in between, the trigger is the OS (Spaces/monitor/sleep), not this app.
#[cfg(target_os = "macos")]
fn spawn_space_state_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<SpaceState> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            let Ok(win) = window(&app) else { continue };
            let Some(now) = read_space_state(&win) else {
                continue;
            };
            if last.as_ref() != Some(&now) {
                log_space_state(&win, "watcher/changed");
                last = Some(now);
            }
        }
    });
}

/// True when `pos` (physical, the window's top-left) still lands inside one of
/// the connected displays. A remembered position can outlive the monitor it
/// was taken on — unplug an external screen and restoring it blindly would put
/// the panel somewhere unreachable, and in compact mode there's no title bar
/// to drag it back by.
fn position_on_some_monitor(win: &WebviewWindow, pos: (i32, i32)) -> bool {
    let Ok(monitors) = win.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let origin = m.position();
        let size = m.size();
        pos.0 >= origin.x
            && pos.1 >= origin.y
            && pos.0 < origin.x + size.width as i32
            && pos.1 < origin.y + size.height as i32
    })
}

/// Applies a window-property change without letting a single failing call
/// (e.g. `set_content_protected` on a platform/window-state combo that
/// rejects it) abort the rest of a mode transition. A half-applied window
/// transform (resized + undecorated but still showing the previous view,
/// because the JS side never got to call `setMode` after the `invoke()`
/// promise rejected) is worse than one property silently not taking effect —
/// it leaves the window stuck in an unmovable, unresizable, seemingly broken
/// state with no way back short of restarting the app.
fn apply<E: std::fmt::Display>(op: &str, result: Result<(), E>) {
    if let Err(e) = result {
        eprintln!("window transform: {op} failed: {e}");
    }
}

#[tauri::command]
fn enter_listening_mode(app: AppHandle, exclude_from_capture: bool) -> Result<(), String> {
    eprintln!("lib: enter_listening_mode called (exclude_from_capture={exclude_from_capture})");
    let win = window(&app)?;
    let state = app.state::<AppState>();
    *state.capture_excluded.lock().unwrap() = exclude_from_capture;

    // First real session ever — flips the "Send feedback" link on for good.
    // This command only runs once audio has actually reported ready (see
    // main.ts's "audio-status" listener), so it's a real usage signal, not
    // just the app being opened.
    let mut saved = settings::load_settings(app.clone());
    if !saved.has_used_app {
        saved.has_used_app = true;
        let _ = settings::save_settings(app.clone(), saved);
    }

    let (w, h) = state.compact_size.lock().unwrap().unwrap_or(COMPACT_SIZE);
    let (min_w, min_h) = COMPACT_MIN_SIZE;
    // Relax the min size *before* shrinking: the setup-mode minimum (from
    // tauri.conf.json) is larger than the compact size, so leaving it in
    // place would clamp the panel back up to setup dimensions.
    apply(
        "set_min_size",
        win.set_min_size(Some(LogicalSize::new(min_w, min_h))),
    );
    apply("set_resizable", win.set_resizable(true));
    apply("set_size", win.set_size(LogicalSize::new(w, h)));
    apply("set_decorations", win.set_decorations(false));
    apply("set_always_on_top", win.set_always_on_top(true));
    // always_on_top only raises the panel within its own Space — a call app
    // in real macOS fullscreen lives on a different Space entirely, where
    // the panel doesn't appear at all despite the global hotkey still
    // "working" (docs/PRODUCT-BRIEF.md §5, "Панель не видна поверх чужого
    // полноэкранного (fullscreen) Space на macOS"). Only meaningfully takes
    // effect on the *first* call, in practice — see the long comment in
    // exit_listening_mode on why this is never turned back off.
    log_space_state(&win, "enter/before-allWorkspaces");
    apply(
        "set_visible_on_all_workspaces",
        win.set_visible_on_all_workspaces(true),
    );
    log_space_state(&win, "enter/after-allWorkspaces");
    #[cfg(target_os = "macos")]
    // The panel is toggled by fading rather than ordering out (see
    // set_panel_hidden_via_alpha), so entering listening mode has to start
    // from a known-opaque window — otherwise a session left hidden by the
    // hotkey would come back invisible.
    #[cfg(target_os = "macos")]
    {
        set_panel_hidden_via_alpha(&win, false);
        PANEL_ALPHA_HIDDEN.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    apply(
        "set_content_protected",
        win.set_content_protected(exclude_from_capture),
    );
    log_space_state(&win, "enter/after-contentProtected");
    // Restore wherever the user last dragged the panel; fall back to
    // centering the screen only on the first entry of the run, or when that
    // spot no longer exists (monitor unplugged or resolution changed) — was a
    // corner placement (docs/TODO.md, 2026-07-30: always landed top-right at
    // launch, which read as an odd default rather than a deliberate one).
    let remembered = *state.compact_position.lock().unwrap();
    match remembered.filter(|p| position_on_some_monitor(&win, *p)) {
        Some((x, y)) => apply("set_position", win.set_position(PhysicalPosition::new(x, y))),
        None => apply("center", win.center()),
    }
    log_space_state(&win, "enter/after-position");
    // Changing the decorations/style mask drops the window's key status on
    // macOS, which left the user having to click the panel once before it
    // would respond to anything. Taking focus back is safe here specifically
    // because this transition is always user-initiated ("Start listening") —
    // an automatically-arriving hint must never steal focus, or the next
    // keystrokes meant for the editor or call would land in the panel.
    apply("set_focus", win.set_focus());
    log_space_state(&win, "enter/after-focus(FINAL)");
    eprintln!("lib: enter_listening_mode finished, returning Ok");
    Ok(())
}

/// Re-asserts cross-Space membership after a live manual drag of the compact
/// panel (macOS's hand-rolled header drag in main.ts, not Tauri's native
/// `performWindowDragWithEvent` — that path was already fixed by avoiding it
/// entirely). Root-caused 2026-08-19 (docs/TODO.md): a *single* programmatic
/// `set_position` call never breaks `CanJoinAllSpaces`, but the live drag
/// loop calls `setPosition()` repeatedly on every `mousemove` — same class of
/// problem as `orderOut`/`performWindowDragWithEvent` before it, just a third
/// trigger. The fix is the same shape as those: don't avoid the risky
/// operation, just re-assert `set_visible_on_all_workspaces(true)`
/// immediately after it, once the drag has actually ended (`mouseup`), not on
/// every intermediate move.
#[tauri::command]
fn reassert_cross_space(app: AppHandle) -> Result<(), String> {
    let win = window(&app)?;
    log_space_state(&win, "dragfix/before-reassert");
    apply(
        "set_visible_on_all_workspaces (drag-fix)",
        win.set_visible_on_all_workspaces(true),
    );
    log_space_state(&win, "dragfix/after-reassert");
    Ok(())
}

#[tauri::command]
fn exit_listening_mode(app: AppHandle) -> Result<(), String> {
    let win = window(&app)?;
    let state = app.state::<AppState>();
    *state.capture_excluded.lock().unwrap() = false;
    let _ = audio::stop_audio_capture_sync(&app);

    // Remember whatever size the panel was actually at (which may differ
    // from COMPACT_SIZE if the user dragged an edge) before resizing away
    // to the setup view, so enter_listening_mode can restore it. Guarded on
    // actually having been in compact mode (undecorated) — this command is
    // also reachable from the setup screen's "Permissions" section (denied →
    // "Back") without ever having entered compact mode at all, and without
    // this guard that path would capture the *setup* window's size/position
    // and silently corrupt the next real entry into listening mode.
    let was_compact = matches!(win.is_decorated(), Ok(false));
    if was_compact {
        if let Ok(scale) = win.scale_factor() {
            if let Ok(size) = win.inner_size() {
                let logical = size.to_logical::<f64>(scale);
                *state.compact_size.lock().unwrap() = Some((logical.width, logical.height));
            }
        }
        // Captured before the resize/center below moves the window away.
        if let Ok(pos) = win.outer_position() {
            *state.compact_position.lock().unwrap() = Some((pos.x, pos.y));
        }
    }

    let (w, h) = SETUP_SIZE;
    let (min_w, min_h) = SETUP_MIN_SIZE;
    // Leaving compact mode while the panel happened to be faded out would
    // otherwise hand the user an invisible setup screen (see
    // set_panel_hidden_via_alpha — a faded window is still ordered in, so
    // nothing else would bring it back).
    #[cfg(target_os = "macos")]
    {
        set_panel_hidden_via_alpha(&win, false);
        PANEL_ALPHA_HIDDEN.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    apply("set_content_protected", win.set_content_protected(false));
    apply("set_always_on_top", win.set_always_on_top(false));
    // Deliberately NOT calling `set_visible_on_all_workspaces(false)` here —
    // this used to turn cross-Space membership back off on every exit, which
    // sounded right (setup mode doesn't need it) but is the actual cause of a
    // second, independent bug found live 2026-08-08: Start listening → ⚙ back
    // to settings → Start listening again silently lost the panel's
    // visibility over other fullscreen Spaces, even though
    // `set_visible_on_all_workspaces(true)` in enter_listening_mode reported
    // success and `collectionBehavior` read back correctly afterwards.
    //
    // Root-caused by instrumenting the window's real `collectionBehavior`/
    // `isOnActiveSpace` state at each step of a live repro: the flag was
    // always correct, but toggling it off and back on left the window
    // permanently unable to actually composite over another Space, despite
    // reporting `CanJoinAllSpaces` — same "WindowServer silently disagrees
    // with the flag" class of bug already found for `orderOut` (see
    // set_panel_hidden_via_alpha above) and window dragging (see main.ts's
    // manual drag handler), just a third trigger. styleMask changes
    // (set_decorations) were ruled out first by testing with that call
    // skipped — bug persisted. Skipping this specific call fixed it: once
    // set, collectionBehavior simply stays CanJoinAllSpaces for the rest of
    // the process's life, including in setup mode. Confirmed harmless there
    // in practice — setup mode is never always-on-top or floating-level, so
    // it doesn't visibly intrude over another Space's fullscreen app even
    // while nominally eligible to.
    apply("set_decorations", win.set_decorations(true));
    apply("set_resizable", win.set_resizable(true));
    // Restore the larger setup-mode floor before resizing back up.
    apply(
        "set_min_size",
        win.set_min_size(Some(LogicalSize::new(min_w, min_h))),
    );
    apply("set_size", win.set_size(LogicalSize::new(w, h)));
    apply("center", win.center());
    // Same reason as in enter_listening_mode: restoring the decorations costs
    // the window its key status, and this transition is user-initiated too
    // (the settings button, or "Back" on the permission screen).
    apply("set_focus", win.set_focus());
    log_space_state(&win, "exit/FINAL");
    Ok(())
}

/// Called when the user flips the "keep hint window out of screen share"
/// toggle while already in compact/listening mode, without a full mode
/// transition. Setup mode always runs with capture exclusion off.
#[tauri::command]
fn set_capture_exclusion(app: AppHandle, enabled: bool) -> Result<(), String> {
    let win = window(&app)?;
    let state = app.state::<AppState>();
    *state.capture_excluded.lock().unwrap() = enabled;
    log_space_state(&win, &format!("captureToggle/BEFORE(enabled={enabled})"));
    apply("set_content_protected", win.set_content_protected(enabled));
    log_space_state(&win, &format!("captureToggle/AFTER(enabled={enabled})"));
    Ok(())
}

#[tauri::command]
fn toggle_window_visibility(app: AppHandle) -> Result<(), String> {
    let win = window(&app)?;
    // macOS, compact panel only: fade instead of ordering the window out —
    // see set_panel_hidden_via_alpha for why ordering out costs the panel its
    // all-Spaces membership. The setup screen keeps the ordinary hide/show: it
    // has no cross-Space requirement, and a real hide keeps it out of
    // Cmd+Tab/Mission Control the way a normal window should be.
    #[cfg(target_os = "macos")]
    if matches!(win.is_decorated(), Ok(false)) {
        let hidden = PANEL_ALPHA_HIDDEN.load(std::sync::atomic::Ordering::SeqCst);
        log_space_state(&win, &format!("hotkeyToggle/BEFORE(wasHidden={hidden})"));
        set_panel_hidden_via_alpha(&win, !hidden);
        PANEL_ALPHA_HIDDEN.store(!hidden, std::sync::atomic::Ordering::SeqCst);
        log_space_state(&win, &format!("hotkeyToggle/AFTER(nowHidden={})", !hidden));
        return Ok(());
    }
    let visible = win.is_visible().map_err(|e| e.to_string())?;
    if visible {
        win.hide().map_err(|e| e.to_string())?;
    } else {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn parse_shortcut(spec: &str) -> Result<Shortcut, String> {
    // Accepts specs like "CmdOrCtrl+Shift+S".
    let mut modifiers = Modifiers::empty();
    let mut code: Option<Code> = None;

    for part in spec.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "cmdorctrl" | "commandorcontrol" => {
                #[cfg(target_os = "macos")]
                {
                    modifiers |= Modifiers::SUPER;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    modifiers |= Modifiers::CONTROL;
                }
            }
            "cmd" | "command" | "super" => modifiers |= Modifiers::SUPER,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            key => {
                code = Some(match key {
                    "a" => Code::KeyA,
                    "b" => Code::KeyB,
                    "c" => Code::KeyC,
                    "d" => Code::KeyD,
                    "e" => Code::KeyE,
                    "f" => Code::KeyF,
                    "g" => Code::KeyG,
                    "h" => Code::KeyH,
                    "i" => Code::KeyI,
                    "j" => Code::KeyJ,
                    "k" => Code::KeyK,
                    "l" => Code::KeyL,
                    "m" => Code::KeyM,
                    "n" => Code::KeyN,
                    "o" => Code::KeyO,
                    "p" => Code::KeyP,
                    "q" => Code::KeyQ,
                    "r" => Code::KeyR,
                    "s" => Code::KeyS,
                    "t" => Code::KeyT,
                    "u" => Code::KeyU,
                    "v" => Code::KeyV,
                    "w" => Code::KeyW,
                    "x" => Code::KeyX,
                    "y" => Code::KeyY,
                    "z" => Code::KeyZ,
                    "0" => Code::Digit0,
                    "1" => Code::Digit1,
                    "2" => Code::Digit2,
                    "3" => Code::Digit3,
                    "4" => Code::Digit4,
                    "5" => Code::Digit5,
                    "6" => Code::Digit6,
                    "7" => Code::Digit7,
                    "8" => Code::Digit8,
                    "9" => Code::Digit9,
                    "space" => Code::Space,
                    other => return Err(format!("unsupported shortcut key: {other}")),
                });
            }
        }
    }

    let code = code.ok_or_else(|| "shortcut spec has no key".to_string())?;
    Ok(Shortcut::new(Some(modifiers), code))
}

/// Registers a new global shortcut into the given slot, replacing whichever
/// one is currently active there. Errors (most commonly: another app already
/// holds that key combination) are returned to the caller rather than
/// swallowed — a shortcut that silently fails to register is worse than no
/// shortcut at all, since the user has no way to know the feature is dead
/// until they're mid-interview and it doesn't respond. Shared by all three
/// fixed hotkeys (show/hide, screenshot, start-over), which is why it takes
/// the state slot as a parameter instead of being duplicated per shortcut —
/// even though none of them re-register after startup anymore (2026-09-15,
/// docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md), "replace whichever one is
/// active" is still exactly what a first-time registration needs, since the
/// slot always starts `None`.
fn register_shortcut(
    app: &AppHandle,
    slot: &Mutex<Option<Shortcut>>,
    spec: &str,
) -> Result<(), String> {
    let shortcut = parse_shortcut(spec)?;
    let gs = app.global_shortcut();

    let previous = slot.lock().unwrap().take();
    if let Some(old) = previous {
        let _ = gs.unregister(old);
    }

    if let Err(e) = gs.register(shortcut) {
        // Registering the new combo failed (most likely: another app already
        // holds it) — put the old one back rather than leaving the user with
        // no working shortcut at all while they try something else.
        if let Some(old) = previous {
            if gs.register(old).is_ok() {
                *slot.lock().unwrap() = Some(old);
            }
        }
        return Err(format!("That shortcut is already in use by another app: {e}"));
    }

    *slot.lock().unwrap() = Some(shortcut);
    Ok(())
}

/// Show/Hide — fixed, not user-configurable (docs/PROPOSAL-TOOLBAR-HOTKEYS-
/// SETTINGS.md, 2026-09-15). ⌘B was picked by the owner directly (matches
/// Cluely); no longer the "S is contested, Space isn't" reasoning that
/// picked the old default (Cmd/Ctrl+Shift+Space) — that reasoning is now
/// moot since the combo isn't offered as a customizable starting point
/// anymore, just used as-is.
fn default_visibility_shortcut_spec() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+B"
    } else {
        "Ctrl+B"
    }
}

/// Screenshot — fixed at ⌘H/Ctrl+H, always registered at startup. Still
/// named `region_*` in the shortcut-slot/state field (settings.json/event-
/// name compat reasoning predates this and still applies to whatever
/// references survive) even though there's no region-select behavior left
/// and, as of 2026-09-15, no on/off setting either — see the toolbar
/// Screenshot button, which triggers the identical capture.
fn default_screenshot_shortcut_spec() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+H"
    } else {
        "Ctrl+H"
    }
}

/// Start Over — fixed at ⌘R/Ctrl+R, always registered at startup.
fn default_start_over_shortcut_spec() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+R"
    } else {
        "Ctrl+R"
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `mut` is only needed for the macOS activation-policy step after `build()`.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut app = tauri::Builder::default()
        .manage(AppState::default())
        .manage(audio::AudioState::default())
        .manage(stt::SttState::default())
        .manage(llm::AgentState::default())
        .manage(summary::SummaryState::default())
        // Must be the first plugin registered — it needs to intercept a
        // second launch before anything else in the chain runs. A relaunch
        // (double-clicking the icon/shortcut again while already running)
        // just shows and focuses the existing window instead of starting a
        // second process; this is user-initiated the same way a fresh
        // launch is, so taking focus here doesn't violate the "focus only on
        // user-initiated action" rule (see enter_listening_mode).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Ok(win) = window(app) {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    let state = app.state::<AppState>();
                    let is_visibility = *state.visibility_shortcut.lock().unwrap() == Some(*shortcut);
                    let is_region_screenshot =
                        *state.region_screenshot_shortcut.lock().unwrap() == Some(*shortcut);
                    let is_start_over =
                        *state.start_over_shortcut.lock().unwrap() == Some(*shortcut);

                    if is_visibility {
                        let _ = toggle_window_visibility(app.clone());
                    } else if is_start_over {
                        // No capture logic of its own — the frontend owns the
                        // hint feed, so this just tells it to clear, same as
                        // clicking the toolbar's Start Over button.
                        let _ = app.emit("start-over-triggered", ());
                    } else if is_region_screenshot {
                        // Repurposed 2026-08-16 (docs/TODO.md, "Убрать
                        // region-select скриншот") to drive the whole-screen
                        // "Screenshot" button's capture instead of
                        // region-select — capture_screen_region() itself is
                        // untouched below, just no longer called from here,
                        // kept so this is cheap to revert. The `is_region_
                        // screenshot`/`region_screenshot_shortcut` names
                        // stay as-is (see the settings.json-compat reasoning
                        // in index.html) even though this now triggers a
                        // whole-screen capture, same one the "Screenshot"
                        // button calls, just triggered here instead of via
                        // invoke() — this has to work while the window isn't
                        // focused, which is the entire point of a global
                        // hotkey.
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            match screenshot::capture_full_screen(app.clone()).await {
                                Ok(Some(data_uri)) => {
                                    let _ = app.emit("screenshot-captured", data_uri);
                                }
                                // No "user cancelled" case for a whole-screen
                                // capture (see capture_full_screen's doc
                                // comment) — None here only means the
                                // in-flight guard was already held, e.g. the
                                // button and hotkey fired at the same
                                // instant.
                                Ok(None) => {}
                                Err(e) => {
                                    eprintln!("screenshot hotkey failed: {e}");
                                    let _ = app.emit("screenshot-error", e);
                                }
                            }
                        });
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            enter_listening_mode,
            exit_listening_mode,
            reassert_cross_space,
            set_capture_exclusion,
            toggle_window_visibility,
            audio::start_audio_capture,
            audio::stop_audio_capture,
            audio::check_audio_permission,
            permissions::check_screen_recording_permission,
            permissions::request_screen_recording_permission,
            permissions::is_macos_tahoe_or_later,
            screenshot::capture_screen_region,
            screenshot::capture_full_screen,
            llm::generate_answer,
            summary::summarize_question,
            knowledge::list_companies,
            knowledge::list_profiles,
            knowledge::reload_knowledge,
            knowledge::log_gap,
            knowledge::log_interview_entry,
            knowledge::log_transcript_line,
            llm::nudge_repaint,
            stt::is_model_downloaded,
            stt::is_model_bundled,
            stt::download_model,
            settings::load_settings,
            settings::save_settings,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            llm::prewarm(&handle);
            summary::prewarm(&handle);
            // Show/Hide, Screenshot and Start Over are fixed, non-
            // configurable global hotkeys as of 2026-09-15
            // (docs/PROPOSAL-TOOLBAR-HOTKEYS-SETTINGS.md) — registered once
            // here, unconditionally, no longer read from settings.json or
            // changeable from a Settings UI. A failure here (the combo
            // already claimed by another app) is just logged: there's no
            // Settings UI left to redirect the user to pick another one, and
            // the toolbar buttons still work as a fallback either way.
            let hotkey_state = handle.state::<AppState>();
            if let Err(e) = register_shortcut(
                &handle,
                &hotkey_state.visibility_shortcut,
                default_visibility_shortcut_spec(),
            ) {
                eprintln!("failed to register show/hide shortcut: {e}");
            }
            if let Err(e) = register_shortcut(
                &handle,
                &hotkey_state.region_screenshot_shortcut,
                default_screenshot_shortcut_spec(),
            ) {
                eprintln!("failed to register screenshot shortcut: {e}");
            }
            if let Err(e) = register_shortcut(
                &handle,
                &hotkey_state.start_over_shortcut,
                default_start_over_shortcut_spec(),
            ) {
                eprintln!("failed to register start-over shortcut: {e}");
            }
            // Second half of the Accessory-at-birth trick set up below the
            // builder — by the time this closure runs, Tauri has already
            // created the config window (it does that at the top of the same
            // internal setup, on RunEvent::Ready), so the window has its
            // cross-Space capability and the app can go back to being an
            // ordinary foreground app: Dock icon, Cmd+Tab and the menu bar
            // (which is what carries ⌘V, a core feature here) all return.
            // Goes through the AppHandle, not `app.set_activation_policy` —
            // the latter only writes the pre-run value, which is already spent
            // at this point, and would silently do nothing.
            #[cfg(target_os = "macos")]
            {
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Regular);
                // Queued behind the policy message on the same event-loop
                // queue, so it lands after the app is Regular again: an
                // Accessory app doesn't activate itself, and without this the
                // window comes up behind whatever was in front. Launching the
                // app is a user-initiated action, so this doesn't breach
                // app/CLAUDE.md's "focus is only ever taken on a
                // user-initiated action" rule.
                if let Ok(win) = window(&handle) {
                    let _ = win.set_focus();
                }
            }
            // SPACEDBG (temporary, see docs/TODO.md): watch for the state
            // degrading with none of our own code running, and tag every
            // native resize/move so a flip can be attributed to one.
            #[cfg(target_os = "macos")]
            {
                if let Ok(win) = window(&handle) {
                    log_space_state(&win, "setup/launch");
                    let ev_win = win.clone();
                    win.on_window_event(move |event| match event {
                        tauri::WindowEvent::Resized(size) => {
                            log_space_state(&ev_win, &format!("event/Resized({}x{})", size.width, size.height));
                        }
                        tauri::WindowEvent::Moved(pos) => {
                            log_space_state(&ev_win, &format!("event/Moved({},{})", pos.x, pos.y));
                        }
                        tauri::WindowEvent::Focused(f) => {
                            log_space_state(&ev_win, &format!("event/Focused({f})"));
                        }
                        _ => {}
                    });
                }
                spawn_space_state_watcher(handle.clone());
                spawn_space_debug_driver(handle.clone());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // The panel can only ever float over another app's fullscreen Space if the
    // app was an Accessory (agent) app at the moment the window was *created* —
    // measured, not assumed: with the app Regular at creation, no
    // collectionBehavior and no window level makes the panel appear over a
    // fullscreen Zoom/VS Code Space, and nothing rescues such a window
    // afterwards (re-applying the behavior, re-ordering, a full hide/show
    // cycle, a Space round-trip — all tried, all fail). A window born under
    // Accessory keeps the capability permanently, even after the app switches
    // back to Regular, which is what makes this two-step dance work at no cost
    // to the Dock icon or the menu bar. See docs/PRODUCT-BRIEF.md §5, "Панель
    // не видна поверх чужого полноэкранного (fullscreen) Space на macOS".
    //
    // This has to be set *before* `run()`: Tauri creates the config window on
    // RunEvent::Ready, which is after macOS has applied this policy in
    // applicationDidFinishLaunching. The `.setup()` closure above puts it back
    // to Regular immediately afterwards.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    app.run(|app_handle, event| {
            // Make sure the audio-capture helper never outlives the app —
            // otherwise quitting the window would leave it silently
            // recording in the background.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Only reached when the window itself is closed; a menu/Cmd+Q
                // quit bypasses this entirely and is handled by the atexit
                // guard instead (see install_exit_guard). Both paths end in
                // `_exit` for the same ggml reason documented there.
                let _ = audio::stop_audio_capture_sync(app_handle);
                // `_exit` is only declared (and only needed) on macOS — see
                // install_exit_guard above for the ggml Metal-destructor crash
                // it routes around. Other platforms have no such static
                // destructor to skip, so a normal process exit is enough.
                #[cfg(target_os = "macos")]
                unsafe {
                    _exit(0)
                }
                #[cfg(not(target_os = "macos"))]
                std::process::exit(0);
            }
        });
}
