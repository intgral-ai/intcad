//! Intcad's control channel identity for the shared token files (`solidcraft_control::auth`):
//! its name, data folder, `SOLIDCRAFT_` environment and default port (docs/control-protocol.md).

use solidcraft_control::auth::App;

/// Intcad's control channel: token files in `<data folder>/control` (`SOLIDCRAFT_CONTROL_DIR`
/// replaces it), `SOLIDCRAFT_CONTROL_TOKEN`, and 27878 first when the launch names no port.
pub const APP: App =
    App { name: "Intcad", data_dir: crate::recovery::data_dir, env: solidcraft_geom::env::var_os, utc: crate::crash::utc, default_port: 27878 };

// INT-1454: remove when MCP/UI adopt the control crate.
//
// The surface the unchanged app, CLI, MCP and UI crates still call. `solidcraft_control::auth`
// carries the same functions as `App` methods and free functions; these thin wrappers keep the
// engine's old API for this PR so those crates compile before their own control-crate migration.
pub use solidcraft_control::auth::{ClientToken, Listen, MIN_TOKEN_LEN, PORT_TRIES, TokenSource, bind_with_fallback, plan, tokens_match};

/// The port Intcad's control channel tries first ([`App::default_port`]).
pub const DEFAULT_PORT: u16 = APP.default_port;

/// The token file of the app listening on `port`.
pub fn token_path(port: u16) -> Option<std::path::PathBuf> {
    APP.token_path(port)
}

/// A token set on purpose with `SOLIDCRAFT_CONTROL_TOKEN`, if it is long enough.
pub fn env_token() -> Option<String> {
    APP.env_token()
}

/// The token a client sends to the app on `port`.
pub fn client_token(port: u16, explicit: Option<&str>) -> Option<ClientToken> {
    APP.client_token(port, explicit)
}

/// Write `token` to `path`, readable and writable only by the user.
pub fn write_token_file(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    APP.write_token_file(path, token)
}

/// Record `port` as the running app's.
pub fn write_current_port(port: u16) -> std::io::Result<std::path::PathBuf> {
    APP.write_current_port(port)
}

/// The port the discovery file names, if there is one.
pub fn read_current_port() -> Option<u16> {
    APP.read_current_port()
}

/// Remove the discovery file at exit, unless a later app has taken it over.
pub fn remove_current_port(port: u16, token: &str) {
    APP.remove_current_port(port, token)
}

/// Remove a token file this launch wrote, while it still holds `token`.
pub fn remove_token_file(path: &std::path::Path, token: &str) {
    solidcraft_control::auth::remove_token_file(path, token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_control::auth::{ClientToken, TokenSource};
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    /// A refused token names its file and when it was written, never the token.
    #[test]
    fn unauthorized_error_names_the_token_file() {
        let path = PathBuf::from("control/7878.token");
        let modified = Some(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000));
        let client = ClientToken { token: "secret-token".into(), file: Some(TokenSource { path: path.clone(), modified, writer: None }), app: APP };
        let error = client.explain_error("unauthorized: send the control token");
        assert!(error.contains(&format!("token from {}, written 2023-11-14T22:13:20Z (", path.display())), "{error}");
        assert!(error.contains("restart it or delete that file") && !error.contains("secret-token"), "{error}");
        assert_eq!(client.explain_error("unknown method"), "unknown method");
        let explicit = APP.client_token(0, Some("explicit-token")).unwrap();
        assert!(explicit.token == "explicit-token" && explicit.file.is_none());
        assert_eq!(explicit.explain_error("unauthorized"), "unauthorized");
    }
}
