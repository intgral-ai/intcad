//! Dependency layering rules (plan/architecture.md §3).
//!
//! Adapted from CADCraft's xtask (MIT OR Apache-2.0), see ATTRIBUTION.md.
//!
//! The rule engine works on a small, metadata-independent model so it can be
//! unit-tested; `from_metadata` builds that model from `cargo metadata`.

use std::fs;
use std::path::Path;

use serde_json::Value;

/// Where a workspace crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Regular layered crate.
    Layer(u8),
    /// Layer 0, and additionally may depend on no workspace crate at all.
    Standalone,
    /// Test tooling: may depend on anything up to L5 (it sits at L6 for rule
    /// purposes); other crates may use it only as a dev-dependency.
    Testkit,
    /// Binaries and build tooling: exempt from the rules.
    Exempt,
}

impl Class {
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) => Some(l),
            Class::Standalone => Some(0),
            Class::Testkit => Some(6),
            Class::Exempt => None,
        }
    }
}

/// The layering table. Names are package names without the `solidcraft-`
/// prefix.
pub const TABLE: &[(&str, Class)] = &[
    ("geom", Class::Layer(0)),
    // the native AI harness task contract/controller (UI-independent; no workspace dependencies)
    ("agent-task", Class::Layer(0)),
    // the pure DFM rule layer translated from intdfm (no workspace dependencies, no OCP)
    ("dfm-rules", Class::Layer(0)),
    // colour rules and data any crate may share (no workspace dependencies)
    ("color", Class::Layer(0)),
    // a length in a design unit with its decimal places (no workspace dependencies)
    ("display-units", Class::Layer(0)),
    // versioned file formats: the tag, the version registry and its container (no app types)
    ("format", Class::Layer(0)),
    // the control channel shared by apps: token files, the loopback server and its requests, no UI
    ("control", Class::Layer(0)),
    // MCP machinery any app may share: JSON-RPC, tool schemas and checks, image results, no app types
    ("mcp-server", Class::Layer(0)),
    // 3D mouse descriptors and reports (no workspace dependencies; the apps read the device)
    ("space-mouse", Class::Layer(0)),
    // the derive macros (proc-macro, no workspace dependencies)
    ("derive", Class::Standalone),
    // the 3Dconnexion driver's client API on macOS (no workspace dependencies)
    ("3dx-client", Class::Standalone),
    // the profiler: timed spans for every layer, the kernel's calls to commands (no workspace dependencies)
    ("profile", Class::Standalone),
    ("sketch", Class::Layer(1)),
    // the B-rep kernel, solidcore (the Rust port of NVIDIA's SMLib, ADR 0002); only the kernel uses it
    ("smlib", Class::Layer(1)),
    ("kernel", Class::Layer(1)),
    // the shared viewport math and headless picture the renderer and the document use
    ("viewport", Class::Layer(1)),
    ("render", Class::Layer(1)),
    // multibody dynamics (rigid bodies, joints, forces, time integration; geom only)
    ("dynamics", Class::Layer(1)),
    ("doc", Class::Layer(2)),
    ("io", Class::Layer(3)),
    ("engine", Class::Layer(4)),
    // real engine/snapshot/validation ports for the native AI harness task controller (no UI)
    ("agent-task-engine", Class::Layer(4)),
    ("ui-egui", Class::Layer(5)),
    ("gpu", Class::Layer(5)),
    // the viewport renderer's GPU work on wgpu: pipelines, offscreen pictures, adapter selection,
    // and display plans evaluated in a compute shader; wgpu is a UI toolkit, so it sits at L5
    ("viewport-gpu", Class::Layer(5)),
    // the viewport in an egui window: egui-wgpu paint callbacks and install
    ("viewport-egui", Class::Layer(5)),
    ("mcp", Class::Layer(5)),
    ("testkit", Class::Testkit),
    // apps and tooling
    ("solidcraft", Class::Exempt),
    ("cli", Class::Exempt),
    ("web", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Explicit orderings *within* a layer (earlier may be used by later).
/// The viewport draws display plans the GPU crate evaluates and draws with its pipelines; the MCP
/// server's tests draw screenshots with them. The kernel reads and writes STEP through `solidcore-step`.
pub const INTRA_LAYER_ORDER: &[&[&str]] = &[
    // The viewport's GPU work is drawn with by the app's GPU crate, the egui paint callbacks and
    // the UI itself (its viewport hands egui's device and pass to the pipelines).
    &["viewport-gpu", "gpu"],
    &["viewport-gpu", "viewport-egui"],
    &["viewport-gpu", "ui-egui"],
    &["viewport-egui", "ui-egui"],
    &["gpu", "ui-egui"],
    &["gpu", "mcp"],
    // The UI hosts the chat MCP server over the Mastra bridge lane (#9); the server never
    // depends on the UI.
    &["mcp", "ui-egui"],
    // The renderer draws the viewport's camera, pictures and shading; it builds on them.
    &["viewport", "render"],
    // The task controller's real engine ports sit beside the engine, below the UI.
    &["engine", "agent-task-engine"],
    &["smlib", "kernel"],
];

fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER.iter().any(|chain| match (chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    })
}

/// External crates that constitute a UI toolkit / windowing dependency.
/// Entries ending in `*` are prefixes.
pub const UI_CRATES: &[&str] = &["egui", "eframe", "egui-wgpu", "wgpu", "winit", "egui_kittest", "rfd", "bevy*"];

/// First layer allowed to use UI crates.
pub const UI_MIN_LAYER: u8 = 5;

pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("solidcraft-").unwrap_or(pkg)
}

pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// `true` if the dependency is a workspace member.
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String },
    UiBelowL5 { krate: String, dep: String, layer: u8 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE (see plan/architecture.md §3)")
            }
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => {
                write!(f, "{krate}: standalone crate must not depend on workspace crate {dep}")
            }
            Violation::TestkitAsNormalDep { krate } => {
                write!(f, "{krate}: solidcraft-testkit may only be a dev-dependency")
            }
            Violation::UiBelowL5 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L5+")
            }
        }
    }
}

/// Check all rules. Returns violations sorted for stable output.
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        if class == Class::Exempt {
            continue;
        }
        let layer = class.layer().unwrap_or(0);
        for d in &c.deps {
            // Self dev-dependencies (e.g. to enable features in tests) are fine.
            if d.name == c.name {
                continue;
            }
            if d.workspace {
                if class == Class::Standalone {
                    out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                    continue;
                }
                match classify(&d.name) {
                    // Unregistered deps are reported on their own entry.
                    None => {}
                    Some(Class::Testkit) if d.kind != DepKind::Dev => {
                        out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() });
                    }
                    Some(Class::Testkit) => {}
                    Some(dc) => {
                        let to = dc.layer().unwrap_or(u8::MAX);
                        if to >= layer && !(to == layer && intra_layer_allowed(&c.name, &d.name)) {
                            out.push(Violation::Upward {
                                krate: c.name.clone(),
                                dep: d.name.clone(),
                                from: layer,
                                to: if to == u8::MAX { 7 } else { to },
                                kind: d.kind,
                            });
                        }
                    }
                }
            } else if layer < UI_MIN_LAYER && is_ui_crate(&d.name) {
                out.push(Violation::UiBelowL5 { krate: c.name.clone(), dep: d.name.clone(), layer });
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

/// Build the model from `cargo metadata --format-version 1 --no-deps`.
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("package without name")?.to_owned();
        let mut deps = Vec::new();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dname = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = match d["kind"].as_str() {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname.as_str()) || d["path"].is_string();
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Standalone) => "L0 standalone".into(),
        Some(Class::Testkit) => "testkit".into(),
        Some(Class::Exempt) => "exempt".into(),
        None => "UNREGISTERED".into(),
    }
}

// Standalone manifest rule adapted from upstream SolidCraft
// 17daee946c339487c5393f0c03aaa9dcb8ca6652, xtask/src/layers.rs (MIT OR Apache-2.0).
const STANDALONE_RULE: &str = "solidcore must build on its own; see solidcore/docs/layers.md";

/// Check relative components without resolving dependencies. A path may reach the
/// vendor root, but it may never step above it (even to re-enter it later).
fn stays_inside_solidcore(depth: usize, path: &str) -> bool {
    // Reject both Unix and Windows roots/prefixes, regardless of the host OS.
    let bytes = path.as_bytes();
    if path.starts_with(['/', '\\']) || matches!(bytes, [drive, b':', ..] if drive.is_ascii_alphabetic()) {
        return false;
    }
    fn contained<'a>(mut depth: usize, parts: impl Iterator<Item = &'a str>) -> bool {
        for part in parts {
            match part {
                "" | "." => {}
                ".." if depth == 0 => return false,
                ".." => depth -= 1,
                _ => depth += 1,
            }
        }
        true
    }
    // A backslash separates components on Windows but can be literal on Unix.
    contained(depth, path.split(['/', '\\'])) && contained(depth, path.split('/'))
}

