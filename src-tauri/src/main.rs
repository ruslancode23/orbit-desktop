#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod badge;
mod origin;
#[cfg(debug_assertions)]
mod probe;
mod shortcut;
mod tray;
mod window;

use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

fn main() {
    let target = origin::resolve();
    match &target {
        origin::Target::Remote(url) => println!("[orbit-desktop] opening {url}"),
        origin::Target::Missing => {
            println!("[orbit-desktop] no origin configured, showing the bundled page")
        }
    }

    let mut builder = tauri::Builder::default()
        // The single-instance guard has to be registered first: a second launch must reach the
        // window that already exists instead of starting another copy of the app.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            window::show_main(app);
        }))
        // Size, place and maximised state survive a restart.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // DESK-3: the three things the web app cannot do itself. Notifications and autostart are
        // reached through PlatformAdapter; the OS details feed `system.describe()` in bug reports.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
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
        builder = builder.invoke_handler(tauri::generate_handler![
            probe::report_spike,
            badge::set_unread,
            shortcut::set_push_to_talk
        ]);
    }
    #[cfg(not(debug_assertions))]
    {
        builder = builder.invoke_handler(tauri::generate_handler![
            badge::set_unread,
            shortcut::set_push_to_talk
        ]);
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

            // Closing the window leaves the app in the tray; quitting is a tray menu item.
            window::hide_on_close(&window);
            tray::install(app.handle())?;

            #[cfg(debug_assertions)]
            if probe::ticks_enabled() {
                probe::watch_timers(&window);
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Orbit desktop failed to start");

    app.run(on_run_event);
}

/// A notification or dock click activates the app, but a window that was minimised stays in the
/// Dock, so it is brought back explicitly. The web side already focuses the window from the
/// notification's own action; this covers the activations the OS reports instead.
///
/// `Reopen` exists only on macOS — Windows brings the app forward itself on a toast click.
fn on_run_event(app: &tauri::AppHandle, event: RunEvent) {
    // A held key must not outlive the app: the page would keep the microphone open.
    if matches!(&event, RunEvent::ExitRequested { .. }) {
        shortcut::release(app);
    }

    #[cfg(target_os = "macos")]
    if matches!(&event, RunEvent::Reopen { .. }) {
        window::show_main(app);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = app;
}
