#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod origin;
#[cfg(debug_assertions)]
mod probe;

use tauri::{WebviewUrl, WebviewWindowBuilder};

fn main() {
    let target = origin::resolve();
    match &target {
        origin::Target::Remote(url) => println!("[orbit-desktop] opening {url}"),
        origin::Target::Missing => {
            println!("[orbit-desktop] no origin configured, showing the bundled page")
        }
    }

    let mut builder = tauri::Builder::default();

    // The probe is a debug-build affordance: in a release the command is not registered at all,
    // so remote content cannot reach it even before the capability list is consulted.
    #[cfg(debug_assertions)]
    {
        builder = builder.invoke_handler(tauri::generate_handler![probe::report_spike]);
    }

    builder
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
        .run(tauri::generate_context!())
        .expect("Orbit desktop failed to start");
}
