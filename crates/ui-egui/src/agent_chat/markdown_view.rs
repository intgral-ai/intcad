//! An assistant's text as the panel shows it: Markdown (paragraphs, headings, lists, quotes,
//! fenced code, bold, italic, code, links), parsed by [`super::markdown`].
//!
//! Parsing and laying out a long reply every frame would show in the frame time while it streams,
//! so the parsed blocks are kept per conversation item and parsed again only when its text
//! changes.
//!
//! The desktop Assistant's conversation is the page in the host's web view; this view stays for
//! the browser build and the old panel's tests.
#![cfg_attr(all(not(target_arch = "wasm32"), not(test)), allow(dead_code))]

use crate::fonts::RichTextExt as _;
use crate::ui_kit::BackingTooltipExt as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::markdown::{self, Block, Span};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Margin, RichText, Stroke, vec2};

use crate::i18n::tr;
use crate::theme::Tokens;

/// The parsed blocks of `src`, kept in the context's temporary memory under `slot` (the item's
/// index) and replaced when the text changes.
fn blocks(ctx: &egui::Context, slot: usize, src: &str) -> Arc<Vec<Block>> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let key = h.finish();
    let id = egui::Id::new(("agent_chat_markdown", slot));
    if let Some((k, b)) = ctx.data(|d| d.get_temp::<(u64, Arc<Vec<Block>>)>(id))
        && k == key
    {
        return b;
    }
    let parsed = Arc::new(markdown::parse(src));
    ctx.data_mut(|d| d.insert_temp(id, (key, parsed.clone())));
    parsed
}

fn job(ui: &egui::Ui, spans: &[Span], size: f32, color: Color32) -> LayoutJob {
    let t = Tokens::get();
    let mut j = LayoutJob::default();
    j.wrap.max_width = ui.available_width();
    let base = FontId::proportional(size);
    for s in spans {
        let plain = TextFormat { font_id: base.clone(), color, ..Default::default() };
        match s {
            Span::Text(x) => j.append(x, 0.0, plain),
            Span::Bold(x) => j.append(x, 0.0, TextFormat { font_id: crate::fonts::semibold(size), color: t.text, ..plain }),
            Span::Italic(x) => j.append(x, 0.0, TextFormat { italics: true, ..plain }),
            Span::Code(x) => {
                j.append(x, 0.0, TextFormat { font_id: FontId::monospace(size - (t.font_button - t.font_body)), background: t.field, ..plain })
            }
            Span::Link { label, .. } => j.append(label, 0.0, TextFormat { color: t.accent, underline: Stroke::new(1.0, t.accent), ..plain }),
        }
    }
    j
}

/// The host of a web address: `example.com` for `https://example.com/a?b`.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// A selectable label of the conversation. egui leaves a label scrolled out of view out of the
/// text selection, so a selection whose start had scrolled away was dropped, and Copy missed the
/// hidden text (#11527). Out of view this one still takes part: it keeps its end of the selection
/// and gives its text to Copy, clipped to nothing so it is not drawn and never moves the selection.
pub(super) fn selectable(ui: &mut egui::Ui, label: egui::Label) -> egui::Response {
    use egui::text_selection::LabelSelectionState;
    let (pos, galley, response) = label.selectable(true).layout_in_ui(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text()));
    // As `egui::Label` draws a label that is not a button.
    let colour = ui.visuals().text_color();
    if ui.is_rect_visible(response.rect) {
        let underline = if response.has_focus() || response.highlighted() { Stroke::new(1.0, colour) } else { Stroke::NONE };
        LabelSelectionState::label_text_selection(ui, &response, pos, galley, colour, underline);
    } else if ui.ctx().plugin::<LabelSelectionState>().lock().has_selection() {
        let clip = ui.clip_rect();
        ui.set_clip_rect(egui::Rect::NOTHING);
        LabelSelectionState::label_text_selection(ui, &response, pos, galley, colour, Stroke::NONE);
        ui.set_clip_rect(clip);
    }
    response
}

/// A paragraph's text, then its links as buttons (a laid-out text cannot be clicked in parts).
fn spans(ui: &mut egui::Ui, spans: &[Span], size: f32, color: Color32) {
    let t = Tokens::get();
    let text = job(ui, spans, size, color);
    selectable(ui, egui::Label::new(text).wrap());
    let links: Vec<(&str, &str)> =
        spans.iter().filter_map(|s| if let Span::Link { label, url } = s { Some((label.as_str(), url.as_str())) } else { None }).collect();
    if !links.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (label, url) in links {
                // The text above already shows the label; the button says where it goes.
                let to = format!("↗ {}", host(url));
                ui.hyperlink_to(RichText::new(to).size(size - (t.font_button - t.font_body)), url).backing_on_hover_text(format!("{label}: {url}"));
            }
        });
    }
}

