//! `cargo xtask control-origins`: a control request's entry category is chosen by the trusted
//! host path that received it, never by an anonymous constructor or a request parameter.
//!
//! Every production `ControlRequest` constructor call must be one of the named host constructors
//! ([`TRUSTED_CONSTRUCTORS`]). `ControlRequest::new` is gone, and the request struct can only be
//! built inside `crates/ui-egui/src/control.rs`. `unclassified_for_test` is test-only. The check
//! parses Rust with syn and leaves out `#[cfg(test)]` modules and `#[test]` functions, exactly as
//! the build does, so test doubles may still build requests without becoming a production seam.

use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};

pub const CONTROL_REQUEST_FILE: &str = "crates/ui-egui/src/control.rs";

/// The only production constructors: each one is named after the host path it received.
pub const TRUSTED_CONSTRUCTORS: &[&str] = &[
    "trusted_acp",
    "trusted_bridge",
    "trusted_bridge_forwarded",
    "trusted_bridge_helper",
    "trusted_cli_dfm",
    "trusted_control",
    "trusted_external_ui",
    "trusted_gui",
    "trusted_mastra",
    "trusted_mcp",
    "trusted_review_panel",
    "trusted_web",
];

pub fn run(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    for dir in ["apps", "crates"] {
        collect_sources(&root.join(dir), &mut files)?;
    }
    let mut violations = Vec::new();
    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        if is_test_file(&rel) {
            continue;
        }
        let source = std::fs::read_to_string(&file).map_err(|e| format!("{rel}: {e}"))?;
        let ast = syn::parse_file(&source).map_err(|e| format!("{rel}: cannot parse Rust source: {e}"))?;
        let mut guard = OriginGuard { file: rel.clone(), in_control_file: rel == CONTROL_REQUEST_FILE, violations: Vec::new() };
        guard.visit_file(&ast);
        violations.extend(guard.violations);
    }
    if violations.is_empty() {
        println!("control-origins: every production control request uses a trusted host constructor");
        return Ok(());
    }
    for violation in &violations {
        eprintln!("{violation}");
    }
    Err(format!("control-origins: {} production control request(s) are not trusted", violations.len()))
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            collect_sources(&path, out)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Out-of-line test modules are named `tests.rs` or `*_tests.rs`; test support files are named
/// `test_support.rs`. Inline test items are skipped by [`OriginGuard`] through their attributes.
fn is_test_file(rel: &str) -> bool {
    let stem = Path::new(rel).file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
    stem == "tests" || stem.ends_with("_tests") || stem == "test_support" || stem.starts_with("test_")
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        if attribute.path().is_ident("test") {
            return true;
        }
        if !attribute.path().is_ident("cfg") {
            return false;
        }
        let Ok(meta) = attribute.parse_args::<syn::Meta>() else { return false };
        match meta {
            syn::Meta::Path(path) => path.is_ident("test"),
            syn::Meta::List(list) if list.path.is_ident("all") => list
                .parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
                .is_ok_and(|items| items.iter().any(|meta| meta.path().is_ident("test"))),
            _ => false,
        }
    })
}

fn constructor(path: &syn::Path) -> Option<(String, usize)> {
    let names: Vec<&syn::PathSegment> = path.segments.iter().collect();
    if names.len() < 2 {
        return None;
    }
    let owner = names.get(names.len() - 2)?;
    if owner.ident != "ControlRequest" {
        return None;
    }
    let call = names.last()?;
    Some((call.ident.to_string(), call.ident.span().start().line))
}

fn struct_literal(path: &syn::Path) -> Option<usize> {
    let last = path.segments.last()?;
    if last.ident == "ControlRequest" { Some(last.ident.span().start().line) } else { None }
}

struct OriginGuard {
    file: String,
    in_control_file: bool,
    violations: Vec<String>,
}

impl OriginGuard {
    fn check_constructor(&mut self, path: &syn::Path) {
        let Some((name, line)) = constructor(path) else { return };
        if TRUSTED_CONSTRUCTORS.contains(&name.as_str()) {
            return;
        }
        self.violations.push(if name == "unclassified_for_test" {
            format!("{}:{line} ControlRequest::{name} is test-only and cannot be a production origin", self.file)
        } else {
            format!("{}:{line} ControlRequest::{name} is not a trusted host constructor", self.file)
        });
    }

