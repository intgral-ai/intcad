//! The Assistant's page as the app ships it: the files `packages/assistant-web` builds, the
//! policy they are served under, and nothing about a browser host.
//!
//! (Moved out of the WRY web view at the CEF cutover: the page is the same page.)

/// Agent output is untrusted text: the page may load only its own files and send nothing out.
/// The page's `<meta>` says the same; this header is the copy the page cannot change.
pub(crate) const POLICY: &str = "default-src 'none'; script-src 'self' openskyfall-assistant:; style-src 'self' openskyfall-assistant: 'unsafe-inline'; \
                      font-src 'self' openskyfall-assistant:; img-src 'self' openskyfall-assistant: data: blob:; connect-src 'none'; \
                      base-uri 'none'; form-action 'none'";

/// The page as `packages/assistant-web` builds it (see its README.md): name, type, bytes.
const FILES: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../../../packages/assistant-web/dist/index.html")),
    ("assistant.js", "text/javascript; charset=utf-8", include_bytes!("../../../packages/assistant-web/dist/assistant.js")),
    ("style.css", "text/css; charset=utf-8", include_bytes!("../../../packages/assistant-web/dist/style.css")),
    ("CommitMono-400-Regular.woff2", "font/woff2", include_bytes!("../../../packages/assistant-web/dist/CommitMono-400-Regular.woff2")),
    ("CommitMono-700-Regular.woff2", "font/woff2", include_bytes!("../../../packages/assistant-web/dist/CommitMono-700-Regular.woff2")),
    ("MonaSans-VariableFont_wdth-wght.ttf", "font/ttf", include_bytes!("../../../packages/assistant-web/dist/MonaSans-VariableFont_wdth-wght.ttf")),
];

/// One of the page's files.
pub(crate) fn page_file(name: &str) -> Option<(&'static str, &'static [u8])> {
    FILES.iter().find(|(file, _, _)| *file == name).map(|(_, kind, bytes)| (*kind, *bytes))
}

/// The page's security policy.
pub(crate) fn page_policy() -> &'static str {
    POLICY
}
