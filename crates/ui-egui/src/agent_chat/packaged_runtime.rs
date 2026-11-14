//! The Mastra workflow runtime installed with the application.
//!
//! The runtime lives in an application-relative `mastra/` directory: `runtime.json`, the built
//! JavaScript entry point, its production dependencies and a pinned `node`/`node.exe`. This
//! module resolves it from the running program (never the development checkout, `PATH`, `npx`
//! or a user-installed Node), then checks the receipt and every artifact digest before a turn
//! can start.
//!
//! `runtime.json` is the `RuntimeReceipt` of `docs/mastra-runtime.md`: it names the runtime and
//! protocol versions, target triple, pinned Node version, repository revision, contract digest,
//! provenance inventory and artifact digests. The host embeds the same reviewed identities at
//! build time (see `build.rs`), so a runtime built from another commit or contract, an incomplete
//! copy, a damaged file or an inventory the reviewed policy has not approved fails closed with an
//! actionable message. The current artifact is unapproved (#15), so that last check must fail
//! until legal review records an approval.

use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

/// A build without Git metadata carries this sentinel. It can never match a runtime receipt.
pub const UNBOUND_REVISION: &str = "0000000000000000000000000000000000000000";

/// The reviewed identities compiled into this host build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expectations {
    pub revision: String,
    pub contract_digest: String,
    pub policy_digest: String,
    pub sources_digest: String,
    pub runtime_version: String,
    pub node_version: String,
    pub approved_inventory_digests: Vec<String>,
    pub target: &'static str,
}

/// One launch of the packaged runtime. The executable is the bundled Node binary; the entry
/// point is passed as its only argument and the runtime's own directory is the working folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub executable: PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub cwd: PathBuf,
    pub root: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeError {
    NotInstalled { searched: PathBuf },
    UnsupportedTarget { target: String },
    InvalidManifest { detail: String },
    MissingFile { path: String },
    DigestMismatch { path: String },
    UnsafePath { path: String },
    RevisionMismatch { expected: String, found: String },
    UnboundHostRevision,
    ContractMismatch { expected: String, found: String },
    VersionMismatch { field: &'static str, expected: String, found: String },
    UnapprovedArtifact { inventory: String },
    Io { path: String },
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled { searched } => write!(
                f,
                "the installed application has no mastra/runtime.json (looked in {}); reinstall Intcad from a package that includes the Mastra runtime",
                searched.display()
            ),
            Self::UnsupportedTarget { target } => write!(
                f,
                "the built-in Mastra workflow is not available for {target}; Windows x86/arm64, Linux arm64 and other targets do not ship it in 1.0"
            ),
            Self::InvalidManifest { detail } => write!(f, "mastra/runtime.json is invalid: {detail}"),
            Self::MissingFile { path } => write!(f, "the installed Mastra runtime is incomplete: mastra/{path} is missing; reinstall Intcad"),
            Self::DigestMismatch { path } => write!(f, "mastra/{path} does not match its recorded SHA-256 digest; reinstall Intcad"),
            Self::UnsafePath { path } => write!(f, "mastra/runtime.json names an unsafe path ({path}); the package is damaged"),
            Self::RevisionMismatch { expected, found } => write!(
                f,
                "the Mastra runtime was built from repository revision {found}, but this Intcad is {expected}; install the matching package"
            ),
            Self::UnboundHostRevision => write!(
                f,
                "this Intcad build has no Git revision, so it cannot verify the installed Mastra runtime; use a release package or rebuild with SOLIDCRAFT_GIT_REVISION_OVERRIDE"
            ),
            Self::ContractMismatch { expected, found } => {
                write!(f, "the Mastra runtime carries contract digest {found}, but this Intcad requires {expected}; install the matching package")
            }
            Self::VersionMismatch { field, expected, found } => {
                write!(f, "the Mastra runtime's {field} is {found}, but this Intcad requires {expected}")
            }
            Self::UnapprovedArtifact { inventory } => write!(
                f,
                "the installed Mastra runtime inventory {inventory} has no distribution approval; it is not a releasable artifact until the legal and license review in #15 approves it"
            ),
            Self::Io { path } => write!(f, "could not read mastra/{path}; the installed package is incomplete or damaged"),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema_version: String,
    runtime_version: String,
    protocol_versions: Vec<String>,
    target_triple: String,
    node_version: String,
    repository_revision: String,
    contract_digest: String,
    policy_digest: String,
    source_records_digest: String,
    provenance_inventory: FileRef,
    artifact_digests: Vec<FileRef>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRef {
    path: String,
    digest: String,
}

/// The target triple this host was built for, or `None` where the workflow is not offered.
pub const fn host_target() -> Option<&'static str> {
    if cfg!(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu")) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc")) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        Some("x86_64-apple-darwin")
    } else {
        None
    }
}

