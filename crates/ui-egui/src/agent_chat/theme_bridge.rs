//! The window's theme, as the Assistant's page is told it.
//!
//! The page cannot read egui's palette, and IntCAD's themes are more than dark and light: the
//! built-in theme files (`assets/styles/ui/themes/*.json`, 25 of them), colour-vision modes and
//! the user's own theme files all resolve into [`Tokens`]. This module is the one place that
//! turns the palette the app is drawing with into the JSON the page applies as CSS variables, so
//! adding a built-in theme needs no change here and no change in the Assistant's code.
//!
//! Only semantic values cross the boundary: surfaces, text, borders, states, accent, the font
//! sizes and the control metrics the page mirrors. Nothing is derived from a theme's name.

use crate::theme::Tokens;
use serde_json::{Value, json};

fn hex(color: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

fn alpha_hex(color: egui::Color32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        color.r(),
        color.g(),
        color.b(),
        color.a()
    )
}

/// The palette and metrics the page styles itself with, for the app's current theme.
pub fn json(app: &crate::SolveApp) -> Value {
    let tokens: Tokens = Tokens::get();
    let id = app.preferences.theme.clone();
    let dark = crate::theme::is_dark();
    json!({
        "id": id,
        "dark": dark,
        "colors": {
            // Surfaces, from the window inwards.
            "window": hex(tokens.panel),
            "surface": hex(tokens.panel),
            "panel": hex(tokens.panel_header),
            "raised": hex(tokens.dialog_bg),
            "field": hex(tokens.field),
            "control": hex(tokens.control_fill),
            "popup": hex(tokens.dialog_bg),
            // Text.
            "text": hex(tokens.text),
            "textDim": hex(tokens.text_dim),
            "onAccent": hex(tokens.on_accent),
            // Lines and states.
            "border": hex(tokens.border),
            "borderStrong": alpha_hex(tokens.control_border),
            "hover": alpha_hex(tokens.hover),
            "selected": hex(tokens.selected_bg),
            "selectedText": hex(tokens.selected_ink),
            "accent": hex(tokens.accent),
            "accentSoft": hex(tokens.accent_soft),
            "error": hex(tokens.error),
            "warning": hex(tokens.warning),
            "success": hex(tokens.agent_ok),
        },
        "metrics": {
            // Points, like every other size the app draws with.
            "fontBody": tokens.font_body,
            "fontSmall": tokens.font_small,
            "fontCaption": tokens.font_caption,
            "fontHeading": tokens.font_heading,
            "fontTitle": tokens.font_title,
            "controlHeight": tokens.control_height,
            "radiusSm": tokens.radius_sm,
            "radiusMd": tokens.radius_md,
            "radiusLg": tokens.radius_lg,
            "radiusXl": tokens.radius_xl,
            "space2": tokens.space_2,
            "space4": tokens.space_4,
            "space8": tokens.space_8,
            "space12": tokens.space_12,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in theme resolves to a complete set: an unknown theme falls back to the base
    /// palette rather than producing missing values, and the page never gets `null` colours.
    #[test]
    fn every_builtin_theme_produces_a_complete_palette() {
        for theme in crate::theme_files::builtin() {
            let tokens = crate::theme_files::resolve(theme);
            for color in [
                tokens.panel,
                tokens.panel_header,
                tokens.text,
                tokens.text_dim,
                tokens.border,
                tokens.accent,
                tokens.error,
                tokens.warning,
                tokens.agent_ok,
            ] {
                // A colour with no alpha would be invisible on the page.
                assert!(color.a() > 0, "{} has a transparent colour", theme.id);
            }
            assert!(tokens.font_body > 0.0, "{} has no body font", theme.id);
            assert!(tokens.control_height > 0.0, "{} has no control height", theme.id);
        }
    }
}
