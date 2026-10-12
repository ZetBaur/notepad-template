// Screen Recording permission preflight/request, for the setup screen's
// "Permissions" section (docs/TODO.md, "Разрешения macOS запрашиваются не
// вовремя" — added 2026-07-30). Unlike system-audio capture (see
// `audio::check_audio_permission`), CoreGraphics does expose a real preflight
// check for this one: `CGPreflightScreenCaptureAccess` reads the current TCC
// decision with no dialog, and `CGRequestScreenCaptureAccess` shows the
// system prompt only if the decision isn't made yet.
//
// This is deliberately unrelated to `screenshot.rs`'s region-capture flow —
// that module's whole point is running the OS's own screenshot tool so this
// app never needs Screen Recording access at all. macOS 26 changed that:
// invoking `/usr/sbin/screencapture` as a subprocess now itself requires the
// calling app to hold this permission (a live-test finding, not something
// this app's own architecture chose). So these checks exist purely to let
// the user grant that requirement calmly, ahead of time, from the setup
// screen — not because the capture design changed.
//
// Raw FFI rather than a crate: same reasoning as `license.rs`'s use of the
// `hostname` binary and `audio.rs`'s raw `kill` — two functions, no crate in
// the ecosystem wraps just these, not worth a new dependency for two extern
// declarations.

#[cfg(target_os = "macos")]
mod cg {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        // Real signature returns `Boolean` (typedef unsigned char in
        // CoreGraphics' C headers, always exactly 0 or 1) — `u8` matches
        // that precisely rather than leaning on Rust `bool`'s FFI niceties.
        pub fn CGPreflightScreenCaptureAccess() -> u8;
        pub fn CGRequestScreenCaptureAccess() -> u8;
    }
}

/// No dialog, just reads the current decision. Safe to call freely — e.g.
/// every time the "Region screenshot hotkey" checkbox is toggled on, to show
/// its current status before the user has asked for anything.
#[tauri::command]
pub fn check_screen_recording_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe { cg::CGPreflightScreenCaptureAccess() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // No equivalent gate on Windows (ms-screenclip needs no extra
        // permission) — nothing to check, so nothing blocks on it.
        true
    }
}

/// Shows the system permission prompt if the decision isn't made yet;
/// otherwise a no-op. Must only be called from an explicit user action (a
/// button click after they've read why), never automatically — the entire
/// point of this section is not surprising the user with a system dialog.
///
/// Note for the caller: granting Screen Recording often only takes effect
/// for a process after it's relaunched (a macOS quirk, not specific to this
/// app) — don't assume `check_screen_recording_permission` flips to `true`
/// immediately after this returns.
#[tauri::command]
pub fn request_screen_recording_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe { cg::CGRequestScreenCaptureAccess() != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// True on macOS 26 (Tahoe) and later, where System Settings merged "Screen
/// Recording" and "System Audio Recording" into one "Screen & System Audio
/// Recording" category — the setup screen uses this only to pick the right
/// wording for the manual recovery instructions ("System Settings didn't
/// open the right section?"), not for any permission decision itself. Shells
/// out to `sw_vers` rather than parsing `ProcessInfo` via FFI — same
/// raw-and-minimal reasoning as the CoreGraphics bindings above, but this one
/// binary is simpler to call than to bind.
#[tauri::command]
pub fn is_macos_tahoe_or_later() -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(output) = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
        else {
            return false;
        };
        let version = String::from_utf8_lossy(&output.stdout);
        let major = version
            .trim()
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        major >= 26
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
