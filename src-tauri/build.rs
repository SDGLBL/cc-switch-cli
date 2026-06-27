//! Build-time embedding of the web dashboard frontend.
//!
//! When `CC_SWITCH_WEB_DIST` points at a built frontend (`dist-web` from the
//! cc-switch desktop project), every file under it is embedded into the binary
//! so `cc-switch web` is self-contained — no `--assets` needed. Text assets
//! (JS/CSS/SVG/…) are gzip-compressed at build time to keep the binary small;
//! `index.html` is left raw because the macOS-window chrome is injected into it
//! at runtime. Images/fonts are stored as-is (already compressed).
//!
//! When the env var is unset, the gitignored top-level `web-dist/` is embedded
//! if present (so a normal build is self-contained once the frontend has been
//! populated). If neither is available — e.g. a fresh checkout — the manifest
//! is empty and the server falls back to serving `--assets` from disk, so the
//! build still succeeds.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn content_type(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "json" | "map" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// gzip text-ish assets; `html` is intentionally excluded (served raw so the
/// runtime chrome injection can rewrite it), images/fonts are already compressed.
fn compressible(ext: &str) -> bool {
    matches!(
        ext,
        "js" | "mjs" | "css" | "svg" | "json" | "map" | "wasm" | "txt"
    )
}

fn collect(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, out);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
}

/// The built frontend at the top-level `web-dist/` — a gitignored build
/// artifact populated before a release build (`pnpm build:web` + copy, or via
/// `CC_SWITCH_WEB_DIST`). Used by default when the env var is unset; if it's
/// absent (e.g. a fresh checkout) the manifest is empty and the server falls
/// back to `--assets`.
fn default_vendored_dist() -> Option<PathBuf> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let vendored = manifest.parent()?.join("web-dist");
    vendored.join("index.html").is_file().then_some(vendored)
}

fn main() {
    println!("cargo:rerun-if-env-changed=CC_SWITCH_WEB_DIST");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_path = out_dir.join("web_embed.rs");

    // Prefer an explicit `CC_SWITCH_WEB_DIST`; otherwise embed the vendored
    // `assets/web-dist`. Either being absent leaves the manifest empty and the
    // server falls back to `--assets`.
    let dist = env::var("CC_SWITCH_WEB_DIST")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(default_vendored_dist);

    let mut entries = String::new();
    if let Some(dist) = dist {
        println!("cargo:rerun-if-changed={}", dist.display());

        let mut files = Vec::new();
        collect(&dist, &dist, &mut files);
        files.sort();

        let data_dir = out_dir.join("web_embed_data");
        fs::create_dir_all(&data_dir).unwrap();

        for (i, (rel, path)) in files.iter().enumerate() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let raw = fs::read(path).unwrap();
            let (bytes, gzipped) = if compressible(&ext) {
                let mut enc =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
                enc.write_all(&raw).unwrap();
                (enc.finish().unwrap(), true)
            } else {
                (raw, false)
            };
            let data_file = data_dir.join(format!("{i}.bin"));
            fs::write(&data_file, &bytes).unwrap();
            entries.push_str(&format!(
                "    EmbeddedAsset {{ path: {rel:?}, bytes: include_bytes!({data_file:?}), content_type: {ct:?}, gzipped: {gzipped} }},\n",
                ct = content_type(&ext),
            ));
        }
    }

    let code = format!(
        "/// One embedded frontend file. Generated by build.rs.\n\
         pub struct EmbeddedAsset {{\n\
         \x20   pub path: &'static str,\n\
         \x20   pub bytes: &'static [u8],\n\
         \x20   pub content_type: &'static str,\n\
         \x20   pub gzipped: bool,\n\
         }}\n\
         pub static EMBEDDED_WEB: &[EmbeddedAsset] = &[\n{entries}];\n"
    );
    fs::write(&manifest_path, code).unwrap();
}
