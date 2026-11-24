//! First-launch adoption of a pre-rename settings or data folder. Compatibility shim (see
//! docs/compatibility.md): when the current folder doesn't exist yet and the legacy one does,
//! the legacy folder is copied over. The legacy folder is never changed or deleted, so an older
//! release installed side by side keeps working.

use std::path::{Path, PathBuf};

/// The Linux app id (Flatpak, desktop entry, AppStream, Wayland `app_id`).
pub const APP_ID: &str = "ai.intgral.intcad";
/// The window/settings key eframe used before Intcad had folders of its own
/// (`ai.storyteller.solvecraft`; docs/compatibility.md §4). It is not a Flatpak app id to adopt
/// from: the app ids upstream SolidCraft used are another product's.
pub const LEGACY_APP_ID: &str = "ai.storyteller.solvecraft";
/// Intcad has no earlier Flatpak app id of its own. The app ids upstream SolidCraft used
/// (`ai.storyteller.solidcraft`, `ai.storyteller.solvecraft`) are upstream's history; their
/// `~/.var/app` folders are another product's and are never read (docs/compatibility.md).
pub const PREVIOUS_APP_IDS: [&str; 0] = [];

/// Most files and nesting copied from a legacy folder (it holds preferences, autosaves and
/// small caches; anything bigger is not ours).
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: usize = 8;

/// Copy `legacy` to `current` if `current` doesn't exist and `legacy` is a folder. Returns
/// whether a copy was made. Symbolic links are skipped. A copy that fails part way leaves what
/// was copied (the next launch sees `current` and doesn't try again) and reports the error.
pub fn adopt_legacy_dir(current: &Path, legacy: &Path) -> std::io::Result<bool> {
    if current.exists() || !legacy.is_dir() || current == legacy {
        return Ok(false);
    }
    let mut budget = MAX_ENTRIES;
    copy_tree(legacy, current, 0, &mut budget)?;
    Ok(true)
}

/// The Flatpak folders a first launch may adopt from. Intcad has no earlier Flatpak app id of
/// its own (`PREVIOUS_APP_IDS` is empty), and the ids upstream SolidCraft used are another
/// product's, so this is empty: an upstream `~/.var/app/ai.storyteller.*` folder is never read
/// (docs/compatibility.md).
pub fn legacy_flatpak_paths(path: &Path) -> Vec<PathBuf> {
    let (Some(id), Some(home)) = (std::env::var_os("FLATPAK_ID"), std::env::var_os("HOME")) else { return Vec::new() };
    flatpak_twins(path, &id, Path::new(&home))
}

fn flatpak_twins(path: &Path, flatpak_id: &std::ffi::OsStr, home: &Path) -> Vec<PathBuf> {
    let apps = home.join(".var").join("app");
    match path.strip_prefix(apps.join(APP_ID)) {
        Ok(rel) if flatpak_id == APP_ID => PREVIOUS_APP_IDS.iter().map(|old| apps.join(old).join(rel)).collect(),
        _ => Vec::new(),
    }
}

fn copy_tree(from: &Path, to: &Path, depth: usize, budget: &mut usize) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    if depth >= MAX_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if *budget == 0 {
            return Ok(());
        }
        *budget -= 1;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, depth + 1, budget)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("solidcraft-legacy-dir-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copies_legacy_folder_once_and_keeps_it() {
        let d = scratch("copy");
        let old = d.join("old");
        std::fs::create_dir_all(old.join("recovery")).unwrap();
        std::fs::write(old.join("graphics.json"), "{\"msaa\":8}").unwrap();
        std::fs::write(old.join("recovery").join("a.sc"), "x").unwrap();
        let new = d.join("solidcraft");
        assert!(adopt_legacy_dir(&new, &old).unwrap());
        assert_eq!(std::fs::read_to_string(new.join("graphics.json")).unwrap(), "{\"msaa\":8}");
        assert!(new.join("recovery").join("a.sc").is_file());
        // The legacy folder is untouched.
        assert!(old.join("graphics.json").is_file() && old.join("recovery").join("a.sc").is_file());
        // Once the current folder exists, nothing is copied again.
        std::fs::write(old.join("later.json"), "{}").unwrap();
        assert!(!adopt_legacy_dir(&new, &old).unwrap());
        assert!(!new.join("later.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn flatpak_twins_do_not_read_upstream_solidcraft_data() {
        let home = Path::new("/home/u");
        let ours = home.join(".var/app/ai.intgral.intcad/data/solidcraft");
        let id = std::ffi::OsStr::new(APP_ID);
        // Intcad never had an earlier Flatpak app id: upstream SolidCraft's/SolveCraft's
        // folders are another product's and are never adopted (docs/compatibility.md).
        assert!(PREVIOUS_APP_IDS.is_empty());
        assert!(flatpak_twins(&ours, id, home).is_empty());
        // The upstream app ids are not ours, in either direction.
        assert!(flatpak_twins(&home.join(".var/app/ai.storyteller.solidcraft/data/solidcraft"), id, home).is_empty());
        assert!(flatpak_twins(&ours, std::ffi::OsStr::new("ai.storyteller.solidcraft"), home).is_empty());
        // Outside our sandbox folder, or in another app's sandbox, there is nothing to map.
        assert!(flatpak_twins(&home.join(".local/share/intcad"), id, home).is_empty());
        assert!(flatpak_twins(&ours, std::ffi::OsStr::new("org.example.Other"), home).is_empty());
    }

    #[test]
    fn the_desktop_entry_is_named_after_the_app_id() {
        // The window's Wayland app_id / X11 class is APP_ID; the desktop names it and its file
        // name, so docks find the entry and its icon.
        let linux = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/platforms/linux");
        let desktop = std::fs::read_to_string(linux.join(format!("{APP_ID}.desktop"))).unwrap();
        assert!(desktop.lines().any(|l| l == format!("StartupWMClass={APP_ID}")));
        assert!(desktop.lines().any(|l| l == format!("Icon={APP_ID}")));
        let metainfo = std::fs::read_to_string(linux.join(format!("{APP_ID}.metainfo.xml.in"))).unwrap();
        assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")));
        // Upstream's earlier app ids are left as upstream wrote them and are not Intcad's to claim.
    }

    #[test]
    fn nothing_to_adopt() {
        let d = scratch("none");
        let new = d.join("solidcraft");
        assert!(!adopt_legacy_dir(&new, &d.join("missing")).unwrap());
        assert!(!new.exists());
        // An existing current folder is never overwritten.
        let old = d.join("old");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("p.json"), "old").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("p.json"), "new").unwrap();
        assert!(!adopt_legacy_dir(&new, &old).unwrap());
        assert_eq!(std::fs::read_to_string(new.join("p.json")).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&d);
    }
}