/// The reviewed identities of this build. An empty approval list means the gate fails closed.
pub fn expectations() -> Expectations {
    let approved = env!("SOLIDCRAFT_MASTRA_APPROVED_INVENTORIES");
    Expectations {
        revision: env!("SOLIDCRAFT_GIT_REVISION").to_string(),
        contract_digest: env!("SOLIDCRAFT_MASTRA_CONTRACT_DIGEST").to_string(),
        policy_digest: env!("SOLIDCRAFT_MASTRA_POLICY_DIGEST").to_string(),
        sources_digest: env!("SOLIDCRAFT_MASTRA_SOURCES_DIGEST").to_string(),
        runtime_version: env!("SOLIDCRAFT_MASTRA_RUNTIME_VERSION").to_string(),
        node_version: env!("SOLIDCRAFT_MASTRA_NODE_VERSION").to_string(),
        approved_inventory_digests: approved.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect(),
        target: host_target().unwrap_or("unsupported"),
    }
}

/// Whether an application-relative `mastra/runtime.json` is present. This is a cheap settings
/// probe; [`resolve`] performs the full receipt and digest checks.
pub fn installed() -> bool {
    current_exe().is_some_and(|exe| roots(&exe).iter().any(|root| root.join("runtime.json").is_file()))
}

/// Resolve and validate the runtime beside the running program.
pub fn resolve_current(expectations: &Expectations) -> Result<Launch, RuntimeError> {
    let exe = current_exe().ok_or_else(|| RuntimeError::NotInstalled { searched: PathBuf::from("the application folder") })?;
    resolve(&exe, expectations)
}

/// Resolve the installed runtime without reading every artifact byte. This is the cheap check for
/// the window thread: missing, mismatched and unapproved runtimes still fail before a session
/// starts, while the multi-hundred-megabyte digest pass is deferred to [`verify_current`] when a
/// turn begins.
pub fn resolve_current_metadata(expectations: &Expectations) -> Result<Launch, RuntimeError> {
    let exe = current_exe().ok_or_else(|| RuntimeError::NotInstalled { searched: PathBuf::from("the application folder") })?;
    resolve_mode(&exe, expectations, false)
}

/// Hash the installed artifact against `runtime.json` before a turn. Called off the first turn's
/// setup so approval drift or a damaged file still stops modeling.
pub fn verify_current(expectations: &Expectations) -> Result<(), RuntimeError> {
    resolve_current(expectations).map(|_| ())
}

fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Candidate application-relative runtime directories, most specific first. They follow each
/// package's installed layout: macOS `Contents/Resources/mastra`, Linux FHS
/// `/usr/lib/solidcraft/mastra`, and `mastra/` next to a Windows executable. A universal macOS
/// app may also carry one folder per architecture under `mastra/<triple>`.
pub fn roots(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else { return Vec::new() };
    let mut roots = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(contents) = dir.parent() {
            roots.push(contents.join("Resources/mastra"));
            if let Some(target) = host_target() {
                roots.push(contents.join("Resources/mastra").join(target));
            }
        }
    } else if cfg!(target_os = "linux")
        && let Some(prefix) = dir.parent()
    {
        roots.push(prefix.join("lib/solidcraft/mastra"));
        if let Some(target) = host_target() {
            roots.push(prefix.join("lib/solidcraft/mastra").join(target));
        }
    }
    roots.push(dir.join("mastra"));
    if let Some(target) = host_target() {
        roots.push(dir.join("mastra").join(target));
    }
    let mut seen = std::collections::HashSet::new();
    roots.retain(|root| seen.insert(root.clone()));
    roots
}

