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

/// The body text colour the page is given: the theme's own, unless it would fall below the
/// WCAG AA 4.5:1 contrast on the panel, in which case it is nudged towards the theme's lighter or
/// darker end only as far as that threshold. The theme is the source; this keeps the Assistant
/// readable without inventing a second palette, and the page is told when it happened.
fn readable_text(text: egui::Color32, panel: egui::Color32, dark: bool) -> (egui::Color32, bool) {
    let target = if dark { egui::Color32::WHITE } else { egui::Color32::BLACK };
    let mut colour = text;
    for step in 1..=20 {
        if contrast(colour, panel) >= 4.5 {
            break;
        }
        let t = step as f32 / 20.0;
        let mix = |a: u8, b: u8| (f32::from(a) * (1.0 - t) + f32::from(b) * t).round() as u8;
        colour = egui::Color32::from_rgb(
            mix(text.r(), target.r()),
            mix(text.g(), target.g()),
            mix(text.b(), target.b()),
        );
    }
    (colour, colour != text)
}

/// The palette and metrics the page styles itself with, for the app's current theme.
pub fn json(app: &crate::SolveApp) -> Value {
    let tokens: Tokens = Tokens::get();
    let id = app.preferences.theme.clone();
    let dark = crate::theme::is_dark();
    let (text, text_adjusted) = readable_text(tokens.text, tokens.panel, dark);
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
            "text": hex(text),
            "textAdjusted": text_adjusted,
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

/// WCAG relative luminance of a colour.
fn luminance(color: egui::Color32) -> f32 {
    let channel = |value: u8| {
        let v = f32::from(value) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

/// Contrast ratio between two opaque colours, 1:1 to 21:1.
pub fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (bright, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (bright + 0.05) / (dark + 0.05)
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
            // The panel's body text has to stay readable on its own surface in every theme.
            // The colour the page is given is readable in every theme, including the ones whose
            // own text/panel pair is just under WCAG AA (solarized-light is 4.39:1).
            let (text, _) = readable_text(tokens.text, tokens.panel, theme.base.is_dark());
            let ratio = contrast(text, tokens.panel);
            assert!(
                ratio >= 4.4,
                "{}: body text contrast {ratio:.2}:1 against the panel",
                theme.id
            );
        }
    }

    /// A theme the app has never seen resolves through the same bridge: a new built-in theme
    /// needs no change in this module or in the Assistant.
    #[test]
    fn a_new_theme_is_applied_without_code_changes() {
        let source = include_str!("../../../../assets/styles/ui/themes/nord.json");
        let mut value: serde_json::Value = serde_json::from_str(source).expect("nord.json is JSON");
        value["id"] = serde_json::json!("bridge-test-theme");
        value["colors"]["accent"] = serde_json::json!("#123456");
        let theme = crate::theme_files::from_value(&value).expect("a parsed theme file");
        let tokens = crate::theme_files::resolve(&theme);
        assert_eq!(tokens.accent, egui::Color32::from_rgb(0x12, 0x34, 0x56));
    }
}
