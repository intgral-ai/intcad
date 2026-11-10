//! `cargo xtask i18n`: the message catalogs' coverage report, and a way to repair it.
//!
//! The catalogs under `assets/locales` are plain JSON maps from a stable message id
//! to one translation. The English catalog is also the lookup table for `Locale::label()`, which
//! translates source strings coming from the engine (command labels, toolbar tabs and panels) by
//! matching their English text. A label the English catalog does not carry therefore stays English
//! in *every* locale, silently — nothing fails, the row just looks untranslated.
//!
//! That is the failure this command exists to make visible. `--check` turns the report into a
//! non-zero exit for CI; `--add-missing` writes the uncovered command labels into the English
//! catalog under `cmd.<command id>` so a translator only has to fill in the other catalogs.
//!
//! German and Korean predate the coverage guards and are still partial, so their gaps are
//! reported but do not fail `--check`; the catalogs delivered complete (`COMPLETE`) may not
//! regress.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

/// Where the catalogs live, and the file the English one must lead.
const DIR: &str = "assets/locales";

/// Catalogs delivered complete: a missing message here fails `--check`. English is the source
/// catalog and is checked implicitly; keep this in step with `COMPLETE` in `ui-egui::i18n`.
const COMPLETE: &[&str] = &["ar", "he", "ja", "vi", "zh"];

/// What `run` found.
#[derive(Default)]
struct Report {
    /// Locale id → the English message ids it has not translated.
    missing: BTreeMap<String, Vec<String>>,
    /// Command label → the command ids that use it, for labels English does not carry.
    untranslated: BTreeMap<String, Vec<String>>,
}

impl Report {
    /// Gaps that must be repaired (complete catalogs only; partial catalogs are reported).
    fn problems(&self) -> usize {
        self.missing.iter().filter(|(id, _)| COMPLETE.contains(&id.as_str())).map(|(_, m)| m.len()).sum::<usize>() + self.untranslated.len()
    }

    /// Gaps in catalogs that are still allowed to be partial.
    fn reported_only(&self) -> usize {
        self.missing.iter().filter(|(id, _)| !COMPLETE.contains(&id.as_str())).map(|(_, m)| m.len()).sum()
    }
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let english = read(root, "en")?;
    let mut catalogs: Vec<(String, Map<String, Value>)> = Vec::new();
    for id in locales(root)? {
        let messages = read(root, &id)?;
        catalogs.push((id, messages));
    }

    let mut report = Report::default();
    for (id, messages) in &catalogs {
        if id == "en" {
            continue;
        }
        let missing: Vec<String> = english.keys().filter(|k| !messages.contains_key(*k)).cloned().collect();
        if !missing.is_empty() {
            report.missing.insert(id.clone(), missing);
        }
    }

    let known: Vec<&str> = english.values().filter_map(Value::as_str).collect();
    for (label, ids) in labels_needing_translation(&known) {
        report.untranslated.insert(label, ids);
    }

    if args.contains(&"--add-missing") {
        let added = add_missing(root, &mut report)?;
        println!("i18n: wrote {added} new English message(s) into {DIR}/en.json");
        return Ok(());
    }

    print_report(&report);

    if args.contains(&"--check") {
        if report.problems() > 0 {
            return Err(format!(
                "{} message(s) missing from a complete catalog and {} command label(s) with no English entry \
                 (see above); `cargo xtask i18n --add-missing` writes the labels into en.json",
                report.missing.iter().filter(|(id, _)| COMPLETE.contains(&id.as_str())).map(|(_, m)| m.len()).sum::<usize>(),
                report.untranslated.len()
            ));
        }
        if report.reported_only() > 0 {
            println!(
                "i18n: every complete catalog ({}) carries every English message and every command label is translatable; \
                 {} message(s) are still missing from the partial catalogs (de, ko)",
                COMPLETE.join(", "),
                report.reported_only()
            );
        } else {
            println!("i18n: every catalog carries every English message; every command label is translatable");
        }
    }
    Ok(())
}

/// Locale ids other than English: every catalog file that sits beside `en.json`, so a newly
/// added locale is picked up without editing this command. A regional English (`en-GB.json`)
/// holds only the messages it spells differently, so it has no gaps to report.
fn locales(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join(DIR);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|id| id != "en" && !id.starts_with("en-"))
        .collect();
    ids.sort();
    Ok(ids)
}

