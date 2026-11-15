//! Build information for Help › About: the git commit and the build date (UTC). Both fall back to
//! "unknown" when git is missing (a source tarball). Also one test per UI scenario file
//! (`tests/scenarios/*.json`), so adding a scenario needs no Rust edit.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// Days since 1970-01-01 to a civil date (proleptic Gregorian).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn main() {
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some();
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    println!("cargo:rustc-env=SOLIDCRAFT_COMMIT={commit}{}", if dirty { "+" } else { "" });
    println!("cargo:rustc-env=SOLIDCRAFT_BUILD_DATE={y:04}-{m:02}-{d:02}");
    // A new commit in this checkout (or worktree) restamps the build.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/logs/HEAD");
    }
    println!("cargo:rerun-if-changed=build.rs");
    scenario_tests();
    if let Err(e) = merged_locales() {
        eprintln!("locales: {e}");
        std::process::exit(1);
    }
    if let Err(e) = mastra_expectations() {
        eprintln!("mastra: {e}");
        std::process::exit(1);
    }
}

/// The reviewed identities a packaged Mastra runtime must match: one repository revision,
/// contract bundle, runtime and Node version, policy and source-record digests, and the
/// inventory digests the reviewed policy currently approves. An empty approval list makes the
/// installed-runtime gate fail closed, which is the state until #15 records a legal review.
fn mastra_expectations() -> Result<(), String> {
    use sha2::{Digest as _, Sha256};

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?;
    let root = std::path::Path::new(&crate_dir).join("../..");
    println!("cargo:rerun-if-env-changed=SOLIDCRAFT_GIT_REVISION_OVERRIDE");
    println!("cargo:rerun-if-env-changed=SOLIDCRAFT_WITH_MASTRA");
    println!("cargo:rerun-if-env-changed=SOLIDCRAFT_RELEASE");
    const UNBOUND_REVISION: &str = "0000000000000000000000000000000000000000";
    let digest = |relative: &str| -> Result<String, String> {
        let path = root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!("sha256:{:x}", Sha256::digest(&bytes)))
    };
    let read_json = |relative: &str| -> Result<serde_json::Value, String> {
        let path = root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    };

    let strict = std::env::var("SOLIDCRAFT_WITH_MASTRA").as_deref() == Ok("1") || std::env::var("SOLIDCRAFT_RELEASE").as_deref() == Ok("1");
    let override_revision =
        std::env::var("SOLIDCRAFT_GIT_REVISION_OVERRIDE").ok().filter(|value| value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()));
    let git_revision = git(&["rev-parse", "HEAD"]).filter(|r| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()));
    let revision = match override_revision.or(git_revision) {
        Some(revision) => revision,
        None if strict => {
            return Err("cannot determine the full 40-character Git revision needed to bind the packaged Mastra runtime; build from a Git checkout or set SOLIDCRAFT_GIT_REVISION_OVERRIDE".to_string());
        }
        // An ordinary source/Nix build has no Git metadata. The sentinel compiles, but the host
        // refuses to match any installed runtime against it, so the runtime still fails closed.
        None => UNBOUND_REVISION.to_string(),
    };
    let contract = digest("schema/mastra-runtime/v1/manifest.json")?;
    let policy = digest("packaging/mastra/policy.json")?;
    let sources = digest("packaging/mastra/source-records.json")?;
    println!("cargo:rerun-if-changed=../../.nvmrc");
    let node = std::fs::read_to_string(root.join(".nvmrc")).map_err(|e| format!(".nvmrc: {e}"))?.trim().to_string();
    let package = read_json("packages/mastra-runtime/package.json")?;
    let runtime = package.get("version").and_then(serde_json::Value::as_str).unwrap_or("unknown").to_string();
    let approvals = read_json("packaging/mastra/policy.json")?
        .get("approvals")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().filter_map(|item| item.get("inventoryDigest").and_then(serde_json::Value::as_str)).collect::<Vec<_>>().join(","))
        .unwrap_or_default();

    println!("cargo:rustc-env=SOLIDCRAFT_GIT_REVISION={revision}");
    println!("cargo:rustc-env=SOLIDCRAFT_MASTRA_CONTRACT_DIGEST={contract}");
    println!("cargo:rustc-env=SOLIDCRAFT_MASTRA_POLICY_DIGEST={policy}");
    println!("cargo:rustc-env=SOLIDCRAFT_MASTRA_SOURCES_DIGEST={sources}");
    println!("cargo:rustc-env=SOLIDCRAFT_MASTRA_RUNTIME_VERSION={runtime}");
    println!("cargo:rustc-env=SOLIDCRAFT_MASTRA_NODE_VERSION={node}");
    println!("cargo:rustc-env=SOLIDCRAFT_MASTRA_APPROVED_INVENTORIES={approvals}");
    Ok(())
}

