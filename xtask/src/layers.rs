//! Dependency layering rules (plan/architecture.md §3).
//!
//! Adapted from CADCraft's xtask (MIT OR Apache-2.0), see ATTRIBUTION.md.
//!
//! The rule engine works on a small, metadata-independent model so it can be
//! unit-tested; `from_metadata` builds that model from `cargo metadata`.

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
    // 3D mouse descriptors and reports (no workspace dependencies; the apps read the device)
    ("space-mouse", Class::Layer(0)),
    // the derive macros (proc-macro, no workspace dependencies)
    ("derive", Class::Standalone),
    // the 3Dconnexion driver's client API on macOS (no workspace dependencies)
    ("3dx-client", Class::Standalone),
    // suspend-inclusive native account clock (no workspace dependencies)
    ("suspend-clock", Class::Standalone),
    // the profiler: timed spans for every layer, the kernel's calls to commands (no workspace dependencies)
    ("profile", Class::Standalone),
    ("sketch", Class::Layer(1)),
    // the B-rep kernel, solidcore (the Rust port of NVIDIA's SMLib, ADR 0002); only the kernel uses it
    ("smlib", Class::Layer(1)),
    // STEP (ISO 10303) exchange structure and geometry, no modelling backend
    ("step", Class::Layer(1)),
    ("kernel", Class::Layer(1)),
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
    ("mcp", Class::Layer(5)),
    ("testkit", Class::Testkit),
    // native account adapters (durable ledger, credential store, HTTP); native-only, never in the
    // wasm closure, and only apps compose it
    ("account-native", Class::Exempt),
    // apps and tooling
    ("solidcraft", Class::Exempt),
    ("cli", Class::Exempt),
    ("web", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Explicit orderings *within* a layer (earlier may be used by later).
/// The viewport draws display plans the GPU crate evaluates and draws with its pipelines; the MCP
/// server's tests draw screenshots with them. The kernel reads and writes STEP through `step`.
pub const INTRA_LAYER_ORDER: &[&[&str]] = &[
    &["gpu", "ui-egui"],
    &["gpu", "mcp"],
    // The UI hosts the chat MCP server over the Mastra bridge lane (#9); the server never
    // depends on the UI.
    &["mcp", "ui-egui"],
    // The task controller's real engine ports sit beside the engine, below the UI.
    &["engine", "agent-task-engine"],
    &["smlib", "kernel"],
    &["step", "kernel"],
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
    ForbiddenClosure { root: String, dep: String, target: ClosureTarget },
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
            Violation::ForbiddenClosure { root, dep, target } => {
                write!(f, "{root} reaches `{dep}` in the {} dependency closure; it is native-only", target.label())
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

/// A dependency edge in the resolved package graph, including cargo's target condition.
#[derive(Debug, Clone)]
pub struct ClosureDep {
    pub name: String,
    pub kind: DepKind,
    pub target: Option<String>,
}

/// A package in the resolved graph. Names can repeat across versions, so the closure follows
/// every package with a matching name (conservative, never narrower than cargo's closure).
#[derive(Debug, Clone)]
pub struct ClosureCrate {
    pub name: String,
    pub deps: Vec<ClosureDep>,
}

/// Which conditional edges a closure guard follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureTarget {
    /// Every target: a dependency is forbidden even if it is reached only on another platform.
    Any,
    /// The browser target: target-specific edges are evaluated for `wasm32-unknown-unknown`.
    Wasm,
}

impl ClosureTarget {
    fn label(self) -> &'static str {
        match self {
            ClosureTarget::Any => "native and wasm",
            ClosureTarget::Wasm => "wasm32-unknown-unknown",
        }
    }
}

/// The browser roots that must never compose native account adapters.
pub const BROWSER_ROOTS: &[&str] = &["solidcraft-web", "solidcraft-ui-egui"];
/// Native-only crates forbidden below the browser roots on every target.
pub const NATIVE_ONLY_CRATES: &[&str] = &["solidcraft-account-native", "solidcraft-suspend-clock"];
/// Native-only crates forbidden in the browser target's closure.
pub const WASM_FORBIDDEN_CRATES: &[&str] = &["solidcraft-account-native", "solidcraft-suspend-clock", "keyring", "ureq"];

/// Build the resolved graph from `cargo metadata --format-version 1 --locked` (without
/// `--no-deps`). Build and normal edges are followed, dev edges are not.
pub fn from_resolve(meta: &Value) -> Result<Vec<ClosureCrate>, String> {
    let nodes = meta["resolve"]["nodes"].as_array().ok_or("metadata: no resolve.nodes array")?;
    let packages = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let mut names = std::collections::HashMap::new();
    for p in packages {
        if let (Some(id), Some(name)) = (p["id"].as_str(), p["name"].as_str()) {
            names.insert(id, name);
        }
    }
    let mut out = Vec::new();
    for node in nodes {
        let id = node["id"].as_str().ok_or("resolve node without id")?;
        let name = names.get(id).ok_or_else(|| format!("resolve node {id} has no package"))?;
        let mut deps = Vec::new();
        for dep in node["deps"].as_array().into_iter().flatten() {
            let pkg = dep["pkg"].as_str().ok_or("resolve dep without pkg")?;
            let dname = names.get(pkg).ok_or_else(|| format!("resolve dep {pkg} has no package"))?;
            for kind in dep["dep_kinds"].as_array().into_iter().flatten() {
                let dep_kind = match kind["kind"].as_str() {
                    Some("dev") => DepKind::Dev,
                    Some("build") => DepKind::Build,
                    _ => DepKind::Normal,
                };
                deps.push(ClosureDep { name: (*dname).to_owned(), kind: dep_kind, target: kind["target"].as_str().map(str::to_owned) });
            }
        }
        out.push(ClosureCrate { name: (*name).to_owned(), deps });
    }
    Ok(out)
}

fn edge_selected(dep: &ClosureDep, target: ClosureTarget) -> bool {
    if dep.kind == DepKind::Dev {
        return false;
    }
    match target {
        ClosureTarget::Any => true,
        // Unknown cfg expressions stay in the closure: a guard may be wider than cargo, never
        // narrower.
        ClosureTarget::Wasm => dep.target.as_deref().is_none_or(|expr| cfg_matches(expr).unwrap_or(true)),
    }
}

/// Whether `root` reaches `forbidden` through normal or build dependencies selected for `target`.
pub fn reaches(graph: &[ClosureCrate], root: &str, forbidden: &str, target: ClosureTarget) -> bool {
    let mut seen = vec![false; graph.len()];
    let mut stack: Vec<usize> = graph.iter().enumerate().filter(|(_, c)| c.name == root).map(|(i, _)| i).collect();
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        for dep in &graph[i].deps {
            if !edge_selected(dep, target) {
                continue;
            }
            if dep.name == forbidden {
                return true;
            }
            for (j, c) in graph.iter().enumerate() {
                if c.name == dep.name && !seen[j] {
                    stack.push(j);
                }
            }
        }
    }
    false
}