/// Dependency entries in package, workspace and target tables, plus Cargo overrides.
fn manifest_dep_entries<'a>(doc: &'a toml::Table) -> Vec<(String, &'a str, &'a toml::Value)> {
    const KINDS: [&str; 5] = ["dependencies", "dev-dependencies", "build-dependencies", "dev_dependencies", "build_dependencies"];
    let mut out = Vec::new();
    let mut add = |section: String, table: Option<&'a toml::Value>| {
        for (key, spec) in table.and_then(toml::Value::as_table).into_iter().flatten() {
            out.push((section.clone(), key.as_str(), spec));
        }
    };
    for kind in KINDS {
        add(kind.into(), doc.get(kind));
        add(format!("workspace.{kind}"), doc.get("workspace").and_then(|w| w.get(kind)));
    }
    for (target, table) in doc.get("target").and_then(toml::Value::as_table).into_iter().flatten() {
        for kind in KINDS {
            add(format!("target.{target}.{kind}"), table.get(kind));
        }
    }
    for (source, table) in doc.get("patch").and_then(toml::Value::as_table).into_iter().flatten() {
        add(format!("patch.{source}"), Some(table));
    }
    add("replace".into(), doc.get("replace"));
    out
}

fn application_package(name: &str) -> bool {
    name.starts_with("solidcraft-") || matches!(name, "solidcraft" | "intcad" | "intcad-cli")
}

fn inspect_manifest(file: &Path, label: &str, depth: usize, errors: &mut Vec<String>) -> Result<(), String> {
    let inspection_error = |e| format!("{label}: {e}; {STANDALONE_RULE}");
    if !fs::symlink_metadata(file).map_err(inspection_error)?.is_file() {
        return Err(format!("{label}: expected a regular manifest file; {STANDALONE_RULE}"));
    }
    let text = fs::read_to_string(file).map_err(inspection_error)?;
    let doc = match text.parse::<toml::Table>() {
        Ok(doc) => doc,
        Err(e) => {
            errors.push(format!("{label}: cannot parse the manifest ({e}); {STANDALONE_RULE}"));
            return Ok(());
        }
    };
    for (section, key, spec) in manifest_dep_entries(&doc) {
        // A replace key is a package ID, optionally qualified as URL#name:version.
        let name = if section == "replace" { key.rsplit('#').next().unwrap_or(key).split([':', '@']).next().unwrap_or(key) } else { key };
        let package = spec.get("package").and_then(toml::Value::as_str).unwrap_or(name);
        if application_package(name) || application_package(package) {
            errors.push(format!("{label}: [{section}] dependency `{key}` refers to application package `{package}`; {STANDALONE_RULE}"));
        }
        if let Some(path) = spec.get("path").and_then(toml::Value::as_str)
            && !stays_inside_solidcore(depth, path)
        {
            errors.push(format!("{label}: [{section}] dependency `{key}` has rooted or escaping path `{path}`; {STANDALONE_RULE}"));
        }
    }
    Ok(())
}