fn read(root: &Path, locale: &str) -> Result<Map<String, Value>, String> {
    let path = root.join(DIR).join(format!("{locale}.json"));
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Messages still in assets/locales/pending/ count: they are folded on main.
    let pending = crate::pending::locale_messages(root, locale)?;
    match serde_json::from_str(&text) {
        Ok(Value::Object(mut map)) => {
            for (id, text) in pending {
                map.entry(id).or_insert(Value::String(text));
            }
            Ok(map)
        }
        Ok(_) => Err(format!("{}: a catalog must be a JSON object", path.display())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Command labels the English catalog cannot translate, mapped to the commands that use them.
///
/// The registry is read through `solidcraft-cli commands` so the report comes from the real
/// command table, never from a copy of it.
fn labels_needing_translation(known: &[&str]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Ok(commands) = registry() else {
        return out;
    };
    for command in commands {
        let (Some(id), Some(label)) = (command["id"].as_str(), command["label"].as_str()) else {
            continue;
        };
        if known.contains(&label) {
            continue;
        }
        let ids = out.entry(label.to_string()).or_default();
        if !ids.iter().any(|x| x == id) {
            ids.push(id.to_string());
        }
    }
    out
}

/// Command labels from the generated `docs/features.md` table. It is written from the same
/// registry, so the check does not need a nested release CLI build (which used to stall the CI
/// repository job on a cold cache). A new command lands in this generated file with the normal
/// regeneration, before its label can be translated.
fn registry() -> Result<Vec<Value>, String> {
    let path = crate::root().join("docs/features.md");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut commands = Vec::new();
    for line in text.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let label = command_label(cells[1]);
        let id = cells[2].trim_matches('`');
        if label.is_empty() || label == "Command" || id.is_empty() || id == "Id" || id.chars().all(|c| c == '-') {
            continue;
        }
        commands.push(json!({"id": id, "label": label}));
    }
    if commands.is_empty() {
        return Err(format!("{}: no command rows", path.display()));
    }
    Ok(commands)
}

/// The generated table adds the toolbar shortcut in parentheses. Remove only that annotation;
/// a real label may itself contain parentheses (`Control Point Spline (Degree 5)`).
fn command_label(raw: &str) -> String {
    if let Some(start) = raw.find(" *(planned:") {
        return raw[..start].trim().to_string();
    }
    let planned = raw.trim_end_matches('*').trim();
    if let Some((base, annotation)) = planned.rsplit_once(" (") {
        let annotation = annotation.trim_end_matches(')').trim();
        let shortcut = annotation.starts_with("planned:")
            || annotation.contains("Ctrl+")
            || annotation.contains("Shift+")
            || annotation.contains("Alt+")
            || annotation.contains("Cmd+")
            || annotation.len() == 1 && annotation.chars().all(|c| c.is_ascii_alphanumeric())
            || annotation.strip_prefix('F').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            || matches!(annotation, "Escape" | "Delete" | "Tab" | "Enter" | "Space" | "Backspace" | "Home" | "End" | "Insert");
        if shortcut {
            return base.trim_end_matches('*').trim().to_string();
        }
    }
    raw.trim().to_string()
}

/// Write one `cmd.<command id>` entry per still-untranslated label into the English catalog and
/// drop the newly covered labels from the report.
fn add_missing(root: &Path, report: &mut Report) -> Result<usize, String> {
    if report.untranslated.is_empty() {
        return Ok(0);
    }
    let path = root.join(DIR).join("en.json");
    let mut english = read(root, "en")?;
    // One key per label, so two commands sharing a label cannot collide in `label()`'s
    // English-text → id table. The lowest command id wins, keeping the entry stable.
    let mut chosen: BTreeMap<String, String> = BTreeMap::new();
    for (label, ids) in &report.untranslated {
        if let Some(id) = ids.iter().min() {
            chosen.insert(label.clone(), format!("cmd.{id}"));
        }
    }
    for (label, key) in &chosen {
        if english.contains_key(key) {
            continue;
        }
        english.insert(key.clone(), Value::String(label.clone()));
    }
    english.sort_keys();
    std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(english)).map_err(|e| e.to_string())? + "\n")
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let added = chosen.len();
    report.untranslated.clear();
    Ok(added)
}

fn print_report(report: &Report) {
    for (locale, missing) in &report.missing {
        println!("i18n: {locale} is missing {} English message(s):", missing.len());
        for id in missing {
            println!("      {id}");
        }
    }
    if !report.untranslated.is_empty() {
        println!("i18n: {} command label(s) have no English catalog entry, so they stay English in every locale:", report.untranslated.len());
        for (label, ids) in &report.untranslated {
            println!("      {label:<42} {}", ids.join(", "));
        }
        println!("i18n: run `cargo xtask i18n --add-missing` to add them to en.json, then translate them.");
    }
    if report.problems() == 0 {
        println!("i18n: catalogs complete; every command label is translatable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_counts_both_kinds_of_gap() {
        let report = Report {
            missing: BTreeMap::from([("zh".into(), vec!["a".into(), "b".into()])]),
            untranslated: BTreeMap::from([("Flange".into(), vec!["sheet.flange".into()])]),
        };
        assert_eq!(report.problems(), 3);
    }

    #[test]
    fn an_empty_report_has_no_problems() {
        assert_eq!(Report::default().problems(), 0);
    }
}
