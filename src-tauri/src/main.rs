#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod origin;
#[cfg(debug_assertions)]
mod probe;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

fn main() {
    let target = origin::resolve();
    match &target {
        origin::Target::Remote(url) => println!("[orbit-desktop] opening {url}"),
        origin::Target::Missing => {
            println!("[orbit-desktop] no origin configured, showing the bundled page")
        }
    }

    let mut builder = tauri::Builder::default()
        // DESK-3: the three things the web app cannot do itself. Notifications and autostart are
        // reached through PlatformAdapter; the OS details feed `system.describe()` in bug reports.
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    // The probe is a debug-build affordance: in a release the command is not registered at all,
    // so remote content cannot reach it even before the capability list is consulted.
    #[cfg(debug_assertions)]
    {
        builder = builder.invoke_handler(tauri::generate_handler![probe::report_spike]);
    }

    let app = builder
        .setup(move |app| {
            // The window is declared in tauri.conf.json (size, title, minimums) but created here,
            // because its URL is not known until the origin is resolved.
            let mut window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .expect("tauri.conf.json must declare the main window");

            window_config.url = match &target {
                origin::Target::Remote(url) => WebviewUrl::External(url.parse()?),
                origin::Target::Missing => WebviewUrl::App("index.html".into()),
            };

            let builder = WebviewWindowBuilder::from_config(app.handle(), &window_config)?;

            #[cfg(debug_assertions)]
            let builder =
                builder.on_page_load(|window, payload| probe::on_page_load(&window, &payload));

            let window = builder.build()?;

            #[cfg(debug_assertions)]
            if probe::ticks_enabled() {
                probe::watch_timers(&window);
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Orbit desktop failed to start");

    app.run(|app, event| {
        // A notification click (or a dock/taskbar click) activates the app; make sure the window
        // is on screen and focused instead of staying minimised. Desktop notifications do not
        // report clicks to the plugin, so this is the closest the shell can get.
        if let RunEvent::Reopen { .. } = event {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });
}
