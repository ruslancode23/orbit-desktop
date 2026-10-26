//! Spike instrumentation for DESK-1, active only in debug builds and only when
//! `ORBIT_DESKTOP_PROBE=1` is set.
//!
//! The spike has to answer "does this WebView support what orbit-web needs?" on a machine
//! nobody can attach a debugger to. Rather than reading a console by hand, one script asks
//! the WebView a list of questions after the page loads and prints the answers as a single
//! JSON line on stdout. It never runs in a release build: the command is not registered
//! there, and remote content is the very thing the capability file is trying to constrain.

use std::sync::Mutex;

use tauri::{
    webview::{PageLoadEvent, PageLoadPayload},
    WebviewWindow,
};

/// The document the probe already answered for. `tauri dev` fires a finished load for the
/// blank page first, and a reload or a second window deserves a fresh report, so the
/// document URL — not a boolean — decides whether this load is worth asking about.
static PROBED: Mutex<Option<String>> = Mutex::new(None);

const PROBE: &str = include_str!("../spike/probe.js");

/// True when the operator asked for the probe on this launch.
pub fn is_enabled() -> bool {
    matches!(std::env::var("ORBIT_DESKTOP_PROBE").as_deref(), Ok("1"))
}

/// Runs the probe after a real http(s) document has finished loading, once per document.
pub fn on_page_load(window: &WebviewWindow, payload: &PageLoadPayload<'_>) {
    if payload.event() != PageLoadEvent::Finished || !is_enabled() {
        return;
    }
    let url = payload.url().to_string();
    if !url.starts_with("http") {
        return;
    }
    // A poisoned lock only loses the "already probed" memory; asking twice is harmless.
    let mut probed = PROBED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if probed.as_deref() == Some(url.as_str()) {
        return;
    }
    *probed = Some(url);

    if let Err(error) = window.eval(PROBE) {
        eprintln!("[orbit-desktop] probe was not injected: {error}");
    }
}

/// Receives the probe's answers and prints them. Debug builds only.
#[tauri::command]
pub fn report_spike(report: serde_json::Value) {
    println!(
        "[probe] {}",
        serde_json::to_string(&report).unwrap_or_else(|_| "unserializable".to_string())
    );
}
