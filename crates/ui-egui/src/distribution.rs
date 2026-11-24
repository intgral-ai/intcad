//! URLs for the public distribution channel. The GitHub values here are temporary until the
//! Migration Agent supplies the closed-source distribution addresses.
//!
//! Two repositories are named here, and they are not interchangeable: `intgral-ai/intcad-releases`
//! is the public distribution channel (release feed, installers, issue tracker), while
//! `intgral-ai/intcad` is the source repository that keeps the documentation and the preview
//! assets readable. The private source repository is never an update or download source.
//!
//! The update payload format may change with that channel. `update::Release::parse` currently
//! expects GitHub's `/releases/latest` JSON, including `tag_name`, `html_url` and
//! `assets[].browser_download_url`. The updater uses the asset URLs from that feed as supplied,
//! if they have the configured lexical prefix.

macro_rules! source_repo {
    () => {
        "intgral-ai/intcad"
    };
}

macro_rules! releases_repo {
    () => {
        "intgral-ai/intcad-releases"
    };
}

// Keep literal fragments in one place: `concat!` needs literals, not named `const` values.
macro_rules! source_url {
    ($($suffix:expr),* $(,)?) => {
        concat!("https://github.com/", source_repo!(), $($suffix),*)
    };
}

macro_rules! releases_url {
    ($($suffix:expr),* $(,)?) => {
        concat!("https://github.com/", releases_repo!(), $($suffix),*)
    };
}

macro_rules! raw_source_url {
    ($path:literal) => {
        concat!("https://raw.githubusercontent.com/", source_repo!(), "/main/", $path)
    };
}

macro_rules! releases_path {
    () => {
        "/releases"
    };
}

macro_rules! latest_path {
    () => {
        concat!(releases_path!(), "/latest")
    };
}

macro_rules! docs_path {
    () => {
        "/blob/main/docs/book/src"
    };
}

macro_rules! issues_path {
    () => {
        "/issues"
    };
}

/// The public distribution repository, exposed for updater test fixtures.
pub(crate) const REPO: &str = releases_repo!();

/// The update feed. The Migration Agent supplies the closed-source channel's public JSON URL.
pub(crate) const LATEST_API: &str = concat!("https://api.github.com/repos/", releases_repo!(), latest_path!());

/// The latest release page. The Migration Agent supplies the closed-source channel's public page.
pub(crate) const RELEASES_PAGE: &str = releases_url!(latest_path!());

/// The release history. The Migration Agent supplies the closed-source channel's public history page.
pub(crate) const RELEASE_HISTORY: &str = releases_url!(releases_path!());

/// A lexical prefix for release pages and downloads. The slash delimits the prefix;
/// the check does not parse URLs or normalize paths.
const RELEASE_PREFIX: &str = releases_url!(releases_path!(), "/");

/// The documentation landing page. It lives in the public source repository, which stays readable
/// after the closed-source switch, so this link keeps working without a new guide site.
pub(crate) const DOCS_URL: &str = source_url!(docs_path!(), "/SUMMARY.md");

/// The troubleshooting guide, in the same public source repository.
pub const TROUBLESHOOTING_URL: &str = source_url!(docs_path!(), "/guide/troubleshooting.md");

/// The report form. Reports must stay openable without access to the private source repository.
pub(crate) const ISSUE_URL: &str = releases_url!(issues_path!(), "/new");

// Shipped installer and web metadata embed their URLs. The update tests pin those copies.
#[cfg(test)]
pub(crate) const PROJECT_HOME: &str = source_url!("");
#[cfg(test)]
pub(crate) const ISSUE_TRACKER: &str = releases_url!(issues_path!());
#[cfg(test)]
pub(crate) const PROJECT_README: &str = source_url!("#readme");
#[cfg(test)]
pub(crate) const APPSTREAM_SCREENSHOT: &str = raw_source_url!("docs/screenshots/sample-plate.png");
#[cfg(test)]
pub(crate) const WEB_PREVIEW_IMAGE: &str = raw_source_url!("assets/branding/logo/open-skyfall-mark-1024.png");

pub(crate) fn is_release_url(url: &str) -> bool {
    url.starts_with(RELEASE_PREFIX)
}

/// Link to the notes for a validated `version` (the caller checks it). The Migration Agent
/// supplies the versioned release URL pattern for the closed-source channel.
pub(crate) fn version_release_url(version: &str) -> String {
    format!("{RELEASE_PREFIX}tag/v{version}")
}
