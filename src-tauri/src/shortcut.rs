//! Global push-to-talk (DESK-6).
//!
//! A browser tab hears the keyboard only while it has focus, which makes push-to-talk useless the
//! moment someone alt-tabs into the game they are talking about. The shell registers the
//! combination with the OS instead and reports both edges to the page, so the microphone opens
//! and closes no matter which window is in front.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// The event the page listens for; the payload says which edge it was.
pub const PUSH_TO_TALK_EVENT: &str = "orbit://push-to-talk";

/// What is currently registered, so a new key unregisters the old one instead of piling up.
static REGISTERED: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Copy, Serialize)]
struct PushToTalk {
    pressed: bool,
}

/// The full chord, as the page asks for it. A modifier alone is refused: the picker stores
/// `KeyboardEvent.code`, and one of those would swallow a bare key in every application.
pub fn parse_push_to_talk(accelerator: &str) -> Result<Shortcut, String> {
    let shortcut = Shortcut::try_from(accelerator).map_err(|error| error.to_string())?;
    if shortcut.mods.is_empty() {
        return Err(format!(
            "{accelerator} has no modifier: a global key without one steals typing from every app"
        ));
    }
    Ok(shortcut)
}

fn handler(app: &AppHandle, _shortcut: &Shortcut, event: ShortcutEvent) {
    let pressed = event.state == ShortcutState::Pressed;
    if let Err(error) = app.emit(PUSH_TO_TALK_EVENT, PushToTalk { pressed }) {
        eprintln!("[orbit-desktop] push-to-talk event was not sent: {error}");
    }
}

/// Announces that the key is up — the app is leaving, and a held microphone must not survive it.
pub fn release(app: &AppHandle) {
    let _ = app.emit(PUSH_TO_TALK_EVENT, PushToTalk { pressed: false });
}

/// Takes the shortcut over, or gives it back when `accelerator` is `None`.
///
/// Returns the combination the OS actually owns, so the page can say what the key really is.
/// A refused combination brings the previous one back: losing push-to-talk because the new
/// chord was taken is worse than not being able to change the chord.
#[tauri::command]
pub fn set_push_to_talk(
    app: AppHandle,
    accelerator: Option<String>,
) -> Result<Option<String>, String> {
    let manager = app.global_shortcut();
    let mut registered = REGISTERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let wanted = match accelerator.filter(|value| !value.trim().is_empty()) {
        Some(accelerator) => parse_push_to_talk(&accelerator)?,
        None => {
            if let Some(previous) = registered.take() {
                if let Err(error) = manager.unregister(previous.as_str()) {
                    eprintln!("[orbit-desktop] push-to-talk {previous} was not released: {error}");
                }
            }
            return Ok(None);
        }
    };

    let previous = registered.take();
    if let Some(previous) = previous.clone() {
        if let Err(error) = manager.unregister(previous.as_str()) {
            eprintln!("[orbit-desktop] push-to-talk {previous} was not released: {error}");
        }
    }

    match manager.on_shortcut(wanted, handler) {
        Ok(()) => {
            let canonical = wanted.into_string();
            *registered = Some(canonical.clone());
            Ok(Some(canonical))
        }
        Err(error) => {
            eprintln!("[orbit-desktop] push-to-talk {wanted} was refused: {error}");
            // Put the old chord back, so the key keeps working with the settings the page has.
            if let Some(previous) = previous {
                if let Ok(previous_shortcut) = parse_push_to_talk(&previous) {
                    if manager.on_shortcut(previous_shortcut, handler).is_ok() {
                        *registered = Some(previous);
                    }
                }
            }
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_chord_the_picker_produces() {
        let shortcut = parse_push_to_talk("CommandOrControl+Shift+Backquote")
            .expect("a chord with a modifier is valid");
        assert!(!shortcut.mods.is_empty());
    }

    #[test]
    fn refuses_a_bare_key_because_it_would_steal_typing() {
        for bare in ["Backquote", "KeyV", "Space"] {
            assert!(parse_push_to_talk(bare).is_err(), "{bare} must be refused");
        }
    }

    #[test]
    fn refuses_malformed_chords() {
        for broken in ["", "Shift+KeyQ+Alt", "CommandOrControl+", "NotAKey+Shift"] {
            assert!(
                parse_push_to_talk(broken).is_err(),
                "{broken} must be refused"
            );
        }
    }
}