/// `$OUT_DIR/locales/<id>.json`: each catalog in `assets/locales` plus the messages of
/// `assets/locales/pending/*.json` (`{"<locale>": {"<id>": "<text>"}}`, one file per change, folded
/// into the catalogs on main), so a pull request's new strings work before they are folded. An id
/// a catalog already has, or a locale without a catalog, fails the build, as `cargo xtask lists`
/// does.
fn merged_locales() -> Result<(), String> {
    use serde_json::{Map, Value};
    let dir = std::path::Path::new("../../assets/locales");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut catalogs: std::collections::BTreeMap<String, Map<String, Value>> = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))? {
            Value::Object(map) => catalogs.insert(id.to_string(), map),
            _ => return Err(format!("{}: a catalog must be a JSON object", path.display())),
        };
    }
    let pending = dir.join("pending");
    println!("cargo:rerun-if-changed={}", pending.display());
    let mut files: Vec<_> = std::fs::read_dir(&pending)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
        .unwrap_or_default();
    files.sort();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let map: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> =
            serde_json::from_str(&text).map_err(|e| format!("{}: expected {{\"<locale>\": {{\"<id>\": \"<text>\"}}}}: {e}", file.display()))?;
        for (locale, messages) in map {
            let catalog =
                catalogs.get_mut(&locale).ok_or_else(|| format!("{}: `{locale}` has no catalog assets/locales/{locale}.json", file.display()))?;
            for (id, text) in messages {
                if catalog.insert(id.clone(), Value::String(text)).is_some() {
                    return Err(format!("{}: `{id}` is already in {locale}.json (or another pending file)", file.display()));
                }
            }
        }
    }
    let out = std::path::Path::new(&std::env::var("OUT_DIR").map_err(|e| e.to_string())?).join("locales");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (id, catalog) in catalogs {
        let text = serde_json::to_string(&catalog).map_err(|e| e.to_string())?;
        std::fs::write(out.join(format!("{id}.json")), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `$OUT_DIR/scenario_tests.rs`: a `#[test]` per scenario file, named after it; a scenario with
/// a `{"pending": "why"}` step is `#[ignore]`d with that reason (it waits for a fix).
fn scenario_tests() {
    let dir = std::path::Path::new("tests/scenarios");
    println!("cargo:rerun-if-changed=tests/scenarios");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
        .unwrap_or_default();
    files.sort();
    let mut out = String::new();
    for f in files {
        let Some(stem) = f.file_stem().and_then(|s| s.to_str()) else { continue };
        let mut name: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
        if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            name.insert_str(0, "s_");
        }
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        let pending = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.as_array().and_then(|a| a.iter().find_map(|s| s.get("pending").map(|p| p.as_str().unwrap_or("pending").to_string()))));
        if let Some(why) = pending {
            out += &format!("#[test]\n#[ignore = {why:?}]\nfn {name}() {{\n    run({stem:?});\n}}\n\n");
        } else {
            out += &format!("#[test]\nfn {name}() {{\n    run({stem:?});\n}}\n\n");
        }
    }
    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into())).join("scenario_tests.rs");
    let _ = std::fs::write(path, out);
}