    fn check_macro_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            if let proc_macro2::TokenTree::Group(group) = token {
                self.check_macro_tokens(group.stream());
            }
            let proc_macro2::TokenTree::Ident(ident) = token else { continue };
            let mut names = vec![ident.to_string()];
            let mut rest = tokens.get(index + 1..).unwrap_or_default();
            while let [proc_macro2::TokenTree::Punct(a), proc_macro2::TokenTree::Punct(b), proc_macro2::TokenTree::Ident(name), tail @ ..] = rest {
                if a.as_char() != ':' || b.as_char() != ':' {
                    break;
                }
                names.push(name.to_string());
                rest = tail;
            }
            if names.len() >= 2 && names.get(names.len() - 2).is_some_and(|owner| owner == "ControlRequest") {
                let name = names.last().map(String::as_str).unwrap_or_default().to_owned();
                let line = ident.span().start().line;
                if !TRUSTED_CONSTRUCTORS.contains(&name.as_str()) {
                    self.violations.push(format!("{}:{line} ControlRequest::{name} in a macro is not a trusted host constructor", self.file));
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for OriginGuard {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if test_only(&item.attrs) {
            return;
        }
        visit::visit_item_mod(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if test_only(&item.attrs) {
            return;
        }
        visit::visit_item_fn(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if test_only(&item.attrs) {
            return;
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            self.check_constructor(&path.path);
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_struct(&mut self, literal: &'ast syn::ExprStruct) {
        if let Some(line) = struct_literal(&literal.path)
            && !self.in_control_file
        {
            self.violations.push(format!("{}:{line} ControlRequest struct literals are only allowed in {CONTROL_REQUEST_FILE}", self.file));
        }
        visit::visit_expr_struct(self, literal);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.check_macro_tokens(mac.tokens.clone());
        visit::visit_macro(self, mac);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(source: &str, in_control_file: bool) -> Vec<String> {
        let ast = syn::parse_file(source).unwrap();
        let mut guard = OriginGuard { file: "test.rs".into(), in_control_file, violations: Vec::new() };
        guard.visit_file(&ast);
        guard.violations
    }

    #[test]
    fn new_and_unknown_constructors_are_refused() {
        let violations = check(
            r#"
            fn production() {
                let _ = ControlRequest::new("engine.execute", json!({}));
                let _ = ControlRequest::unclassified_for_test("engine.execute", json!({}));
            }
            "#,
            false,
        );
        assert_eq!(violations.len(), 2, "{violations:?}");
        assert!(violations[0].contains("ControlRequest::new"));
        assert!(violations[1].contains("unclassified_for_test"));
    }

    #[test]
    fn named_trusted_constructors_are_accepted_in_production() {
        let violations = check(
            r#"
            fn production() {
                let _ = ControlRequest::trusted_gui("engine.execute", json!({}));
                let _ = ControlRequest::trusted_control("engine.execute", json!({}));
                let _ = ControlRequest::trusted_bridge_forwarded("engine.execute", json!({}), trusted, None, None);
                let _ = ControlRequest::trusted_bridge_helper("engine.execute", json!({}), "measure");
            }
            "#,
            false,
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn test_modules_and_functions_are_left_out() {
        let violations = check(
            r#"
            #[cfg(test)]
            mod tests {
                fn helper() { let _ = ControlRequest::new("engine.execute", json!({})); }
            }
            #[test]
            fn one() { let _ = ControlRequest::new("engine.execute", json!({})); }
            "#,
            false,
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn struct_literals_are_only_allowed_in_the_control_file() {
        let source = "fn build() -> ControlRequest { ControlRequest { method: String::new() } }";
        assert_eq!(check(source, false).len(), 1);
        assert!(check(source, true).is_empty());
    }

    #[test]
    fn macros_are_inspected_too() {
        let violations = check(
            r#"
            fn production() { forward!(ControlRequest::new("engine.execute", json!({}))); }
            "#,
            false,
        );
        assert_eq!(violations.len(), 1, "{violations:?}");
    }
}