/// Guard rules for the browser and native closures. Returns violations sorted for stable output.
pub fn closure_check(graph: &[ClosureCrate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for root in BROWSER_ROOTS {
        for dep in NATIVE_ONLY_CRATES {
            if reaches(graph, root, dep, ClosureTarget::Any) {
                out.push(Violation::ForbiddenClosure { root: (*root).into(), dep: (*dep).into(), target: ClosureTarget::Any });
            }
        }
    }
    let mut wasm_roots: Vec<&str> = Vec::new();
    for c in graph {
        if matches!(classify(&c.name), Some(Class::Layer(_))) && !wasm_roots.contains(&c.name.as_str()) {
            wasm_roots.push(&c.name);
        }
    }
    if !wasm_roots.contains(&"solidcraft-web") {
        wasm_roots.push("solidcraft-web");
    }
    for root in wasm_roots {
        for dep in WASM_FORBIDDEN_CRATES {
            if reaches(graph, root, dep, ClosureTarget::Wasm) {
                out.push(Violation::ForbiddenClosure { root: root.into(), dep: (*dep).into(), target: ClosureTarget::Wasm });
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

fn cfg_matches(expr: &str) -> Option<bool> {
    let expr = expr.trim();
    if expr == "wasm32-unknown-unknown" {
        return Some(true);
    }
    let inner = expr.strip_prefix("cfg(")?.strip_suffix(')')?.trim();
    eval_cfg(inner)
}

fn eval_cfg(pred: &str) -> Option<bool> {
    let pred = pred.trim();
    if pred.is_empty() {
        return None;
    }
    if let Some(inner) = cfg_call(pred, "all") {
        let mut unknown = false;
        for part in split_cfg_args(inner) {
            match eval_cfg(part) {
                Some(false) => return Some(false),
                None => unknown = true,
                Some(true) => {}
            }
        }
        return if unknown { None } else { Some(true) };
    }
    if let Some(inner) = cfg_call(pred, "any") {
        let mut unknown = false;
        for part in split_cfg_args(inner) {
            match eval_cfg(part) {
                Some(true) => return Some(true),
                None => unknown = true,
                Some(false) => {}
            }
        }
        return if unknown { None } else { Some(false) };
    }
    if let Some(inner) = cfg_call(pred, "not") {
        return eval_cfg(inner).map(|b| !b);
    }
    if let Some((key, value)) = pred.split_once('=') {
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        return wasm_cfg_value(key).map(|v| v == value);
    }
    match pred {
        "unix" | "windows" => Some(false),
        _ => None,
    }
}

fn cfg_call<'a>(pred: &'a str, name: &str) -> Option<&'a str> {
    pred.strip_prefix(name)?.strip_prefix('(')?.strip_suffix(')')
}

fn split_cfg_args(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

fn wasm_cfg_value(key: &str) -> Option<&'static str> {
    match key {
        "target_arch" => Some("wasm32"),
        "target_os" => Some("unknown"),
        "target_family" => Some("wasm"),
        "target_vendor" => Some("unknown"),
        "target_env" | "target_abi" => Some(""),
        "target_pointer_width" => Some("32"),
        "target_endian" => Some("little"),
        _ => None,
    }
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

    fn cc(name: &str, deps: &[(&str, DepKind, Option<&str>)]) -> ClosureCrate {
        ClosureCrate {
            name: name.into(),
            deps: deps.iter().map(|(n, k, t)| ClosureDep { name: (*n).into(), kind: *k, target: t.map(str::to_owned) }).collect(),
        }
    }

    fn closure_reaches(graph: &[ClosureCrate], root: &str, forbidden: &str, target: ClosureTarget) -> bool {
        reaches(graph, root, forbidden, target)
    }

    #[test]
    fn account_native_is_not_in_web_or_ui_wasm_closure() {
        // Inert synthetic cargo graph: the clean roots reach engine but no account adapter.
        let clean = [
            cc("solidcraft-web", &[("solidcraft-engine", Normal, None)]),
            cc("solidcraft-ui-egui", &[("solidcraft-engine", Normal, None)]),
            cc("solidcraft-engine", &[("solidcraft-geom", Normal, None)]),
            cc("solidcraft-geom", &[]),
        ];
        assert!(closure_check(&clean).is_empty(), "{:?}", closure_check(&clean));

        // A transitive forbidden dependency below a browser root must turn the guard red.
        let dirty = [
            cc("solidcraft-web", &[("solidcraft-web-shim", Normal, None)]),
            cc("solidcraft-web-shim", &[("solidcraft-account-native", Normal, None)]),
            cc("solidcraft-account-native", &[("keyring", Normal, None), ("ureq", Normal, None)]),
            cc("solidcraft-ui-egui", &[("solidcraft-engine", Normal, None)]),
            cc("solidcraft-engine", &[("solidcraft-geom", Normal, None)]),
            cc("solidcraft-geom", &[]),
        ];
        let v = closure_check(&dirty);
        assert!(
            v.iter().any(|v| matches!(v, Violation::ForbiddenClosure { root, dep, target: ClosureTarget::Any }
            if root == "solidcraft-web" && dep == "solidcraft-account-native")),
            "{v:?}"
        );
        assert!(
            v.iter().any(|v| matches!(v, Violation::ForbiddenClosure { root, dep, target: ClosureTarget::Wasm }
            if root == "solidcraft-web" && dep == "keyring")),
            "{v:?}"
        );
    }

    #[test]
    fn wasm_closure_ignores_edges_for_other_targets() {
        // `keyring` is reached only from a Windows-gated edge; `ureq` only from a wasm edge.
        let g = [
            cc(
                "solidcraft-web",
                &[("windows-shim", Normal, Some("cfg(target_os = \"windows\")")), ("wasm-shim", Normal, Some("cfg(target_arch = \"wasm32\")"))],
            ),
            cc("windows-shim", &[("keyring", Normal, None)]),
            cc("wasm-shim", &[("ureq", Normal, None)]),
            cc("keyring", &[]),
            cc("ureq", &[]),
        ];
        assert!(!closure_reaches(&g, "solidcraft-web", "keyring", ClosureTarget::Wasm));
        assert!(closure_reaches(&g, "solidcraft-web", "keyring", ClosureTarget::Any));
        assert!(closure_reaches(&g, "solidcraft-web", "ureq", ClosureTarget::Wasm));
        assert!(closure_reaches(&g, "solidcraft-web", "ureq", ClosureTarget::Any));

        // The wasm rule covers every layer crate, not only the browser roots.
        let layer = [
            cc("solidcraft-engine", &[("wasm-shim", Normal, Some("cfg(target_arch = \"wasm32\")"))]),
            cc("wasm-shim", &[("ureq", Normal, None)]),
            cc("ureq", &[]),
        ];
        let v = closure_check(&layer);
        assert!(
            v.iter().any(|v| matches!(v, Violation::ForbiddenClosure { root, dep, target: ClosureTarget::Wasm }
            if root == "solidcraft-engine" && dep == "ureq")),
            "{v:?}"
        );
    }

    #[test]
    fn target_filter_is_conservative_for_unknown_and_nested_conditions() {
        // The guard must keep an edge selected when any conjunct is unknown: it may be wider than
        // cargo, never narrower.
        let unknown_conjunct = [
            cc("solidcraft-web", &[("unknown-shim", Normal, Some("cfg(all(target_arch = \"wasm32\", target_feature = \"simd128\"))"))]),
            cc("unknown-shim", &[("keyring", Normal, None)]),
            cc("keyring", &[]),
        ];
        let v = closure_check(&unknown_conjunct);
        assert!(
            v.iter().any(|v| matches!(v, Violation::ForbiddenClosure { root, dep, target: ClosureTarget::Wasm }
                if root == "solidcraft-web" && dep == "keyring")),
            "unknown conjunct was dropped instead of retained: {v:?}"
        );

        // A known false conjunct still excludes the edge.
        let known_false = [
            cc("solidcraft-web", &[("false-shim", Normal, Some("cfg(all(target_arch = \"wasm32\", target_os = \"windows\"))"))]),
            cc("false-shim", &[("keyring", Normal, None)]),
            cc("keyring", &[]),
        ];
        assert!(closure_check(&known_false).is_empty(), "{:?}", closure_check(&known_false));

        // Unknown inside a nested `all` also survives an outer `any` with a false alternative.
        let nested_unknown = [
            cc(
                "solidcraft-web",
                &[("nested-shim", Normal, Some("cfg(any(target_os = \"linux\", all(target_arch = \"wasm32\", target_feature = \"simd128\")))"))],
            ),
            cc("nested-shim", &[("keyring", Normal, None)]),
            cc("keyring", &[]),
        ];
        let v = closure_check(&nested_unknown);
        assert!(
            v.iter().any(|v| matches!(v, Violation::ForbiddenClosure { root, dep, target: ClosureTarget::Wasm }
                if root == "solidcraft-web" && dep == "keyring")),
            "nested unknown conjunct was dropped: {v:?}"
        );

        // Build edges are followed; dev edges are not.
        let build = [
            cc("solidcraft-web", &[("build-shim", Build, Some("cfg(target_arch = \"wasm32\")"))]),
            cc("build-shim", &[("keyring", Normal, None)]),
            cc("keyring", &[]),
        ];
        assert!(!closure_check(&build).is_empty(), "build edge was not followed");
        let dev = [
            cc("solidcraft-web", &[("dev-shim", Dev, Some("cfg(target_arch = \"wasm32\")"))]),
            cc("dev-shim", &[("keyring", Normal, None)]),
            cc("keyring", &[]),
        ];
        assert!(closure_check(&dev).is_empty(), "dev edge must not be followed");
    }

    #[test]
    fn resolved_graph_reads_target_conditions() {
        let meta: Value = serde_json::from_str(
            r#"{
                "packages":[
                    {"id":"pkg web","name":"solidcraft-web"},
                    {"id":"pkg shim","name":"web-shim"},
                    {"id":"pkg native","name":"solidcraft-account-native"}
                ],
                "resolve":{"nodes":[
                    {"id":"pkg web","deps":[
                        {"pkg":"pkg shim","dep_kinds":[
                            {"kind":null,"target":null},
                            {"kind":"dev","target":null}
                        ]}
                    ]},
                    {"id":"pkg shim","deps":[
                        {"pkg":"pkg native","dep_kinds":[{"kind":null,"target":"cfg(target_os = \"windows\")"}]}
                    ]},
                    {"id":"pkg native","deps":[]}
                ]}
            }"#,
        )
        .unwrap();
        let g = from_resolve(&meta).unwrap();
        let web = g.iter().find(|c| c.name == "solidcraft-web").unwrap();
        assert_eq!(web.deps.len(), 2);
        assert_eq!(web.deps[1].kind, Dev);
        assert!(!closure_reaches(&g, "solidcraft-web", "solidcraft-account-native", ClosureTarget::Wasm));
        assert!(closure_reaches(&g, "solidcraft-web", "solidcraft-account-native", ClosureTarget::Any));
    }
}
