//! Materials a stress estimate can take by name: a few common grades, each with its stiffness and
//! the lowest yield strength its standard specifies (not a typical value) for each band of
//! thickness, and where each number comes from. The standards themselves are paywalled: the
//! yields were read from supplier datasheets and other secondary sources that quote them
//! (checked 2026-11-16 and 2026-11-20), so each source says so.
//!
//! Plastics are left out on purpose: no standard specifies a minimum yield for PLA or ABS, and a
//! printed part's strength depends on how it was printed.

use super::Material;

/// One band of thickness in a grade's standard, and the minimum yield it specifies there
/// (PIL-294 A11).
#[derive(Debug, PartialEq)]
pub struct Band {
    /// Thicker than this, mm. The first band of a grade includes it: that band starts there.
    pub over_mm: f64,
    /// Up to and including this, mm; infinite where the standard gives no upper limit.
    pub up_to_mm: f64,
    pub yield_mpa: f64,
    /// The product the band is of ("sheet", "plate"), or "" where the standard does not split it
    /// (the reply then says "stock").
    pub form: &'static str,
}

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
    /// The yield of the band most parts in this grade are made from (its first band's, or the
    /// lowest where the bands differ little); the one reported when no thickness is known.
    pub yield_mpa: f64,
    /// The standard the bands are from, as the reply names it.
    pub standard: &'static str,
    /// The standard's bands, thinnest first, each starting where the one before ends.
    pub bands: &'static [Band],
    pub source: &'static str,
}

