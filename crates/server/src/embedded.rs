//! Assets carried inside the executable so a release is one file.
//!
//! Two kinds, for two reasons:
//!
//! - The web build is served straight from memory; nothing has to exist on disk.
//! - The native bridges are Node ES modules that run as child processes, so they
//!   cannot stay in memory. They are written next to the server's own state on
//!   startup, replaced only when their contents differ.
//!
//! In debug builds `rust-embed` reads from the source tree instead, so editing
//! the web app or a bridge during development does not need a rebuild.
//! `TENDLY_WEB_DIR` and `TENDLY_NATIVE_BRIDGE` still override both, which
//! is what an installed layout or a packaging test uses.

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;
use std::path::{Path, PathBuf};

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../apps/web/dist"]
pub struct Web;

/// Only the modules the server actually spawns. Tests and fixtures are part of
/// the repository, not of what a user installs.
///
/// History loading imports the Claude Agent SDK by package name, so its entry
/// module travels with the bridges -- and so does its `package.json`, because
/// that is how Node finds the entry module. Carrying `sdk.mjs` alone left a
/// package directory with no manifest, and every import of it failed looking
/// for an `index.js` that was never there. The SDK's other bundles are not
/// imported and stay behind.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../packages/native-bridge"]
#[include = "*.mjs"]
#[include = "node_modules/@anthropic-ai/claude-agent-sdk/package.json"]
#[exclude = "*.test.mjs"]
#[exclude = "fixtures/*"]
#[exclude = "node_modules/@anthropic-ai/claude-agent-sdk/bridge.mjs"]
#[exclude = "node_modules/@anthropic-ai/claude-agent-sdk/node_modules/*"]
pub struct Bridge;

/// Whether anything was embedded at all. A `cargo build` that never ran the web
/// build produces an empty set, and saying so is better than serving 404s that
/// look like a routing bug.
pub fn has_web() -> bool {
    Web::iter().next().is_some()
}

/// Serve one embedded file, falling back to `index.html` so client-side routes
/// resolve the way `ServeDir`'s not-found service did.
pub fn serve_web(uri: &Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let nested = format!("{}/index.html", path.trim_end_matches('/'));
    // Resolve which file answers this request before describing it. The type has
    // to come from the file actually served, not from the path asked for: every
    // client-side route falls back to index.html and none of them carries an
    // extension, so guessing from the request labelled the whole application
    // `application/octet-stream` -- which, with `nosniff`, a browser downloads
    // instead of rendering. Assets were unaffected, so only the pages a person
    // opens were broken.
    let name = if Web::get(path).is_some() {
        path
    } else if Web::get(&nested).is_some() {
        nested.as_str()
    } else {
        "index.html"
    };
    let Some(file) = Web::get(name) else {
        return (
            StatusCode::NOT_FOUND,
            "Web assets were not built into this binary",
        )
            .into_response();
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let mut response = Response::new(Body::from(file.data.into_owned()));
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    response
}

/// Write the bridges into `directory`, creating it if needed.
///
/// Only files whose contents differ are rewritten, so a running server's
/// modules are not replaced on every start, and a host that pinned its own copy
/// through `TENDLY_NATIVE_BRIDGE` is never touched.
pub fn extract_bridge(directory: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    for name in Bridge::iter() {
        let Some(file) = Bridge::get(&name) else {
            continue;
        };
        let target = directory.join(name.as_ref());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let unchanged = std::fs::read(&target).is_ok_and(|existing| existing == file.data.as_ref());
        if !unchanged {
            std::fs::write(&target, file.data.as_ref())?;
        }
    }
    Ok(directory.join("history.mjs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_type(path: &str) -> String {
        let response = serve_web(&path.parse::<Uri>().unwrap());
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned())
            .unwrap_or_default()
    }

    /// A page is served as a page, whatever route the browser asked for.
    ///
    /// Every client-side route falls back to `index.html` and none of them has
    /// an extension, so describing the response by the requested path made the
    /// application `application/octet-stream`. Paired with the `nosniff` header
    /// the server already sends, a browser downloads that instead of rendering
    /// it: the released binary offered a file where the workspace should be.
    #[test]
    fn client_side_routes_are_served_as_html_rather_than_offered_as_a_download() {
        if !has_web() {
            return; // A build without the web assets has nothing to describe.
        }
        for route in ["/", "/workspace/anything", "/deep/client/route"] {
            assert_eq!(
                content_type(route),
                "text/html",
                "{route} must render as a page"
            );
        }
    }

    /// The fallback must not relabel real files: an asset keeps its own type.
    #[test]
    fn embedded_assets_keep_the_type_their_own_name_implies() {
        if !has_web() {
            return;
        }
        let assets: Vec<String> = Web::iter().map(|name| name.to_string()).collect();
        for (extension, expected) in [(".js", "text/javascript"), (".css", "text/css")] {
            let Some(asset) = assets.iter().find(|name| name.ends_with(extension)) else {
                continue;
            };
            assert_eq!(content_type(&format!("/{asset}")), expected, "{asset}");
        }
    }

    #[test]
    fn the_bridge_carries_the_modules_the_server_spawns_and_not_its_tests() {
        let names: Vec<_> = Bridge::iter().map(|name| name.to_string()).collect();
        // These are the entry points the server launches as child processes.
        for required in ["history.mjs", "account.mjs", "file-search.mjs"] {
            assert!(
                names.iter().any(|name| name == required),
                "{required} missing from {names:?}"
            );
        }
        // Their imports have to travel with them or the modules fail to load.
        for imported in [
            "clients.mjs",
            "json-lines.mjs",
            "account-claude.mjs",
            "account-codex.mjs",
            "account-common.mjs",
            "history-claude.mjs",
            "history-codex.mjs",
            "history-common.mjs",
            "native-spawn.mjs",
            "app-server.mjs",
        ] {
            assert!(
                names.iter().any(|name| name == imported),
                "{imported} missing"
            );
        }
        // A package is its manifest plus its entry; either alone fails to import.
        for sdk in [
            "node_modules/@anthropic-ai/claude-agent-sdk/package.json",
            "node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs",
        ] {
            assert!(names.iter().any(|name| name == sdk), "{sdk} missing");
        }
        assert!(
            !names
                .iter()
                .any(|name| name.ends_with(".test.mjs") || name.starts_with("fixtures/")),
            "tests and fixtures are repository content, not installed content: {names:?}"
        );
    }

    #[test]
    fn extracting_twice_leaves_the_files_alone_the_second_time() {
        let directory = std::env::temp_dir().join(format!("tendly-embed-{}", uuid::Uuid::new_v4()));
        let entry = extract_bridge(&directory).unwrap();
        assert!(entry.exists());
        let before = std::fs::metadata(&entry).unwrap().modified().unwrap();
        // A rewrite on every start would churn modules a running server loaded.
        extract_bridge(&directory).unwrap();
        assert_eq!(
            std::fs::metadata(&entry).unwrap().modified().unwrap(),
            before
        );
        // A changed file is replaced, so an upgrade actually takes effect.
        std::fs::write(&entry, b"stale").unwrap();
        extract_bridge(&directory).unwrap();
        assert_ne!(std::fs::read(&entry).unwrap(), b"stale");
        std::fs::remove_dir_all(&directory).ok();
    }
}