/// Resolve and validate the runtime for an executable path. Tests use this directly.
pub fn resolve(exe: &Path, expectations: &Expectations) -> Result<Launch, RuntimeError> {
    resolve_mode(exe, expectations, true)
}

fn resolve_mode(exe: &Path, expectations: &Expectations, hash_files: bool) -> Result<Launch, RuntimeError> {
    let Some(target) = host_target() else {
        return Err(RuntimeError::UnsupportedTarget { target: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH) });
    };
    let roots = roots(exe);
    let Some(root) = roots.iter().find(|root| root.join("runtime.json").is_file()) else {
        return Err(RuntimeError::NotInstalled { searched: roots.first().cloned().unwrap_or_else(|| PathBuf::from("the application folder")) });
    };
    validate(root, target, expectations, hash_files)
}

fn validate(root: &Path, target: &'static str, expectations: &Expectations, hash_files: bool) -> Result<Launch, RuntimeError> {
    if expectations.revision == UNBOUND_REVISION {
        return Err(RuntimeError::UnboundHostRevision);
    }
    let manifest = root.join("runtime.json");
    let bytes = std::fs::read(&manifest).map_err(|_| RuntimeError::MissingFile { path: "runtime.json".into() })?;
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|error| RuntimeError::InvalidManifest { detail: error.to_string() })?;

    if receipt.schema_version != "1.0" {
        return Err(RuntimeError::InvalidManifest { detail: format!("schemaVersion is {}, expected 1.0", receipt.schema_version) });
    }
    if receipt.protocol_versions != ["1.0"] {
        return Err(RuntimeError::InvalidManifest { detail: "protocolVersions is not exactly [\"1.0\"]".into() });
    }
    if receipt.target_triple != target {
        return Err(RuntimeError::VersionMismatch { field: "target triple", expected: target.into(), found: receipt.target_triple.clone() });
    }
    if receipt.node_version != expectations.node_version {
        return Err(RuntimeError::VersionMismatch {
            field: "Node version",
            expected: expectations.node_version.clone(),
            found: receipt.node_version.clone(),
        });
    }
    if receipt.runtime_version != expectations.runtime_version {
        return Err(RuntimeError::VersionMismatch {
            field: "runtime version",
            expected: expectations.runtime_version.clone(),
            found: receipt.runtime_version.clone(),
        });
    }
    if receipt.repository_revision != expectations.revision {
        return Err(RuntimeError::RevisionMismatch { expected: expectations.revision.clone(), found: receipt.repository_revision.clone() });
    }
    if receipt.contract_digest != expectations.contract_digest {
        return Err(RuntimeError::ContractMismatch { expected: expectations.contract_digest.clone(), found: receipt.contract_digest.clone() });
    }
    if receipt.policy_digest != expectations.policy_digest {
        return Err(RuntimeError::VersionMismatch {
            field: "policy digest",
            expected: expectations.policy_digest.clone(),
            found: receipt.policy_digest.clone(),
        });
    }
    if receipt.source_records_digest != expectations.sources_digest {
        return Err(RuntimeError::VersionMismatch {
            field: "source-records digest",
            expected: expectations.sources_digest.clone(),
            found: receipt.source_records_digest.clone(),
        });
    }

    // The inventory is the provenance record admission approved; a missing or unapproved one
    // stops the turn. The reviewed policy currently has no approvals, so this fails closed.
    if receipt.provenance_inventory.path != "inventory.json" {
        return Err(RuntimeError::InvalidManifest { detail: "provenanceInventory.path must be inventory.json".into() });
    }
    safe_path(&receipt.provenance_inventory.path)?;
    if !is_digest(&receipt.provenance_inventory.digest) {
        return Err(RuntimeError::InvalidManifest { detail: "provenanceInventory.digest is not sha256:<64 lowercase hex>".into() });
    }
    if !expectations.approved_inventory_digests.iter().any(|digest| digest == &receipt.provenance_inventory.digest) {
        return Err(RuntimeError::UnapprovedArtifact { inventory: receipt.provenance_inventory.digest.clone() });
    }
    verify_file(root, &receipt.provenance_inventory, hash_files)?;

    if receipt.artifact_digests.len() > 100_000 {
        return Err(RuntimeError::InvalidManifest { detail: "artifactDigests has too many entries".into() });
    }
    let mut seen = std::collections::HashSet::new();
    for entry in &receipt.artifact_digests {
        safe_path(&entry.path)?;
        if !seen.insert(entry.path.as_str()) {
            return Err(RuntimeError::InvalidManifest { detail: format!("artifactDigests repeats {}", entry.path) });
        }
        if !is_digest(&entry.digest) {
            return Err(RuntimeError::InvalidManifest { detail: format!("artifactDigests digest for {} is not SHA-256", entry.path) });
        }
        verify_file(root, entry, hash_files)?;
    }

    // Shape checks independent of the digest list: an artifact missing its entry point, Node,
    // dependencies, contract or notices is incomplete even if every listed digest matches.
    let node = if target.ends_with("windows-msvc") { "node.exe" } else { "node" };
    for path in [
        "dist/src/main.js",
        "package.json",
        "inventory.json",
        "schema/mastra-runtime/v1/manifest.json",
        "licenses/LICENSES.txt",
        "notices/NOTICE.txt",
    ] {
        if !root.join(path).is_file() {
            return Err(RuntimeError::MissingFile { path: path.into() });
        }
    }
    for path in ["dist/src/main.js", "package.json", "schema/mastra-runtime/v1/manifest.json", "licenses/LICENSES.txt", "notices/NOTICE.txt", node] {
        if !seen.contains(path) {
            return Err(RuntimeError::InvalidManifest { detail: format!("artifactDigests omits required file {path}") });
        }
    }
    let contract_path = "schema/mastra-runtime/v1/manifest.json";
    if let Some(entry) = receipt.artifact_digests.iter().find(|entry| entry.path == contract_path)
        && entry.digest != receipt.contract_digest
    {
        return Err(RuntimeError::InvalidManifest { detail: "artifactDigests contract manifest digest differs from contractDigest".into() });
    }
    if hash_files && hash_file(root, contract_path)? != receipt.contract_digest {
        return Err(RuntimeError::DigestMismatch { path: contract_path.into() });
    }
    if !root.join("node_modules").is_dir() {
        return Err(RuntimeError::MissingFile { path: "node_modules".into() });
    }
    let executable = root.join(node);
    if !executable.is_file() {
        return Err(RuntimeError::MissingFile { path: node.into() });
    }
    Ok(Launch { executable, args: vec![root.join("dist/src/main.js").into_os_string()], cwd: root.to_path_buf(), root: root.to_path_buf() })
}