/// Draw an Anatomy answer with host object names as clickable spans. Plain text keeps the
/// answer's line breaks and simple headings/bullets; only a host reference can be clicked.
pub fn show_objects(ui: &mut egui::Ui, _slot: usize, src: &str, refs: &[crate::agent_chat::object_ref::ObjectRef], on_click: &mut dyn FnMut(usize)) {
    use crate::agent_chat::object_ref::{Segment, segments};
    let t = Tokens::get();
    let base = t.font_button;
    for line in src.lines() {
        if line.trim().is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let (marker, body, size, color) = if let Some(heading) = line.strip_prefix("# ") {
            ("", heading, t.font_heading, t.text)
        } else if let Some(heading) = line.strip_prefix("## ") {
            ("", heading, t.font_subtitle, t.text)
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            ("•", item, base, t.text)
        } else {
            ("", line, base, t.text)
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if !marker.is_empty() {
                ui.label(RichText::new(marker).size(size).color(t.text_dim));
                ui.add_space(4.0);
            }
            for segment in segments(body, refs) {
                match segment {
                    Segment::Text(text) => {
                        ui.label(RichText::new(text).size(size).color(color));
                    }
                    Segment::Object { label, index } => {
                        let link = ui.link(RichText::new(label).size(size).color(t.accent));
                        crate::scenario::publish_handle(&format!("agent_chat:object:{index}"), link.rect.center());
                        if link.clicked() {
                            on_click(index);
                        }
                    }
                }
            }
        });
    }
}

/// Draw `src` as Markdown. `slot`: the conversation item it belongs to.
pub fn show(ui: &mut egui::Ui, slot: usize, src: &str) {
    let t = Tokens::get();
    let size = t.font_button;
    let parsed = blocks(ui.ctx(), slot, src);
    for block in parsed.iter() {
        match block {
            Block::Paragraph(s) => spans(ui, s, size, t.text),
            Block::Heading { level, spans: s } => {
                let size = match level {
                    1 => t.font_article_heading,
                    2 => t.font_subtitle,
                    _ => t.font_heading,
                };
                ui.add_space(4.0);
                selectable(ui, egui::Label::new(RichText::new(markdown::plain(s)).size(size).semibold().color(t.text)).wrap());
            }
            Block::Code { text, .. } => {
                crate::ui_kit::Container::show_backing(
                    ui,
                    egui::Frame::NONE.fill(t.field).corner_radius(t.radius_sm).inner_margin(Margin::same(6)),
                    |ui| {
                        ui.set_width(ui.available_width());
                        selectable(
                            ui,
                            egui::Label::new(RichText::new(text).monospace().size(size - (t.font_button - t.font_body)).color(t.text)).wrap(),
                        );
                        if crate::ui_kit::button(ui, tr("ui.agent_chat_copy"), crate::ui_kit::Button::Small).clicked() {
                            ui.ctx().copy_text(text.clone());
                        }
                    },
                );
            }
            Block::List { ordered, start, items } => {
                for (i, item) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let mark = if *ordered { format!("{}.", start.saturating_add(i as u64)) } else { "•".to_string() };
                        ui.label(RichText::new(mark).size(size).color(t.text_dim));
                        ui.vertical(|ui| spans(ui, item, size, t.text));
                    });
                }
            }
            Block::Quote(s) => {
                ui.horizontal_top(|ui| {
                    let (bar, _) = ui.allocate_exact_size(vec2(3.0, 16.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, t.radius_sm / 4, t.border);
                    ui.vertical(|ui| spans(ui, s, size, t.text_dim));
                });
            }
            Block::Pre(text) => {
                egui::ScrollArea::horizontal().id_salt(("agent_chat_pre", slot, text.len())).show(ui, |ui| {
                    selectable(ui, egui::Label::new(RichText::new(text).monospace().size(t.font_small).color(t.text)));
                });
            }
            Block::Rule => {
                ui.separator();
            }
        }
        ui.add_space(3.0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_link_button_names_the_host() {
        assert_eq!(super::host("https://example.com/a?b#c"), "example.com");
        assert_eq!(super::host("http://localhost:8080/x"), "localhost:8080");
        assert_eq!(super::host("https://a.b"), "a.b");
    }
}
