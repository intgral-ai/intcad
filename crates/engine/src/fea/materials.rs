//! Materials a stress estimate can take by name: a few common grades, each with its stiffness and
//! the lowest yield strength its standard specifies (not a typical value), and where each number
//! comes from. The standards themselves are paywalled: the yields were read from supplier
//! datasheets that quote them (checked 2026-11-16), so each source says so.
//!
//! Plastics are left out on purpose: no standard specifies a minimum yield for PLA or ABS, and a
//! printed part's strength depends on how it was printed.

use super::Material;

/// One grade: what it is called, its numbers (MPa) and their sources.
pub struct Grade {
    /// The name it is reported under.
    pub name: &'static str,
    /// Other ways people write it, compared without case, spaces, hyphens or the family word.
    pub aliases: &'static [&'static str],
    /// The app's generic material it is one grade of (`Aluminum`, `Steel`, `Stainless Steel`).
    pub family: &'static str,
    pub e_mpa: f64,
    pub nu: f64,
    pub yield_mpa: f64,
    /// The thickest material (mm) the yield is specified for, where its standard stops or
    /// specifies thicker material lower; `None` where the source gives no limit (PIL-294 D10).
    pub up_to_mm: Option<f64>,
    pub source: &'static str,
}

pub const GRADES: &[Grade] = &[
    Grade {
        name: "6061-T6 aluminium",
        aliases: &["6061t6", "6061t651", "aa6061t6", "al6061t6"],
        family: "Aluminum",
        e_mpa: 68_900.0,
        nu: 0.33,
        yield_mpa: 240.0,
        up_to_mm: Some(6.3),
        source: "Yield: the ASTM B209M minimum for 6061-T6 sheet and plate 0.15 to 6.3 mm (35 ksi), as quoted by Kaiser Aluminum's \
                 sheet and plate data; thicker plate is not covered. E and Poisson's ratio: typical handbook values (ASM)",
    },
    Grade {
        name: "6082-T6 aluminium",
        aliases: &["6082t6", "6082t651", "aa6082t6", "en aw6082t6", "enaw6082t6"],
        family: "Aluminum",
        e_mpa: 70_000.0,
        nu: 0.3,
        yield_mpa: 240.0,
        up_to_mm: Some(100.0),
        source: "Yield: the EN 485-2 minimum proof stress for 6082-T6 plate over 12.5 to 100 mm, the lowest of its bands (sheet 0.4 to 6 mm \
                 is specified at 260 MPa, plate to 12.5 mm at 255 MPa), as quoted by Righton Blackburns' 6082 data sheet. E and Poisson's \
                 ratio: EN 1999-1-1 design values",
    },
    Grade {
        name: "5052-H32 aluminium",
        aliases: &["5052h32", "aa5052h32", "al5052h32"],
        family: "Aluminum",
        e_mpa: 70_300.0,
        nu: 0.33,
        yield_mpa: 160.0,
        up_to_mm: None,
        source: "Yield: the ASTM B209M minimum for 5052-H32 sheet and plate, as quoted by BlueScope Distribution's 5052 data sheet. E and \
                 Poisson's ratio: typical handbook values (ASM)",
    },
    Grade {
        name: "S235 structural steel",
        aliases: &["s235", "s235jr", "s235j0", "s235j2"],
        family: "Steel",
        e_mpa: 210_000.0,
        nu: 0.3,
        yield_mpa: 235.0,
        up_to_mm: Some(16.0),
        source: "Yield: the EN 10025-2 minimum for S235 up to 16 mm thick (thicker sections are specified lower). E and Poisson's ratio: \
                 EN 1993-1-1 design values",
    },
    Grade {
        name: "S355 structural steel",
        aliases: &["s355", "s355jr", "s355j0", "s355j2"],
        family: "Steel",
        e_mpa: 210_000.0,
        nu: 0.3,
        yield_mpa: 355.0,
        up_to_mm: Some(16.0),
        source: "Yield: the EN 10025-2 minimum for S355 up to 16 mm thick (thicker sections are specified lower). E and Poisson's ratio: \
                 EN 1993-1-1 design values",
    },
    Grade {
        name: "304 stainless steel",
        aliases: &["304", "14301", "aisi304", "ss304", "x5crni1810"],
        family: "Stainless Steel",
        e_mpa: 193_000.0,
        nu: 0.3,
        yield_mpa: 205.0,
        up_to_mm: None,
        source: "Yield: the ASTM A240 minimum for type 304 (30 ksi), lower than EN 10088-2's 210 to 230 MPa for 1.4301, as quoted by Aalco's \
                 and Total Materia's data. E: a typical handbook value (ASM; EN 10088-1 gives 200 GPa). Poisson's ratio: 0.3 assumed, not \
                 found in a checked source",
    },
    // 304L is its own grade, not 304: its lower carbon gives a lower minimum yield, and taking
    // 304's 205 MPa overstated it by a fifth (PIL-294 D5).
    Grade {
        name: "304L stainless steel",
        aliases: &["304l", "14307", "aisi304l", "ss304l", "x2crni189"],
        family: "Stainless Steel",
        e_mpa: 193_000.0,
        nu: 0.3,
        yield_mpa: 170.0,
        up_to_mm: None,
        source: "Yield: the ASTM A240 minimum for type 304L, annealed (25 ksi), as quoted by ATI's 304L data sheet (checked 2026-11-18); \
                 EN 10088-2 specifies 1.4307 higher (200 to 220 MPa). E: a typical handbook value (ASM), as for 304. Poisson's ratio: 0.3 \
                 assumed, not found in a checked source",
    },
];

/// A name as compared: lower case, without spaces, hyphens, dots, slashes or a family word.
fn key(name: &str) -> String {
    let mut k: String = name.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    for word in ["aluminium", "aluminum", "alloy", "stainless", "structural", "steel", "grade", "type"] {
        k = k.replace(word, "");
    }
    k
}

/// The grade called `name`, if it is one of [`GRADES`].
pub fn grade(name: &str) -> Option<&'static Grade> {
    let k = key(name);
    if k.is_empty() {
        return None;
    }
    GRADES.iter().find(|g| key(g.name) == k || g.aliases.iter().any(|a| key(a) == k))
}

/// The grades of the app's generic material `family` (`Aluminum`, ...), for "which one?".
pub fn grades_of(family: &str) -> Vec<&'static str> {
    GRADES.iter().filter(|g| g.family.eq_ignore_ascii_case(family)).map(|g| g.name).collect()
}

/// Every grade's name, for an error that lists them.
pub fn names() -> String {
    GRADES.iter().map(|g| g.name).collect::<Vec<_>>().join(", ")
}

impl Grade {
    pub fn material(&self) -> Material {
        Material { name: self.name.to_string(), e_mpa: self.e_mpa, nu: self.nu, yield_mpa: self.yield_mpa, source: self.source.to_string() }
    }
}
