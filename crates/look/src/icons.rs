//! Intcad's icon set: the SVG masters in `icons/design/svg`, drawn by the kit's renderer
//! (`solidcraft_ui_kit::icons`).
//!
//! The masters, `palette.toml` (every colour role in light and dark, the accents and their
//! default icons) and `aliases.toml` are embedded at build time (see `build.rs`) as [`DATA`],
//! which [`SET`] draws and [`install`] makes the kit's set. The artwork is Intgral's; it names
//! palette roles rather than carrying colours, so themes can recolour it.
//!
//! A host that draws these icons plus a few masters of its own installs
//! `IconSet::extended(DATA, extension)` instead of [`SET`]: the same tables, never a copy.

use solidcraft_ui_kit::icons::{self, IconData, IconSet};

// The compiled tables: `ROLE_NAMES`, `ROLE_SOURCE_HEX`, `PALETTE`, `ACCENT_NAMES`, `ACCENTS`,
// `ACCENT_VARIANTS`, `MASTERS`, `ALIASES` and `ICON_ACCENTS`, as `IconData` names them.
include!(concat!(env!("OUT_DIR"), "/icon_data.rs"));

const _: () = assert!(
    ROLE_NAMES.len() <= icons::MAX_ROLES && ACCENT_NAMES.len() <= icons::MAX_ACCENTS,
    "a theme file must be able to override every role and accent"
);

/// Intcad's icon artwork as data, for [`SET`] or a host's `IconSet::extended` over it.
pub const DATA: IconData = IconData {
    role_names: ROLE_NAMES,
    role_source_hex: ROLE_SOURCE_HEX,
    palette: PALETTE,
    accent_names: ACCENT_NAMES,
    accents: ACCENTS,
    accent_variants: ACCENT_VARIANTS,
    masters: MASTERS,
    aliases: ALIASES,
    icon_accents: ICON_ACCENTS,
    danger_accent: "constraint",
};

/// Intcad's icons.
pub static SET: IconSet = IconSet::new(DATA);

/// Make [`SET`] the icons the kit draws, once per process, before the first frame and in every
/// test context; calling it again does nothing.
pub fn install() {
    icons::install(&SET);
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_ui_kit::icons::PLACEHOLDER;

    #[test]
    fn every_master_parses_and_the_placeholder_exists() {
        for (id, family, source) in MASTERS {
            if let Err(e) = SET.parse(source) {
                panic!("{family}/{id}: {e}");
            }
        }
        assert!(SET.has_master(PLACEHOLDER), "the placeholder master exists");
    }

    #[test]
    fn aliases_name_masters_and_known_accents() {
        for (alias, master, accent) in ALIASES {
            assert!(SET.has_master(master), "{alias} -> {master}");
            assert!(accent.is_empty() || ACCENT_NAMES.contains(accent), "{alias}: accent {accent}");
            assert!(!SET.resolve(alias).placeholder, "{alias} resolves");
        }
    }

    #[test]
    fn every_master_id_resolves_to_itself() {
        for id in SET.master_ids() {
            assert_eq!(SET.resolve(id).master, id);
        }
        assert!(SET.resolve("no_such_icon").placeholder);
    }

    #[test]
    fn tokens_share_the_artworks_primary_strokes() {
        use solidcraft_theme::icon_strokes as tokens;
        let role = ROLE_NAMES.iter().position(|r| *r == "active-3").unwrap();
        let primary = |accent: &str| {
            let items = ACCENTS[SET.accent_index(accent).unwrap()];
            items.iter().find(|(r, _, _)| *r == role).map(|(_, light, dark)| [*light, *dark]).unwrap()
        };
        let variant = |vision: &str, accent: &str| {
            let a = SET.accent_index(accent).unwrap();
            ACCENT_VARIANTS
                .iter()
                .find(|(v, x, r, _, _)| *v == vision && *x == a && *r == role)
                .map(|(_, _, _, light, dark)| [*light, *dark])
                .unwrap()
        };
        assert_eq!(tokens::SKETCH_PRIMARY, primary("sketch"));
        assert_eq!(tokens::CONSTRAINT_PRIMARY, primary("constraint"));
        assert_eq!(tokens::CONSTRUCTION_PRIMARY, primary("construction"));
        assert_eq!(tokens::SKETCH_RED_GREEN, variant("red-green", "sketch"));
        assert_eq!(tokens::CONSTRUCTION_RED_GREEN, variant("red-green", "construction"));
        assert_eq!(tokens::CONSTRAINT_RED_GREEN, variant("red-green", "constraint"));
        assert_eq!(tokens::CONSTRUCTION_BLUE_YELLOW, variant("blue-yellow", "construction"));
        assert_eq!(tokens::CONSTRAINT_BLUE_YELLOW, variant("blue-yellow", "constraint"));
    }

    #[test]
    fn roles_are_complete_and_source_identifiers_unique() {
        assert_eq!(ROLE_NAMES.len(), ROLE_SOURCE_HEX.len());
        assert_eq!(PALETTE.len(), 2);
        assert!(PALETTE.iter().all(|p| p.len() == ROLE_NAMES.len()));
        let mut seen = std::collections::HashSet::new();
        assert!(
            ROLE_NAMES.iter().zip(ROLE_SOURCE_HEX).filter(|(n, _)| **n != "line").all(|(_, h)| seen.insert(*h)),
            "two roles share a source identifier"
        );
        assert!(ROLE_NAMES.contains(&"line"));
    }
}
