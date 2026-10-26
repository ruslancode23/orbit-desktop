//! Which origin the window opens.
//!
//! Variant A of ADR-0006: the desktop shell is a window on the web app's own origin, so
//! cookies, CORS and the WebSocket `Origin` check behave exactly as in a browser. The origin
//! is therefore configuration, not code:
//!
//! 1. `ORBIT_DESKTOP_URL` — a runtime override for development and QA;
//! 2. `http://localhost:5173` — Vite's dev server, the default for debug builds;
//! 3. `ORBIT_DESKTOP_ORIGIN` — baked in by `cargo build` for release builds;
//! 4. nothing — the window shows the bundled page that explains how to build with an origin.

use tauri::Url;

/// Vite's dev server, the same port orbit-web pins in `vite.config.ts`.
pub const DEV_ORIGIN: &str = "http://localhost:5173";

/// `ORBIT_DESKTOP_URL` at runtime; read on every start, never at compile time.
pub const ORIGIN_VAR: &str = "ORBIT_DESKTOP_URL";

/// `ORBIT_DESKTOP_ORIGIN` at compile time. `option_env!` keeps a missing value a compile-time `None`
/// instead of a panic, so debug builds work without any environment at all.
pub const BUILD_ORIGIN: Option<&str> = option_env!("ORBIT_DESKTOP_ORIGIN");

/// Where the window should load the web app from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An http(s) URL: the app itself.
    Remote(String),
    /// No usable origin was configured; the window shows the bundled explanation page.
    Missing,
}

/// Reads the environment and picks the target for this launch.
pub fn resolve() -> Target {
    let from_env = std::env::var(ORIGIN_VAR).ok();
    resolve_from(from_env.as_deref(), cfg!(debug_assertions), BUILD_ORIGIN)
}

/// Reads the same decision out of its inputs, so it can be tested without touching the process.
///
/// A value that is set but unusable is an error, not a reason to fall back: a typo in
/// `ORBIT_DESKTOP_URL` should show the explanation page, not quietly open the wrong app.
pub fn resolve_from(runtime: Option<&str>, debug: bool, build: Option<&str>) -> Target {
    if let Some(raw) = runtime.filter(|value| !value.trim().is_empty()) {
        return match normalize(raw) {
            Ok(url) => Target::Remote(url),
            Err(reason) => {
                eprintln!("[orbit-desktop] {ORIGIN_VAR}={raw}: {reason}");
                Target::Missing
            }
        };
    }

    if debug {
        return Target::Remote(DEV_ORIGIN.to_string());
    }

    match build.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => match normalize(raw) {
            Ok(url) => Target::Remote(url),
            Err(reason) => {
                eprintln!("[orbit-desktop] ORBIT_DESKTOP_ORIGIN={raw}: {reason}");
                Target::Missing
            }
        },
        None => Target::Missing,
    }
}

/// Accepts an absolute `http`/`https` URL without credentials, and drops the trailing slash
/// of a bare origin so the same value matches the capability allow-list and looks the same everywhere.
fn normalize(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let url = Url::parse(trimmed).map_err(|error| format!("not a URL: {error}"))?;

    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("scheme {other} is not http or https")),
    }
    if url.host_str().is_none() {
        return Err("no host".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("credentials in the URL are not allowed".to_string());
    }

    // `Url` keeps an empty path as `/` for http(s), so the slash is trimmed from the text.
    let text = url.to_string();
    let bare_origin = url.path() == "/" && url.query().is_none() && url.fragment().is_none();
    Ok(if bare_origin {
        text.trim_end_matches('/').to_string()
    } else {
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_defaults_to_the_vite_dev_server() {
        assert_eq!(
            resolve_from(None, true, None),
            Target::Remote(DEV_ORIGIN.to_string())
        );
    }

    #[test]
    fn runtime_override_wins_over_the_debug_default() {
        assert_eq!(
            resolve_from(Some("http://192.168.1.10:5173"), true, None),
            Target::Remote("http://192.168.1.10:5173".to_string())
        );
    }

    #[test]
    fn runtime_override_wins_over_the_build_origin() {
        assert_eq!(
            resolve_from(
                Some("https://stage.orbit.test"),
                false,
                Some("https://orbit.test")
            ),
            Target::Remote("https://stage.orbit.test".to_string())
        );
    }

    #[test]
    fn release_uses_the_build_origin() {
        assert_eq!(
            resolve_from(None, false, Some("https://orbit.test")),
            Target::Remote("https://orbit.test".to_string())
        );
    }

    #[test]
    fn release_without_an_origin_shows_the_explanation_page() {
        assert_eq!(resolve_from(None, false, None), Target::Missing);
    }

    #[test]
    fn blank_values_are_ignored() {
        assert_eq!(
            resolve_from(Some("   "), false, Some("https://orbit.test")),
            Target::Remote("https://orbit.test".to_string())
        );
        assert_eq!(
            resolve_from(None, true, None),
            Target::Remote(DEV_ORIGIN.to_string())
        );
    }

    #[test]
    fn a_bare_origin_loses_its_trailing_slash() {
        assert_eq!(
            resolve_from(Some("https://orbit.test/"), true, None),
            Target::Remote("https://orbit.test".to_string())
        );
    }

    #[test]
    fn a_path_survives_so_links_can_point_inside_the_app() {
        assert_eq!(
            resolve_from(Some("https://orbit.test/invite/abc"), true, None),
            Target::Remote("https://orbit.test/invite/abc".to_string())
        );
    }

    #[test]
    fn an_unusable_value_is_an_error_not_a_fallback() {
        for broken in [
            "localhost:5173",
            "ftp://orbit.test",
            "https://user:secret@orbit.test",
            "not a url",
        ] {
            assert_eq!(resolve_from(Some(broken), true, None), Target::Missing);
        }
    }
}