/// Inspect the vendored manifests independently of root Cargo metadata.
pub fn check_solidcore_manifests(root: &Path) -> Result<Vec<String>, String> {
    fn walk(dir: &Path, label: &str, depth: usize, errors: &mut Vec<String>) -> Result<(), String> {
        let inspection_error = |e| format!("{label}: {e}; {STANDALONE_RULE}");
        let mut entries = fs::read_dir(dir).map_err(inspection_error)?.collect::<Result<Vec<_>, _>>().map_err(inspection_error)?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name();
            let child = format!("{label}/{}", name.to_string_lossy());
            if name == "Cargo.toml" {
                if depth != 0 {
                    inspect_manifest(&entry.path(), &child, depth, errors)?;
                }
            } else if !matches!(name.to_str(), Some("target" | "upstream" | ".git" | "node_modules"))
                && entry.file_type().map_err(|e| format!("{child}: {e}; {STANDALONE_RULE}"))?.is_dir()
            {
                walk(&entry.path(), &child, depth + 1, errors)?;
            }
        }
        Ok(())
    }

    let dir = root.join("solidcore");
    if !fs::symlink_metadata(&dir).map_err(|e| format!("solidcore/: {e}; {STANDALONE_RULE}"))?.is_dir() {
        return Err(format!("solidcore/: expected the vendored directory; {STANDALONE_RULE}"));
    }
    let mut errors = Vec::new();
    inspect_manifest(&dir.join("Cargo.toml"), "solidcore/Cargo.toml", 0, &mut errors)?;
    walk(&dir, "solidcore", 0, &mut errors)?;
    errors.sort();
    errors.dedup();
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }
    use DepKind::*;

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("solidcraft-geom", &[("kurbo", Normal, false)]),
            c("solidcraft-doc", &[("solidcraft-geom", Normal, true)]),
            c("solidcraft-engine", &[("solidcraft-doc", Normal, true), ("solidcraft-testkit", Dev, true)]),
            c("solidcraft-ui-egui", &[("solidcraft-engine", Normal, true), ("egui", Normal, false)]),
            c("solidcraft-cli", &[("solidcraft-ui-egui", Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_dependency_flagged() {
        let v = check(&[c("solidcraft-doc", &[("solidcraft-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 2, to: 4, .. }]));
    }

    #[test]
    fn sideways_dependency_flagged() {
        let v = check(&[c("solidcraft-kernel", &[("solidcraft-sketch", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 1, .. }]));
    }

    #[test]
    fn self_dev_dependency_ignored() {
        assert!(check(&[c("solidcraft-doc", &[("solidcraft-doc", Dev, true)])]).is_empty());
    }

    #[test]
    fn upward_dev_dependency_flagged() {
        let v = check(&[c("solidcraft-geom", &[("solidcraft-doc", Dev, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Dev, .. }]));
    }

    #[test]
    fn ui_crates_below_l6_flagged() {
        for dep in ["egui", "eframe", "winit", "egui_kittest", "rfd", "bevy_ecs", "bevy"] {
            let v = check(&[c("solidcraft-engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowL5 { layer: 4, .. }]), "{dep}");
        }
        assert!(check(&[c("solidcraft-engine", &[("egui_extras_not", Normal, false)])]).is_empty());
        assert!(check(&[c("solidcraft-ui-egui", &[("winit", Normal, false)])]).is_empty());
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&[c("solidcraft-mystery", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "solidcraft-mystery"));
        assert!(v[0].to_string().contains("register"));
    }

    #[test]
    fn testkit_only_as_dev_dependency() {
        let v = check(&[c("solidcraft-doc", &[("solidcraft-testkit", Normal, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]));
        assert!(check(&[c("solidcraft-doc", &[("solidcraft-testkit", Dev, true)])]).is_empty());
        assert!(check(&[c("solidcraft-testkit", &[("solidcraft-engine", Normal, true)])]).is_empty());
    }

    #[test]
    fn apps_and_xtask_exempt() {
        for app in ["solidcraft", "solidcraft-cli", "xtask"] {
            assert!(check(&[c(app, &[("egui", Normal, false), ("solidcraft-ui-egui", Normal, true)])]).is_empty());
        }
    }

    #[test]
    fn metadata_parsing() {
        let meta: Value = serde_json::from_str(
            r#"{"packages":[
                {"name":"solidcraft-doc","dependencies":[
                    {"name":"solidcraft-geom","kind":null,"path":"/x/crates/geom"},
                    {"name":"serde","kind":null},
                    {"name":"proptest","kind":"dev"}]},
                {"name":"solidcraft-geom","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        let doc = g.iter().find(|c| c.name == "solidcraft-doc").unwrap();
        assert!(doc.deps[0].workspace && !doc.deps[1].workspace);
        assert_eq!(doc.deps[2].kind, Dev);
        assert!(check(&g).is_empty());
    }

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let dir = std::env::temp_dir().join(format!("xtask-layers-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                match std::fs::create_dir(&dir) {
                    Ok(()) => return Self(dir),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("{}: {e}", dir.display()),
                }
            }
        }

        fn write(&self, file: &str, bytes: impl AsRef<[u8]>) {
            let path = self.0.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn standalone_accepts_internal_paths_and_workspace_inheritance() {
        let tree = Fixture::new();
        tree.write(
            "solidcore/Cargo.toml",
            "[workspace]\nmembers = ['crates/local']\n[workspace.dependencies]\nlocal = { path = './crates/local' }\nserde = '1'",
        );
        tree.write(
            "solidcore/crates/local/Cargo.toml",
            "[package]\nname = 'local'\n[dependencies]\nserde.workspace = true\n[dev-dependencies]\nlocal.path = '../local'",
        );
        tree.write("solidcore/target/Cargo.toml", "not a manifest");
        let violations = check_solidcore_manifests(&tree.0).unwrap();
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn standalone_rejects_rooted_and_escaping_paths() {
        let tree = Fixture::new();
        tree.write("solidcore/Cargo.toml", "[workspace]");
        for path in [
            "../../../crates/geom",
            "../../../solidcore/crates/local",
            r"..\..\..\crates\geom",
            r"a\b/../../../../outside",
            "/crates/geom",
            r"\crates\geom",
            r"\\server\share\geom",
            r"C:\crates\geom",
            "C:crates/geom",
        ] {
            tree.write("solidcore/crates/local/Cargo.toml", format!("[dependencies]\nleak.path = '{path}'"));
            let errors = check_solidcore_manifests(&tree.0).unwrap();
            assert!(matches!(&errors[..], [e] if e.contains("solidcore/crates/local/Cargo.toml") && e.contains(path)), "{path}: {errors:?}");
        }
    }

    #[test]
    fn standalone_rejects_application_packages_in_all_dependency_tables() {
        let tree = Fixture::new();
        tree.write("solidcore/Cargo.toml", "[workspace]");
        for (table, entry) in [
            ("dependencies", "solidcraft-geom = '1'"),
            ("dependencies", "alias = { package = 'solidcraft', version = '1' }"),
            ("dependencies", "alias = { package = 'solidcraft-geom', version = '1' }"),
            ("workspace.dependencies", "alias.package = 'solidcraft-geom'"),
            ("target.'cfg(unix)'.dependencies", "alias.package = 'solidcraft-geom'"),
            ("target.'cfg(unix)'.dev-dependencies", "alias.package = 'solidcraft-geom'"),
            ("target.'cfg(unix)'.build-dependencies", "alias.package = 'solidcraft-geom'"),
            ("dev-dependencies", "alias.package = 'solidcraft-geom'"),
            ("build-dependencies", "alias.package = 'solidcraft-geom'"),
            ("dev_dependencies", "alias.package = 'solidcraft-geom'"),
            ("build_dependencies", "alias.package = 'solidcraft-geom'"),
            ("patch.crates-io", "alias.package = 'solidcraft-geom'"),
            ("replace", "'solidcraft-geom:1.0.0' = { path = 'crates/local' }"),
            ("replace", "'solidcraft:1.0.0' = { path = 'crates/local' }"),
        ] {
            tree.write("solidcore/Cargo.toml", format!("[{table}]\n{entry}"));
            let errors = check_solidcore_manifests(&tree.0).unwrap();
            assert!(matches!(&errors[..], [e] if e.contains("solidcraft")), "{table}: {entry}: {errors:?}");
        }
    }

    #[test]
    fn standalone_reports_malformed_toml() {
        let tree = Fixture::new();
        tree.write("solidcore/Cargo.toml", "[workspace");
        let errors = check_solidcore_manifests(&tree.0).unwrap();
        assert!(matches!(&errors[..], [e] if e.contains("Cargo.toml") && e.contains("parse")), "{errors:?}");
    }

    #[test]
    fn standalone_fails_when_manifest_inspection_fails() {
        let tree = Fixture::new();
        assert!(check_solidcore_manifests(&tree.0).is_err(), "missing solidcore tree");
        tree.write("solidcore/crates/local/Cargo.toml", "[dependencies]");
        assert!(check_solidcore_manifests(&tree.0).is_err(), "missing solidcore/Cargo.toml");
        tree.write("solidcore/Cargo.toml", "[workspace]");
        // Invalid UTF-8 represents a read failure without relying on permissions
        // or whether the test process is privileged.
        tree.write("solidcore/crates/local/Cargo.toml", [0xFF]);
        let error = check_solidcore_manifests(&tree.0).unwrap_err();
        assert!(error.contains("solidcore/crates/local/Cargo.toml"), "{error}");
    }
}
