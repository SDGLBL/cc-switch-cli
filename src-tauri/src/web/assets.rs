use std::path::Path;

use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_status::SetStatus;

/// Build a static-file service for the built frontend at `dir`.
///
/// Unknown paths fall back to `index.html` so the single-page app can boot from
/// any URL. Path-traversal protection and content-type detection are handled by
/// `tower-http`'s `ServeDir`.
///
/// For the prototype the assets live on disk (built via `pnpm build:web` in the
/// cc-switch repo and pointed at with `--assets`). A future packaging step can
/// swap this for an embedded bundle (e.g. `rust-embed`) to keep cc-switch-cli a
/// single self-contained binary.
pub fn serve_dir(dir: &Path) -> ServeDir<SetStatus<ServeFile>> {
    let index = dir.join("index.html");
    ServeDir::new(dir).not_found_service(ServeFile::new(index))
}
