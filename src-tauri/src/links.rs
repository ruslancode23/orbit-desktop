//! Links that leave and re-enter the app (DESK-8).
//!
//! Two directions: a `http(s)` link on the page belongs in the OS browser, not inside a webview
//! that has no address bar and no back button; and an `orbit://invite/<код>` link opened from a
//! browser has to land in the running app, on the invitation.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use tauri::{webview::PageLoadEvent, AppHandle, Emitter, Runtime, Url};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

/// The page listens for this; the payload is the whole URL.
pub const DEEP_LINK_EVENT: &str = "orbit://open-url";

/// Set once a real document has loaded: before that, an emitted event reaches nobody.
static READY: AtomicBool = AtomicBool::new(false);
/// A link that arrived before the page could hear it.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// Opens a URL in the OS browser. A failure is worth a line in the log, nothing more.
pub fn open_external<R: Runtime>(app: &AppHandle<R>, url: &str) {
    if let Err(error) = app.opener().open_url(url, None::<String>) {
        eprintln!("[orbit-desktop] could not open {url}: {error}");
    }
}

/// True for a link the app must not render itself: another site, or a scheme that is not ours.
pub fn is_external(url: &Url, origin: &str) -> bool {
    match url.scheme() {
        "http" | "https" => match Url::parse(origin) {
            // A different host, port or scheme belongs to the browser.
            Ok(own) => own.origin() != url.origin(),
            Err(_) => true,
        },
        // The bundled fallback page and the empty view stay inside the window.
        "tauri" | "about" => false,
        // `mailto:`, `orbit://`, `tel:` — the OS decides who handles them.
        _ => true,
    }
}

/// Wires the scheme up: a running app gets the link, a cold start gets it after the first load.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            crate::window::show_main(&handle);
            deliver(&handle, url.to_string());
        }
    });

    // The installer registers the scheme in a release build; in development it has to be done here.
    if let Err(error) = app.deep_link().register_all() {
        eprintln!("[orbit-desktop] deep link scheme was not registered: {error}");
    }

    // A cold start carries the link in the arguments, not in an event.
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        if let Some(url) = urls.into_iter().next() {
            deliver(app, url.to_string());
        }
    }

    Ok(())
}

/// Records the URL, or hands it over if the page is already listening.
pub fn deliver<R: Runtime>(app: &AppHandle<R>, url: String) {
    if READY.load(Ordering::SeqCst) {
        emit(app, url);
        return;
    }
    *PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(url);
}

fn emit<R: Runtime>(app: &AppHandle<R>, url: String) {
    if let Err(error) = app.emit(DEEP_LINK_EVENT, url) {
        eprintln!("[orbit-desktop] deep link was not delivered: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("test URL")
    }

    #[test]
    fn our_own_origin_stays_in_the_window() {
        let origin = "http://localhost:5173";
        assert!(!is_external(&url("http://localhost:5173/s/1/2"), origin));
        assert!(!is_external(
            &url("http://localhost:5173/invite/abc"),
            origin
        ));
    }

    #[test]
    fn another_site_belongs_to_the_browser() {
        let origin = "http://localhost:5173";
        assert!(is_external(&url("https://example.com/page"), origin));
        // Same host, another port is another origin.
        assert!(is_external(&url("http://localhost:5174/"), origin));
        // The same link over https is a different origin too.
        assert!(is_external(&url("https://localhost:5173/"), origin));
    }

    #[test]
    fn other_schemes_go_to_the_os() {
        let origin = "https://orbit.invalid";
        assert!(is_external(&url("mailto:hi@orbit.invalid"), origin));
        assert!(is_external(&url("orbit://invite/abc"), origin));
        assert!(!is_external(&url("tauri://localhost/index.html"), origin));
        assert!(!is_external(&url("about:blank"), origin));
    }

    #[test]
    fn a_broken_origin_does_not_open_the_gate() {
        assert!(is_external(&url("https://example.com"), "not a url"));
    }
}

/// Every real document load: from now on an event reaches the page. A link that arrived earlier
/// is delivered here, because at startup nobody was listening yet.
pub fn on_page_load<R: Runtime>(app: &AppHandle<R>, event: PageLoadEvent) {
    if event != PageLoadEvent::Finished {
        return;
    }
    READY.store(true, Ordering::SeqCst);
    let pending = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(url) = pending {
        emit(app, url);
    }
}
