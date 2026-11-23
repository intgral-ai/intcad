//! The Section Analysis as this window edits it: the engine keeps a section as a list of planes
//! (each cutting on its normal's side), and the browser's Analysis row, the Section dialog and
//! the viewport work with the one cut they show. Reading it here keeps that single-plane view in
//! one place, so a later section editor changes this module rather than its callers.

use solidcore_record::Vec3;
#[cfg(test)]
use solidcraft_engine::SectionPlane;
use solidcraft_engine::Session;

/// The cut this window shows of a session's section.
pub(crate) trait SectionCut {
    /// The plane shown: the section's first (none: no section).
    fn cut(&self) -> Option<(Vec3, Vec3)>;
    /// Replace the section with one plane at `cut` (none: no section). Tests set the section
    /// this way; the window itself goes through the `inspect.section` command, as for any edit.
    #[cfg(test)]
    fn set_cut(&mut self, cut: Option<(Vec3, Vec3)>);
}

impl SectionCut for Session {
    fn cut(&self) -> Option<(Vec3, Vec3)> {
        self.section.first().map(|p| (p.origin, p.normal))
    }

    #[cfg(test)]
    fn set_cut(&mut self, cut: Option<(Vec3, Vec3)>) {
        // Id 1: the first plane, hatched at the cap's own angle.
        self.section = cut.map(|(origin, normal)| vec![SectionPlane { id: 1, origin, normal }]).unwrap_or_default();
    }
}
