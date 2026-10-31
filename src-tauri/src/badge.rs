//! Unread count on the app icon (DESK-5).
//!
//! macOS paints the number itself (`set_badge_count`); Windows has no such call and takes an
//! overlay image instead. There the shell draws a red dot — the taskbar is 16 pixels tall, and a
//! dot reads at that size where a two-digit number does not. Say the word and we render digits
//! from an asset instead.

use tauri::{AppHandle, Manager};

/// The web app reports the total of unread mentions and messages; `0` clears the badge.
#[tauri::command]
pub fn set_unread(app: AppHandle, count: u32) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        {
            let value = if count == 0 {
                None
            } else {
                Some(i64::from(count))
            };
            if let Err(error) = window.set_badge_count(value) {
                eprintln!("[orbit-desktop] badge was not updated: {error}");
            }
        }

        #[cfg(target_os = "windows")]
        {
            let icon = if count == 0 { None } else { Some(unread_dot()) };
            if let Err(error) = window.set_overlay_icon(icon) {
                eprintln!("[orbit-desktop] overlay icon was not updated: {error}");
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (window, count);
    }
}

/// A 16×16 red circle with a soft edge, in the form the taskbar wants (RGBA, premultiplied alpha
/// is not required: the overlay is composited by the shell).
#[cfg(target_os = "windows")]
fn unread_dot() -> tauri::image::Image<'static> {
    const SIZE: u32 = 16;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let centre = (SIZE as f32 - 1.0) / 2.0;
    let radius = centre - 0.5;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let distance = (dx * dx + dy * dy).sqrt();
            // One pixel of feathering, so the dot does not look cut out on a Retina taskbar.
            let alpha = ((radius + 0.5 - distance).clamp(0.0, 1.0) * 255.0) as u8;
            let offset = ((y * SIZE + x) * 4) as usize;
            rgba[offset] = 235;
            rgba[offset + 1] = 59;
            rgba[offset + 2] = 59;
            rgba[offset + 3] = alpha;
        }
    }

    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}
