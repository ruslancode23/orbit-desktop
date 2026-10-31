//! The one window (DESK-5).
//!
//! Closing the window puts Orbit in the tray instead of quitting: the call keeps going and the
//! web socket keeps the person's place. Quitting is a tray menu item, never an accidental click
//! on a red button while a voice channel is open.

use tauri::{AppHandle, Manager, Runtime, WindowEvent};

/// Brings the window back from the tray, the Dock or a second launch.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Turns the close button into "hide to tray".
pub fn hide_on_close<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let hidden = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hidden.hide();
        }
    });
}
