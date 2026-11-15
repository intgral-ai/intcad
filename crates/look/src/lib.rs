//! Intcad's look, ready to install into the UI kit (`solidcraft_ui_kit`): its icon masters and
//! colour roles ([`icons`]), its code-drawn glyphs ([`glyphs`]), its IBM Plex interface faces
//! ([`fonts`]) and its built-in themes ([`themes`]). Derived from Intcad's v0.1.0 icon, glyph and
//! theme code (MIT OR Apache-2.0) and Intgral's artwork; the kit ships none of these, so a host
//! that wants this look installs them from here and there is one copy of each.
//!
//! [`install`] installs all of it. A host that wants only some, or these icons with a few masters
//! of its own ([`icons::DATA`] in a `solidcraft_ui_kit::icons::IconSet::extended`), installs the
//! parts instead. Intcad's workspace tabs are the app's, not the look's.

pub mod compass;
pub mod fonts;
pub mod glyphs;
pub mod icons;
pub mod pointer;
pub mod themes;

/// Install Intcad's look: [`install_tables`] and its fonts into `ctx`. Call it before the first
/// frame and in every test context.
pub fn install(ctx: &egui::Context) {
    install_tables();
    fonts::install(ctx);
}

/// Install the look's process-wide tables into the kit: its icon set, glyphs and built-in themes,
/// once per process (later calls find them installed), before preferences are read or the first
/// frame. A host that composes its own fonts on [`fonts::definitions`] calls this and installs
/// those.
pub fn install_tables() {
    icons::install();
    glyphs::install();
    themes::install();
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
