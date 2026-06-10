//! Kōrero (v1.18.1): SPIKE for contextual prompt auto-routing (UX roadmap
//! item 8 — the research's headline finding).
//!
//! Answers exactly ONE question: can we reliably read the active window
//! title from Tauri's privilege context on this machine? Surfaced as a
//! diagnostics button on the Help page (3 s delay so the user can focus
//! Slack/Gmail/etc. first).
//!
//! Verdict routing (see KORERO_VERIFICATION_CHECKLIST.md §E):
//! - Titles come back correct and stable → v1.20 builds the routing table
//!   (window-title pattern → prompt id) on top of this exact call.
//! - Flaky/blocked → delete this file, drop the bindings stub patch, park
//!   the idea permanently.
//!
//! windows-sys is used directly (GetForegroundWindow + GetWindowTextW)
//! instead of a wrapper crate: ~20 lines, no new transitive tree, and the
//! 0.59 line is already in Cargo.lock via the Tauri stack.

#[tauri::command]
#[specta::specta]
pub fn get_active_window_title() -> Result<String, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowTextW,
        };
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return Err("No foreground window detected.".to_string());
            }
            let mut buf = [0u16; 512];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len <= 0 {
                return Err(
                    "Foreground window has no title (or it is access-protected)."
                        .to_string(),
                );
            }
            Ok(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }
    #[cfg(not(windows))]
    {
        Err("Active-window detection is Windows-only in this spike.".to_string())
    }
}
