// Region-screenshot input for live coding questions.
//
// Critical design constraint from docs/PRODUCT-BRIEF.md §5: the capture is
// performed by the operating system's *own* screenshot tool, never by this
// app. Doing it ourselves (Tauri screenshot APIs, ScreenCaptureKit, etc.)
// would require the macOS "Screen Recording" permission, which would make
// the product's central claim — that it only listens to audio and never
// looks at your screen — literally false. Here the OS captures, drops the
// result on the clipboard, and we only read the clipboard, exactly like any
// app receiving a paste. That needs no screen permission at all, and works
// the same whether the user's screenshot came from the built-in tool or a
// third-party one (Lightshot, Snagit, ShareX, …).

use std::sync::atomic::Ordering;

use base64::Engine;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::AppState;

/// Encodes a clipboard image (raw RGBA) as a base64 PNG data URI, which is
/// both directly renderable in the webview and the format vision LLM APIs
/// expect.
fn rgba_to_png_data_uri(rgba: &[u8], width: u32, height: u32) -> Result<String, String> {
    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

/// (width, height, RGBA bytes) snapshot of whatever image is currently on
/// the clipboard, or `None` if there isn't one — used to detect whether
/// `screencapture` actually put something new there, since its exit status
/// alone can't tell us that (see the comment in capture_region).
fn clipboard_image_signature(app: &AppHandle) -> Option<(u32, u32, Vec<u8>)> {
    let image = app.clipboard().read_image().ok()?;
    Some((image.width(), image.height(), image.rgba().to_vec()))
}

/// Launches the OS region-selection screenshot tool and returns whatever
/// image it leaves on the clipboard, as a PNG data URI. Returns `None` if
/// the user cancelled.
#[tauri::command]
pub async fn capture_screen_region(app: AppHandle) -> Result<Option<String>, String> {
    // Bail out instead of racing a second `screencapture` process against
    // one that's already waiting on the user's selection — see the
    // `screenshot_capturing` doc comment on AppState for why.
    if app
        .state::<AppState>()
        .screenshot_capturing
        .swap(true, Ordering::SeqCst)
    {
        return Ok(None);
    }

    let outcome = capture_region(&app).await;

    app.state::<AppState>()
        .screenshot_capturing
        .store(false, Ordering::SeqCst);

    outcome
}

/// Longest we'll wait for the Windows snipping tool to produce something
/// before treating the capture as cancelled.
#[cfg(target_os = "windows")]
const SNIP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// How long the hint window stays hidden while we wait. Dragging out a region
/// takes a few seconds; well past that the user has most likely pressed Esc,
/// and leaving their window hidden for a capture that is never coming is
/// worse than the small chance of the panel showing up in an unusually slow
/// selection. Polling continues after the window is back, so a late capture
/// still lands.
#[cfg(target_os = "windows")]
const SNIP_HIDE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Waits for the snipping tool to put a *new* image on the clipboard.
///
/// Needed only on Windows, and this is the whole reason the platforms differ
/// here: `screencapture -i -c` on macOS blocks until the user is finished, so
/// the clipboard is already populated when it returns. `explorer
/// ms-screenclip:` merely hands the URI to a registered handler and exits —
/// measured at ~350 ms, i.e. long before any selection has been made — so
/// reading the clipboard once, right after it returns, always finds the old
/// contents and reports a cancelled capture no matter what the user does.
#[cfg(target_os = "windows")]
fn wait_for_new_clipboard_image(
    app: &AppHandle,
    before: Option<&(u32, u32, Vec<u8>)>,
    window: &Option<tauri::WebviewWindow>,
) -> Option<(u32, u32, Vec<u8>)> {
    let started = std::time::Instant::now();
    let mut window_restored = false;

    loop {
        if let Some(current) = clipboard_image_signature(app) {
            if Some(&current) != before {
                return Some(current);
            }
        }

        if !window_restored && started.elapsed() >= SNIP_HIDE_LIMIT {
            if let Some(win) = window {
                let _ = win.show();
            }
            window_restored = true;
        }

        if started.elapsed() >= SNIP_TIMEOUT {
            return None;
        }

        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Silently captures the whole main screen via the OS's own tool — same
/// principle as `capture_screen_region` (see the module doc comment), just
/// skipping the interactive selection step entirely. No "user cancelled"
/// case exists here since there's no selection to cancel.
#[tauri::command]
pub async fn capture_full_screen(app: AppHandle) -> Result<Option<String>, String> {
    if app
        .state::<AppState>()
        .screenshot_capturing
        .swap(true, Ordering::SeqCst)
    {
        return Ok(None);
    }

    let outcome = capture_full(&app).await;

    app.state::<AppState>()
        .screenshot_capturing
        .store(false, Ordering::SeqCst);

    outcome
}

/// Grabs the primary monitor straight off the desktop via GDI `BitBlt`, no
/// clipboard involved.
///
/// This used to simulate a PrtScn keypress (`SendInput` + `VK_SNAPSHOT`) and
/// read the result off the clipboard, on the assumption that plain PrtScn is
/// Windows' own silent full-screen-to-clipboard shortcut. That assumption
/// broke: starting with Windows 11 build 22621.1928, `PrintScreenKeyForSnippingEnabled`
/// defaults to on, which makes a simulated PrtScn open the Snip & Sketch
/// region-selection overlay instead — the exact "drag to select" experience
/// this button exists to skip. GDI screen capture needs no Windows
/// permission (unlike macOS's Screen Recording TCC gate — see the module
/// doc comment), so grabbing pixels directly doesn't compromise the "OS
/// does the capturing" design, and it no longer depends on a system
/// keyboard-shortcut setting Microsoft can silently flip.
#[cfg(target_os = "windows")]
fn capture_primary_screen_rgba() -> Result<(u32, u32, Vec<u8>), String> {
    use std::mem::size_of;
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT,
        DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err("could not read the primary screen's dimensions".to_string());
        }

        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return Err("could not get a device context for the screen".to_string());
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        if mem_dc.is_null() || bitmap.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return Err("could not create an off-screen bitmap for the capture".to_string());
        }
        let previous: HGDIOBJ = SelectObject(mem_dc, bitmap as HGDIOBJ);

        // CAPTUREBLT includes layered/WS_EX_LAYERED windows (e.g. anything
        // using transparency) in the capture, matching what a real PrtScn
        // would have shown.
        let blit_ok = BitBlt(
            mem_dc,
            0,
            0,
            width,
            height,
            screen_dc,
            0,
            0,
            SRCCOPY | CAPTUREBLT,
        );

        let mut rgba = Vec::new();
        let mut result = Ok(());
        if blit_ok == 0 {
            result = Err("BitBlt failed while copying the screen".to_string());
        } else {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height requests a top-down DIB, so rows come out
                // in normal (not upside-down) image order.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
            let lines = GetDIBits(
                mem_dc,
                bitmap,
                0,
                height as u32,
                bgra.as_mut_ptr() as *mut _,
                &mut info,
                DIB_RGB_COLORS,
            );

            if lines == 0 {
                result = Err("GetDIBits failed while reading the captured pixels".to_string());
            } else {
                // GDI hands back BGRA with an unset (garbage/zero) alpha
                // byte — swap to RGBA and force alpha opaque, matching what
                // `rgba_to_png_data_uri` (and the clipboard path) expect.
                rgba = bgra;
                for px in rgba.chunks_exact_mut(4) {
                    px.swap(0, 2);
                    px[3] = 255;
                }
            }
        }

        SelectObject(mem_dc, previous);
        DeleteObject(bitmap as HGDIOBJ);
        DeleteDC(mem_dc);
        ReleaseDC(std::ptr::null_mut(), screen_dc);

        result?;
        Ok((width as u32, height as u32, rgba))
    }
}

