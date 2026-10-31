//! The tray icon (DESK-5): Orbit lives here while the window is closed.
//!
//! The menu is deliberately two items — "Открыть" and "Выход" — because everything else already
//! has a home inside the app. A left click on the icon opens the window; a right click opens the
//! menu (macOS shows it on a left click too, so the click handler checks the button).

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Runtime,
};

use crate::window;

/// The id is stable so the icon can be found and updated later (the unread badge uses the window,
/// but a future "доступен в голосе" state would be drawn here).
const TRAY_ID: &str = "orbit";

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Открыть", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Orbit")
        .menu(&menu)
        // Left click opens the window; the menu is for the right button.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::show_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}