fn verify_file(root: &Path, file: &FileRef, hash: bool) -> Result<(), RuntimeError> {
    let path = safe_join(root, &file.path)?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| RuntimeError::MissingFile { path: file.path.clone() })?;
    if !metadata.file_type().is_file() {
        return Err(RuntimeError::MissingFile { path: file.path.clone() });
    }
    if hash && hash_file(root, &file.path)? != file.digest {
        return Err(RuntimeError::DigestMismatch { path: file.path.clone() });
    }
    Ok(())
}

fn hash_file(root: &Path, relative: &str) -> Result<String, RuntimeError> {
    let path = safe_join(root, relative)?;
    let mut handle = std::fs::File::open(&path).map_err(|_| RuntimeError::Io { path: relative.into() })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = handle.read(&mut buffer).map_err(|_| RuntimeError::Io { path: relative.into() })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, RuntimeError> {
    safe_path(relative)?;
    Ok(root.join(relative))
}

/// The contract's relative-path rule, narrowed to what a shipped artifact may name. A hostile or
/// damaged manifest is rejected, never repaired.
fn safe_path(path: &str) -> Result<(), RuntimeError> {
    let bad = |path: &str| RuntimeError::UnsafePath { path: path.into() };
    if path.is_empty() || path.len() > 1024 || !path.is_ascii() {
        return Err(bad(path));
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') || path.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(bad(path));
    }
    const DEVICE: &[&str] = &[
        "con", "conin$", "conout$", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
        "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    for part in path.split('/') {
        let stem = part.split('.').next().unwrap_or(part);
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.ends_with(' ')
            || part.bytes().any(|b| matches!(b, b'<' | b'>' | b'"' | b'|' | b'?' | b'*'))
            || part.eq_ignore_ascii_case("ee")
            || DEVICE.iter().any(|device| stem.eq_ignore_ascii_case(device))
        {
            return Err(bad(path));
        }
    }
    if Path::new(path).components().any(|component| !matches!(component, Component::Normal(_))) {
        return Err(bad(path));
    }
    Ok(())
}

fn is_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    struct Fixture {
        dir: PathBuf,
        exe: PathBuf,
        root: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("sc-packaged-runtime-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (exe, root) = if cfg!(target_os = "macos") {
            (dir.join("Contents/MacOS/SolidCraft"), dir.join("Contents/Resources/mastra"))
        } else if cfg!(target_os = "windows") {
            (dir.join("bin/solidcraft.exe"), dir.join("bin/mastra"))
        } else {
            (dir.join("usr/bin/solidcraft"), dir.join("usr/lib/solidcraft/mastra"))
        };
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(root.join("node_modules/example")).unwrap();
        std::fs::create_dir_all(root.join("licenses")).unwrap();
        std::fs::create_dir_all(root.join("notices")).unwrap();
        std::fs::create_dir_all(root.join("schema/mastra-runtime/v1")).unwrap();
        write(&root.join("node"), b"#!/bin/sh\nexit 0\n");
        write(&root.join("dist/src/main.js"), b"console.log('runtime');\n");
        write(&root.join("package.json"), b"{\"name\":\"runtime\"}\n");
        write(&root.join("schema/mastra-runtime/v1/manifest.json"), b"{}\n");
        write(&root.join("licenses/LICENSES.txt"), b"licences\n");
        write(&root.join("notices/NOTICE.txt"), b"notice\n");
        write(&root.join("node_modules/example/index.js"), b"module\n");
        write(&root.join("inventory.json"), b"{}\n");
        let entries =
            ["dist/src/main.js", "package.json", "schema/mastra-runtime/v1/manifest.json", "licenses/LICENSES.txt", "notices/NOTICE.txt", "node"];
        let artifact: Vec<serde_json::Value> = entries
            .iter()
            .map(|path| {
                let bytes = std::fs::read(root.join(path)).unwrap();
                serde_json::json!({"path": path, "digest": digest(&bytes)})
            })
            .collect();
        let inventory = digest(b"{}\n");
        let receipt = serde_json::json!({
            "schemaVersion": "1.0",
            "runtimeVersion": "0.1.0",
            "protocolVersions": ["1.0"],
            "targetTriple": host_target().unwrap(),
            "nodeVersion": "22.23.4",
            "repositoryRevision": "0123456789abcdef0123456789abcdef01234567",
            "contractDigest": digest(b"{}\n"),
            "policyDigest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "sourceRecordsDigest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "provenanceInventory": {"path": "inventory.json", "digest": inventory},
            "artifactDigests": artifact,
        });
        write(&root.join("runtime.json"), serde_json::to_vec_pretty(&receipt).unwrap().as_slice());
        Fixture { dir, exe, root }
    }

    fn expected(inventory: &str) -> Expectations {
        Expectations {
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
            contract_digest: digest(b"{}\n"),
            policy_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            sources_digest: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
            runtime_version: "0.1.0".into(),
            node_version: "22.23.4".into(),
            approved_inventory_digests: vec![inventory.into()],
            target: host_target().unwrap_or("unsupported"),
        }
    }

    fn approved(_fixture: &Fixture) -> Expectations {
        expected(&digest(b"{}\n"))
    }

    #[test]
    fn a_matching_installed_runtime_launches_the_bundled_node_and_entry_point() {
        let fixture = fixture("valid");
        let launch = resolve(&fixture.exe, &approved(&fixture)).unwrap();
        assert_eq!(launch.executable, fixture.root.join("node"));
        assert_eq!(launch.args, vec![fixture.root.join("dist/src/main.js").into_os_string()]);
        assert_eq!(launch.cwd, fixture.root);
    }

    #[test]
    fn no_application_relative_runtime_fails_before_a_session() {
        let fixture = fixture("missing");
        std::fs::remove_file(fixture.root.join("runtime.json")).unwrap();
        // A Node installation in PATH or beside the program is not a packaged runtime.
        write(&fixture.exe.parent().unwrap().join("node"), b"#!/bin/sh\nexit 0\n");
        assert!(matches!(resolve(&fixture.exe, &approved(&fixture)), Err(RuntimeError::NotInstalled { .. })));
    }

    #[test]
    fn a_different_runtime_or_protocol_version_fails_closed() {
        for (key, value) in [("nodeVersion", "24.0.0"), ("runtimeVersion", "9.9.9")] {
            let fixture = fixture(key);
            let path = fixture.root.join("runtime.json");
            let mut receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            receipt[key] = serde_json::json!(value);
            write(&path, serde_json::to_vec(&receipt).unwrap().as_slice());
            assert!(matches!(resolve(&fixture.exe, &approved(&fixture)), Err(RuntimeError::VersionMismatch { .. })));
        }
        let fixture = fixture("protocol");
        let path = fixture.root.join("runtime.json");
        let mut receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        receipt["protocolVersions"] = serde_json::json!(["2.0"]);
        write(&path, serde_json::to_vec(&receipt).unwrap().as_slice());
        assert!(matches!(resolve(&fixture.exe, &approved(&fixture)), Err(RuntimeError::InvalidManifest { .. })));
    }

    #[test]
    fn another_revision_or_contract_fails_closed() {
        let revision = fixture("revision");
        let path = revision.root.join("runtime.json");
        let mut receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        receipt["repositoryRevision"] = serde_json::json!("fedcba9876543210fedcba9876543210fedcba98");
        write(&path, serde_json::to_vec(&receipt).unwrap().as_slice());
        assert!(matches!(resolve(&revision.exe, &approved(&revision)), Err(RuntimeError::RevisionMismatch { .. })));

        let contract = fixture("contract");
        let path = contract.root.join("runtime.json");
        let mut receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        receipt["contractDigest"] = serde_json::json!(digest(b"another contract"));
        write(&path, serde_json::to_vec(&receipt).unwrap().as_slice());
        assert!(matches!(resolve(&contract.exe, &approved(&contract)), Err(RuntimeError::ContractMismatch { .. })));
    }

    #[test]
    fn an_unapproved_or_damaged_inventory_fails_closed() {
        let unapproved = fixture("unapproved");
        // The reviewed policy currently has no approvals, so an empty host list must refuse.
        let mut expectations = approved(&unapproved);
        expectations.approved_inventory_digests.clear();
        assert!(matches!(resolve(&unapproved.exe, &expectations), Err(RuntimeError::UnapprovedArtifact { .. })));

        let damaged = fixture("damaged");
        let path = damaged.root.join("inventory.json");
        write(&path, b"{\"changed\":true}\n");
        assert!(matches!(resolve(&damaged.exe, &approved(&damaged)), Err(RuntimeError::DigestMismatch { .. })));
    }

    #[test]
    fn an_incomplete_tree_and_a_tampered_file_fail_closed() {
        let incomplete = fixture("incomplete");
        std::fs::remove_dir_all(incomplete.root.join("node_modules")).unwrap();
        assert!(matches!(resolve(&incomplete.exe, &approved(&incomplete)), Err(RuntimeError::MissingFile { .. })));

        let tampered = fixture("tampered");
        write(&tampered.root.join("dist/src/main.js"), b"tampered\n");
        assert!(matches!(resolve(&tampered.exe, &approved(&tampered)), Err(RuntimeError::DigestMismatch { .. })));
    }

    #[test]
    fn an_unknown_receipt_field_is_rejected() {
        let fixture = fixture("unknown");
        let path = fixture.root.join("runtime.json");
        let mut receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        receipt["extra"] = serde_json::json!(true);
        write(&path, serde_json::to_vec(&receipt).unwrap().as_slice());
        assert!(matches!(resolve(&fixture.exe, &approved(&fixture)), Err(RuntimeError::InvalidManifest { .. })));
    }

    #[test]
    fn an_unbound_source_build_refuses_the_installed_runtime() {
        let fixture = fixture("unbound");
        let mut expectations = approved(&fixture);
        expectations.revision = UNBOUND_REVISION.into();
        assert!(matches!(resolve(&fixture.exe, &expectations), Err(RuntimeError::UnboundHostRevision)));
    }

    #[test]
    fn hostile_manifest_paths_are_rejected() {
        for path in ["../node", "/etc/passwd", "ee/index.js", "dist\\main.js", "C:/node", "node_modules/ee/x.js"] {
            assert!(safe_path(path).is_err(), "{path}");
        }
        assert!(safe_path("dist/src/main.js").is_ok());
    }

    #[test]
    fn the_settings_probe_only_needs_the_receipt() {
        let fixture = fixture("probe");
        assert!(installed_for(&fixture.exe));
        std::fs::remove_file(fixture.root.join("runtime.json")).unwrap();
        assert!(!installed_for(&fixture.exe));
    }

    fn installed_for(exe: &Path) -> bool {
        roots(exe).iter().any(|root| root.join("runtime.json").is_file())
    }

    /// Starting the bundled Node through the existing process owner and stopping it must leave no
    /// helper process behind. This is the same cleanup path the live runtime client uses.
    #[test]
    #[cfg(unix)]
    fn the_packaged_node_is_started_in_its_own_group_and_stopped_with_its_children() {
        use super::super::process::{AgentProcess, FromAgent};
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        let fixture = fixture("cleanup");
        let node = fixture.root.join("node");
        let body = b"#!/bin/sh\nsleep 300 &\necho $!\nwait\n";
        write(&node, body);
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The receipt was made before this test replaced the placeholder node.
        let manifest = fixture.root.join("runtime.json");
        let mut receipt: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        for entry in receipt["artifactDigests"].as_array_mut().unwrap() {
            if entry["path"] == "node" {
                entry["digest"] = serde_json::json!(digest(body));
            }
        }
        write(&manifest, serde_json::to_vec(&receipt).unwrap().as_slice());
        let launch = resolve(&fixture.exe, &approved(&fixture)).unwrap();
        let mut argv = vec![launch.executable.to_string_lossy().into_owned()];
        argv.extend(launch.args.iter().map(|arg| arg.to_string_lossy().into_owned()));
        let mut process = AgentProcess::spawn(&argv, &launch.cwd, &[], || {}).unwrap();
        let leader = process.pid().unwrap();
        let FromAgent::Line(line) = process.from_agent.recv_timeout(Duration::from_secs(10)).unwrap() else {
            panic!("the bundled node did not print its helper")
        };
        let helper: u32 = line.trim().parse().unwrap();
        process.shutdown(true);
        let deadline = Instant::now() + Duration::from_secs(8);
        while (alive(leader) || alive(helper)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(leader), "the bundled node survived shutdown");
        assert!(!alive(helper), "a child of the bundled node survived shutdown");
    }

    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            return !stat.rsplit(')').next().is_some_and(|s| s.trim_start().starts_with('Z'));
        }
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
    }
}
