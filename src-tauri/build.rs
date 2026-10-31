use std::{env, fs, path::Path};

/// Where the release build gets the web app from. Dev builds use `http://localhost:5173`
/// and may override it at runtime with `ORBIT_DESKTOP_URL`; a release has to bake the origin in,
/// because Tauri embeds the capability list at build time.
const ORIGIN_VAR: &str = "ORBIT_DESKTOP_ORIGIN";
const CAPABILITY: &str = "capabilities/main-remote.json";

fn main() {
    println!("cargo:rerun-if-env-changed={ORIGIN_VAR}");
    println!("cargo:rerun-if-changed={CAPABILITY}");

    guard_origin_is_in_capability();

    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["report_spike", "set_unread"]),
        ),
    )
    .expect("tauri-build failed");
}

/// A release build opens `ORBIT_DESKTOP_ORIGIN`, but the IPC allow-list lives in a static
/// capability file. If the two disagree the app starts and silently loses every native
/// feature, which is far harder to notice than a failed build. A glob pattern counts as
/// "cannot check here" and passes.
fn guard_origin_is_in_capability() {
    let Ok(origin) = env::var(ORIGIN_VAR) else {
        return;
    };
    let origin = origin.trim().trim_end_matches('/').to_string();
    if origin.is_empty() {
        return;
    }

    let path = Path::new(CAPABILITY);
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let capability: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));

    let urls = capability
        .get("remote")
        .and_then(|remote| remote.get("urls"))
        .and_then(|urls| urls.as_array())
        .unwrap_or_else(|| {
            panic!("{CAPABILITY} has no remote.urls, so {ORIGIN_VAR} would lose the IPC")
        });

    let allowed = urls
        .iter()
        .filter_map(|url| url.as_str())
        .map(|url| url.trim().trim_end_matches('/'))
        .collect::<Vec<_>>();

    if allowed.iter().any(|url| url.contains('*')) {
        return;
    }
    if !allowed.contains(&origin.as_str()) {
        panic!(
            "{ORIGIN_VAR}={origin} is not allowed by {CAPABILITY} (remote.urls: {allowed:?}).\n\
             Add it there: without the entry the window opens but every native feature is denied."
        );
    }
}