// Windows never touches the clipboard for a full-screen capture — see
// `capture_primary_screen_rgba`'s doc comment for why — so it has its own,
// much shorter implementation instead of sharing the clipboard-polling path
// below with macOS.
#[cfg(target_os = "windows")]
async fn capture_full(app: &AppHandle) -> Result<Option<String>, String> {
    // Same self-exclusion as the macOS branch below (see its comment) — GDI
    // `BitBlt` grabs whatever's actually on screen just as unconditionally
    // as `screencapture -x`, so without hiding the panel first it can end up
    // in its own capture too.
    let window = app.get_webview_window("main");
    if let Some(win) = &window {
        let _ = win.hide();
    }

    let outcome: Result<Result<(u32, u32, Vec<u8>), String>, String> =
        tauri::async_runtime::spawn_blocking(capture_primary_screen_rgba)
            .await
            .map_err(|e| e.to_string());

    if let Some(win) = &window {
        let _ = win.show();
    }

    let (width, height, rgba) = outcome??;
    rgba_to_png_data_uri(&rgba, width, height).map(Some)
}

#[cfg(not(target_os = "windows"))]
async fn capture_full(app: &AppHandle) -> Result<Option<String>, String> {
    // Get our own always-on-top panel out of the way first — same reasoning
    // as capture_region below: with no self-exclusion, a full-screen capture
    // will happily include SyntaxCue's own panel (and whatever hint text is
    // currently showing in it) in the image sent to the LLM. Confirmed live
    // 2026-08-21: a screenshot taken with a prior answer still visible in the
    // panel got refused by the model as "an interview-cheating tool actively
    // displaying a pre-generated answer" — an accurate description of what
    // was actually in the frame, not a false positive. Unconditional, not
    // gated behind "Keep hint window out of screen share"
    // (`capture_exclusion`, on by default) — that setting is the user's own
    // privacy choice about third-party screen share; this is a correctness
    // bug that must not happen regardless of it (docs/TODO.md "Полноэкранный
    // скриншот может захватывать саму панель").
    //
    // Must be the alpha-fade from `lib.rs::hide_panel_for_capture_async`,
    // never `WebviewWindow::hide()`/`show()` — a first version of this fix
    // used plain hide()/show() and reintroduced the exact cross-Space
    // visibility bug that alpha-fade exists to avoid (found live-testing
    // 2026-08-21/22, see the long comment on `set_panel_hidden_via_alpha` in
    // `lib.rs`). Must also go through the *_async wrapper, not the raw
    // AppKit call directly — this command runs on a tokio worker thread, not
    // the main thread, and AppKit crashes if called off it (also found live
    // 2026-08-22, `EXC_BREAKPOINT`/"Must only be used from the main thread",
    // the first version of this exact fix).
    #[cfg(target_os = "macos")]
    let was_already_hidden = crate::hide_panel_for_capture_async(app).await;

    let before = clipboard_image_signature(app);

    let result = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        {
            // -x: silent (no shutter sound) and, critically, no interactive
            // UI at all — captures the whole main screen straight to the
            // clipboard. Same binary and same Screen Recording permission
            // as the `-i` region variant, just skipping the selection step.
            std::process::Command::new("/usr/sbin/screencapture")
                .args(["-x", "-c"])
                .status()
                .map(|_| ())
                .map_err(|e| format!("could not launch the macOS screenshot tool: {e}"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err::<(), String>("full-screen screenshots aren't supported on this OS yet".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    result?;

    #[cfg(target_os = "macos")]
    crate::restore_panel_after_capture_async(app, was_already_hidden).await;

    let mut captured = None;
    for _ in 0..20 {
        if let Some(current) = clipboard_image_signature(app) {
            if Some(&current) != before.as_ref() {
                captured = Some(current);
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    match captured {
        Some((width, height, rgba)) => rgba_to_png_data_uri(&rgba, width, height).map(Some),
        None => Ok(None),
    }
}

async fn capture_region(app: &AppHandle) -> Result<Option<String>, String> {
    // Get our own always-on-top panel out of the way so it neither covers
    // the thing being captured nor ends up inside the capture itself.
    //
    // macOS must go through the async alpha-fade in
    // `lib.rs::hide_panel_for_capture_async`, never a plain
    // `WebviewWindow::hide()` — see the long comment on
    // `set_panel_hidden_via_alpha` in `lib.rs` (and `capture_full` above,
    // which hit both the cross-Space visibility bug *and* a main-thread
    // AppKit crash live 2026-08-21/22, in two successive versions of this
    // same fix) for why. The async wrapper dispatches to the main thread
    // itself, so it doesn't need `window` here at all. Windows has no such
    // bug, so it keeps the plain call.
    #[cfg(target_os = "macos")]
    let was_already_hidden = crate::hide_panel_for_capture_async(app).await;
    // Only fetched on non-macOS — the async wrapper above looks up the
    // window itself, so nothing here needs a handle to it on macOS.
    #[cfg(not(target_os = "macos"))]
    let window = app.get_webview_window("main");
    #[cfg(not(target_os = "macos"))]
    if let Some(win) = &window {
        let _ = win.hide();
    }

    // screencapture -i -c exits 0 whether the user selected a region or
    // pressed Escape — cancelling is not an error, it just leaves the
    // clipboard untouched. So "is there an image on the clipboard
    // afterwards" is not the same question as "did the user just take a
    // screenshot": if an old screenshot was already sitting on the
    // clipboard from earlier, pressing Escape here would silently re-add
    // that stale image as if it were new. Snapshotting before and comparing
    // after is the only reliable way to tell the two cases apart.
    let before = clipboard_image_signature(app);

    let result = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        {
            // -i: interactive region selection, -c: copy to clipboard.
            std::process::Command::new("/usr/sbin/screencapture")
                .args(["-i", "-c"])
                .status()
                .map_err(|e| format!("could not launch the macOS screenshot tool: {e}"))
        }
        #[cfg(target_os = "windows")]
        {
            // Windows' Snipping Tool region capture, same idea: the OS
            // captures, the result lands on the clipboard.
            std::process::Command::new("explorer")
                .arg("ms-screenclip:")
                .status()
                .map_err(|e| format!("could not launch the Windows snipping tool: {e}"))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err("region screenshots aren't supported on this OS yet".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    result?;

    // macOS: the tool has already finished by the time it returns, so the
    // clipboard can be read straight away. Windows: it has barely started —
    // see wait_for_new_clipboard_image.
    #[cfg(target_os = "windows")]
    let captured = {
        let app_for_wait = app.clone();
        let window_for_wait = window.clone();
        let before_for_wait = before.clone();
        let found = tauri::async_runtime::spawn_blocking(move || {
            wait_for_new_clipboard_image(
                &app_for_wait,
                before_for_wait.as_ref(),
                &window_for_wait,
            )
        })
        .await
        .map_err(|e| e.to_string())?;
        if let Some(win) = &window {
            let _ = win.show();
        }
        found
    };

    #[cfg(target_os = "macos")]
    let captured = {
        crate::restore_panel_after_capture_async(app, was_already_hidden).await;
        clipboard_image_signature(app).filter(|current| Some(current) != before.as_ref())
    };

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let captured = {
        if let Some(win) = &window {
            let _ = win.show();
        }
        clipboard_image_signature(app).filter(|current| Some(current) != before.as_ref())
    };

    match captured {
        Some((width, height, rgba)) => rgba_to_png_data_uri(&rgba, width, height).map(Some),
        // Either nothing on the clipboard, or it's byte-for-byte the same
        // image that was there before the capture tool ran — both mean the
        // user cancelled, not an error to surface.
        None => Ok(None),
    }
}