/// Where a thickness falls among a grade's bands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    /// In this band.
    In(&'static Band),
    /// Thinner than the first band: the standard does not specify the grade this thin.
    Thinner(&'static Band),
    /// Thicker than the last band: the standard does not specify the grade this thick.
    Thicker(&'static Band),
}

/// How far past a band's edge a thickness may be and still count as in it: a part drawn 16 mm
/// thick can measure 16.000001 mm.
const EDGE: f64 = 1e-3;

pub const GRADES: &[Grade] = &[
    Grade {
        name: "6061-T6 aluminium",
        aliases: &["6061t6", "6061t651", "aa6061t6", "al6061t6"],
        family: "Aluminum",
        e_mpa: 68_900.0,
        nu: 0.33,
        yield_mpa: 240.0,
        standard: "ASTM B209M",
        bands: &[
            // T6 sheet 0.15 to 6.30 mm (0.006 to 0.249 in), 35 ksi: Kaiser Aluminum's sheet and
            // plate data (checked 2026-11-16) and EPO Crafter's 6061 guide, citing B209/B209M-21a
            // (epocrafter.com, checked 2026-11-20); Smiths Metal Centres' 6061 data sheet gives
            // 35 ksi from 0.010 in (smithmetal.com, checked 2026-11-20).
            Band { over_mm: 0.15, up_to_mm: 6.3, yield_mpa: 240.0, form: "sheet" },
            // T651 plate over 6.30 to 150 mm (0.250 to 6.000 in), 35 ksi: EPO Crafter's 6061
            // guide, citing B209/B209M-21a; Mecomeco's 6061 page, from Kaiser's data, to 4.000 in
            // (mecomeco.com); both checked 2026-11-20. Over 4.000 in only EPO Crafter gives the
            // yield.
            Band { over_mm: 6.3, up_to_mm: 150.0, yield_mpa: 240.0, form: "plate" },
        ],
        source: "Yield: the ASTM B209M minimum for 6061-T6 sheet 0.15 to 6.3 mm and 6061-T651 plate to 150 mm (35 ksi), as quoted by \
                 Kaiser Aluminum's sheet and plate data and other suppliers' data sheets. E and Poisson's ratio: typical handbook values \
                 (ASM)",
    },
    Grade {
        name: "6082-T6 aluminium",
        aliases: &["6082t6", "6082t651", "aa6082t6", "en aw6082t6", "enaw6082t6"],
        family: "Aluminum",
        e_mpa: 70_000.0,
        nu: 0.3,
        yield_mpa: 240.0,
        standard: "EN 485-2",
        // EN 485-2 minimum proof stress, as quoted by Righton Blackburns' 6082-T6/T651 sheet and
        // plate data sheet (BS EN 485-2; checked 2026-11-16 and 2026-11-20) and, for plate only,
        // Aalco's 6082 plate data sheet (BS EN 485-2:2008; checked 2026-11-20): one UK
        // distributor's data under two names. Smiths Metal Centres' M82 data sheet
        // (smithmetal.com) gives the same 255 and 240 MPa for plate; its band edges did not
        // survive text extraction. The sheets print each range as "6.00 to 12.5"; which band an
        // edge belongs to follows EN 485-2's "over ... up to" columns.
        bands: &[
            Band { over_mm: 0.4, up_to_mm: 6.0, yield_mpa: 260.0, form: "sheet" },
            Band { over_mm: 6.0, up_to_mm: 12.5, yield_mpa: 255.0, form: "plate" },
            Band { over_mm: 12.5, up_to_mm: 100.0, yield_mpa: 240.0, form: "plate" },
            // Only the Righton Blackburns and Aalco sheets give this band (tensile 275 MPa).
            Band { over_mm: 100.0, up_to_mm: 150.0, yield_mpa: 240.0, form: "plate" },
        ],
        source: "Yield: the EN 485-2 minimum proof stress for 6082-T6 sheet 0.4 to 6 mm (260 MPa) and 6082-T651 plate to 150 mm (255 MPa \
                 to 12.5 mm, then 240 MPa), as quoted by Righton Blackburns' and Aalco's 6082 data sheets. E and Poisson's ratio: EN \
                 1999-1-1 design values",
    },
    Grade {
        name: "5052-H32 aluminium",
        aliases: &["5052h32", "aa5052h32", "al5052h32"],
        family: "Aluminum",
        e_mpa: 70_300.0,
        nu: 0.33,
        yield_mpa: 160.0,
        standard: "ASTM B209M",
        bands: &[
            // 160 MPa (23 ksi): BlueScope Distribution's 5052 data sheet, which cites B209M-14,
            // for 0.63 to 50 mm (checked 2026-11-16 and 2026-11-20); American Steel &
            // Aluminum's B209 page gives the same 23 ksi for sheet (checked 2026-11-20). The 50 mm
            // upper edge rests on BlueScope alone (SAE AMS 4016, not B209, also stops at 2.000
            // in). The thinnest sheet covered is not settled (0.63 mm there, 0.43 mm in AMS
            // 4016), so no lower edge is used.
            Band { over_mm: 0.0, up_to_mm: 50.0, yield_mpa: 160.0, form: "" },
        ],
        source: "Yield: the ASTM B209M minimum for 5052-H32 sheet and plate up to 50 mm, as quoted by BlueScope Distribution's 5052 data \
                 sheet. E and Poisson's ratio: typical handbook values (ASM)",
    },
    Grade {
        name: "S235 structural steel",
        aliases: &["s235", "s235jr", "s235j0", "s235j2"],
        family: "Steel",
        e_mpa: 210_000.0,
        nu: 0.3,
        yield_mpa: 235.0,
        standard: "EN 10025-2",
        // EN 10025-2 minimum upper yield strength ReH by nominal thickness, as quoted by
        // steels-supplier.com's EN 10025-2 page, Roymech's BS EN 10025-2 table and Sider
        // Ticino's S235JR data sheet (checked 2026-11-20; all three agree; none names an
        // edition). Over 250 mm the standard covers flat products only, and per steels-supplier
        // and Roymech the J2 quality only, so the bands stop there.
        bands: &[
            Band { over_mm: 0.0, up_to_mm: 16.0, yield_mpa: 235.0, form: "" },
            Band { over_mm: 16.0, up_to_mm: 40.0, yield_mpa: 225.0, form: "" },
            Band { over_mm: 40.0, up_to_mm: 63.0, yield_mpa: 215.0, form: "" },
            Band { over_mm: 63.0, up_to_mm: 80.0, yield_mpa: 215.0, form: "" },
            Band { over_mm: 80.0, up_to_mm: 100.0, yield_mpa: 215.0, form: "" },
            Band { over_mm: 100.0, up_to_mm: 150.0, yield_mpa: 195.0, form: "" },
            Band { over_mm: 150.0, up_to_mm: 200.0, yield_mpa: 185.0, form: "" },
            Band { over_mm: 200.0, up_to_mm: 250.0, yield_mpa: 175.0, form: "" },
        ],
        source: "Yield: the EN 10025-2 minimum for S235 by thickness (235 MPa up to 16 mm, lower for thicker), as quoted by steel \
                 suppliers' EN 10025-2 tables. E and Poisson's ratio: EN 1993-1-1 design values",
    },
    Grade {
        name: "S355 structural steel",
        aliases: &["s355", "s355jr", "s355j0", "s355j2"],
        family: "Steel",
        e_mpa: 210_000.0,
        nu: 0.3,
        yield_mpa: 355.0,
        standard: "EN 10025-2",
        // EN 10025-2 minimum ReH by nominal thickness, as quoted by steels-supplier.com, Roymech,
        // Sider Ticino's S355J2 data sheet (the only one to name an edition, 2019) and
        // materialwelding.com's S355J2 page (checked 2026-11-20; all agree). Over 250 mm the
        // standard covers flat products only (265 MPa, Sider Ticino), and per steels-supplier and
        // Roymech the J2 and K2 qualities only, so the bands stop there.
        bands: &[
            Band { over_mm: 0.0, up_to_mm: 16.0, yield_mpa: 355.0, form: "" },
            Band { over_mm: 16.0, up_to_mm: 40.0, yield_mpa: 345.0, form: "" },
            Band { over_mm: 40.0, up_to_mm: 63.0, yield_mpa: 335.0, form: "" },
            Band { over_mm: 63.0, up_to_mm: 80.0, yield_mpa: 325.0, form: "" },
            Band { over_mm: 80.0, up_to_mm: 100.0, yield_mpa: 315.0, form: "" },
            Band { over_mm: 100.0, up_to_mm: 150.0, yield_mpa: 295.0, form: "" },
            Band { over_mm: 150.0, up_to_mm: 200.0, yield_mpa: 285.0, form: "" },
            Band { over_mm: 200.0, up_to_mm: 250.0, yield_mpa: 275.0, form: "" },
        ],
        source: "Yield: the EN 10025-2 minimum for S355 by thickness (355 MPa up to 16 mm, lower for thicker), as quoted by steel \
                 suppliers' EN 10025-2 tables. E and Poisson's ratio: EN 1993-1-1 design values",
    },
    Grade {
        name: "304 stainless steel",
        aliases: &["304", "14301", "aisi304", "ss304", "x5crni1810"],
        family: "Stainless Steel",
        e_mpa: 193_000.0,
        nu: 0.3,
        yield_mpa: 205.0,
        standard: "ASTM A240",
        // A240 gives one minimum for plate, sheet and strip of type 304, annealed: 30 ksi, 205
        // MPa (Aalco, Total Materia, checked 2026-11-16; HT Pipe's A240 page and Penn
        // Stainless, checked 2026-11-20). None of them shows a thickness band or limit.
        bands: &[Band { over_mm: 0.0, up_to_mm: f64::INFINITY, yield_mpa: 205.0, form: "" }],
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
        standard: "ASTM A240",
        // A240 gives one minimum for plate, sheet and strip of type 304L, annealed: 25 ksi, 170
        // MPa (ATI's 304L data sheet, checked 2026-11-18 and 2026-11-20; HT Pipe's A240 page
        // and Penn Stainless, checked 2026-11-20). None of them shows a thickness band or limit.
        bands: &[Band { over_mm: 0.0, up_to_mm: f64::INFINITY, yield_mpa: 170.0, form: "" }],
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

/// The grade `material` was taken from, if it came from the table rather than with the request:
/// its name and source are the grade's (the source may go on to say which band it is from).
pub fn of(material: &Material) -> Option<&'static Grade> {
    GRADES.iter().find(|g| g.name == material.name && material.source.starts_with(g.source))
}

impl Grade {
    pub fn material(&self) -> Material {
        Material { name: self.name.to_string(), e_mpa: self.e_mpa, nu: self.nu, yield_mpa: self.yield_mpa, source: self.source.to_string() }
    }

    /// Where `thickness` (mm) falls among the bands; `None` when it is not a thickness (not
    /// finite, or not above 0) or the grade has no bands.
    pub fn band_at(&'static self, thickness: f64) -> Option<Fit> {
        if !(thickness.is_finite() && thickness > 0.0) {
            return None;
        }
        let (first, last) = (self.bands.first()?, self.bands.last()?);
        if thickness < first.over_mm * (1.0 - EDGE) {
            return Some(Fit::Thinner(first));
        }
        Some(self.bands.iter().find(|b| thickness <= b.up_to_mm * (1.0 + EDGE)).map_or(Fit::Thicker(last), Fit::In))
    }

    /// The first band thicker than `band` whose minimum yield is lower: what thicker stock of the
    /// grade may be.
    pub fn lower_after(&self, band: &Band) -> Option<&'static Band> {
        let at = self.bands.iter().position(|b| std::ptr::eq(b, band))?;
        self.bands.iter().skip(at + 1).find(|b| b.yield_mpa < band.yield_mpa)
    }

    /// The thicknesses the standard specifies the grade for, in words: "from 0.4 to 150 mm",
    /// "up to 250 mm".
    pub fn range_words(&self) -> String {
        match (self.bands.first(), self.bands.last()) {
            (Some(f), Some(l)) if f.over_mm > 0.0 => format!("from {} to {} mm", f.over_mm, l.up_to_mm),
            (Some(_), Some(l)) if l.up_to_mm.is_finite() => format!("up to {} mm", l.up_to_mm),
            _ => "at any thickness".to_string(),
        }
    }

    /// One of its bands in words, with its product, or "stock" where the standard names none:
    /// "sheet 0.15 to 6.3 mm" (a first band that starts above 0), "stock up to 16 mm", "plate over
    /// 12.5 to 100 mm".
    pub fn words(&self, band: &Band) -> String {
        let first = self.bands.first().is_some_and(|f| std::ptr::eq(f, band));
        let range = match (band.over_mm > 0.0, band.up_to_mm.is_finite()) {
            (false, false) => "any thickness".to_string(),
            (false, true) => format!("up to {} mm", band.up_to_mm),
            (true, false) => format!("over {} mm", band.over_mm),
            (true, true) if first => format!("{} to {} mm", band.over_mm, band.up_to_mm),
            (true, true) => format!("over {} to {} mm", band.over_mm, band.up_to_mm),
        };
        format!("{} {range}", if band.form.is_empty() { "stock" } else { band.form })
    }
}
