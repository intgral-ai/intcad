//! Design-for-manufacturing marks over the viewport: what one run found, where it is on the
//! model, and the labels that let a person go from a report row to the part and back.
//!
//! The marks come from the run's `RunFindings` ([`RunFindings`]), read through the fields the
//! shared contract fixes: the run id, the candidate digest, and each finding's id, state, body,
//! sheet, anchor, flat point, named faces and outline. A finding with no anchor is never marked:
//! it stays in the report and cannot be pointed at. Only the newest run's marks are shown; a
//! stale run hides every mark and cancels a locate.
//!
//! One body is one run's sheet: marks for different bodies are separate, even when their holes
//! sit at similar positions, because a mark is addressed by `(run_id, finding_id)`.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, pos2, vec2};
use serde::Deserialize;
use solidcore_record::Vec3;
use std::hash::{Hash, Hasher};

use crate::SolveApp;
use crate::theme::Tokens;
use crate::viewport::Proj;

/// The width and height of a mark's label, in points.
pub const LABEL_WIDTH: f32 = 168.0;
pub const LABEL_HEIGHT: f32 = 22.0;
/// The gap between two labels in a column, and between a label and the model's screen box.
const LABEL_GAP: f32 = 5.0;
const LEADER_GAP: f32 = 10.0;
/// How far inside the viewport a label may sit.
const MARGIN: f32 = 6.0;

/// How fresh the displayed run is. The host decides; the viewport only obeys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Freshness {
    /// No run has been loaded.
    #[default]
    Unknown,
    Current,
    StaleModel,
    StaleConfig,
    StaleBoth,
}

impl Freshness {
    /// Only a current run may draw marks or start a locate.
    pub fn is_current(self) -> bool {
        matches!(self, Freshness::Current)
    }
}

/// Which marks the viewport shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DfmDisplay {
    /// Every mark of the current run.
    #[default]
    All,
    /// Only the selected finding's mark.
    Selected,
    /// No marks; the report itself stays readable.
    Off,
}

/// How loudly a mark speaks. Colour is never the only signal: each tone also has a status word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DfmTone {
    Failed,
    Advisory,
    Dimmed,
}

/// The state of one finding, as the contract spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingState {
    Passed,
    Checked,
    Advisory,
    Failed,
    Unsupported,
    NotApplicable,
    NotRun,
}

impl FindingState {
    fn tone(self) -> DfmTone {
        match self {
            FindingState::Failed => DfmTone::Failed,
            FindingState::Advisory => DfmTone::Advisory,
            _ => DfmTone::Dimmed,
        }
    }
}

/// One finding from a run's `findings.json`, as far as the viewport reads it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub sheet: usize,
    pub body: String,
    pub state: FindingState,
    #[serde(default)]
    pub anchor: Option<[f64; 3]>,
    #[serde(default)]
    pub flat: Option<[f64; 2]>,
    #[serde(default)]
    pub faces: Vec<String>,
    #[serde(default)]
    pub outline: Vec<[f64; 3]>,
}

/// One run's findings, read through the contract's fields. Other fields in the file are ignored:
/// the viewport never needs a report's prose, and must not print it as model geometry.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFindings {
    pub run_id: String,
    pub candidate_digest: String,
    pub findings: Vec<Finding>,
}

/// One drawable mark: its identity, what to tint, what to outline, and how loudly.
#[derive(Clone, Debug, PartialEq)]
pub struct DfmMark {
    pub run_id: String,
    pub finding_id: String,
    pub body: String,
    pub sheet: usize,
    pub state: FindingState,
    pub tone: DfmTone,
    /// The model-space point the label points at. A finding without one makes no mark.
    pub anchor: Vec3,
    pub flat: Option<[f64; 2]>,
    /// Named faces the mark may tint (`sheet:<body>:<panel>:<side>`).
    pub faces: Vec<String>,
    /// The mark's outline in model space, for a hole or a region.
    pub outline: Vec<Vec3>,
}

impl Hash for DfmMark {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.run_id.hash(h);
        self.finding_id.hash(h);
        self.body.hash(h);
        self.sheet.hash(h);
        self.state.hash(h);
        self.tone.hash(h);
        hash_vec3(self.anchor, h);
        self.flat.map(|p| (p[0].to_bits(), p[1].to_bits())).hash(h);
        self.faces.hash(h);
        for point in &self.outline {
            hash_vec3(*point, h);
        }
    }
}

fn hash_vec3<H: Hasher>(p: Vec3, h: &mut H) {
    p.x.to_bits().hash(h);
    p.y.to_bits().hash(h);
    p.z.to_bits().hash(h);
}

impl DfmMark {
    fn from_finding(run_id: &str, finding: &Finding) -> Option<Self> {
        let anchor = finding.anchor?;
        if !anchor.iter().all(|v| v.is_finite()) {
            return None;
        }
        let outline = finding.outline.iter().filter(|p| p.iter().all(|v| v.is_finite())).map(|p| Vec3::new(p[0], p[1], p[2])).collect();
        Some(DfmMark {
            run_id: run_id.to_string(),
            finding_id: finding.id.clone(),
            body: finding.body.clone(),
            sheet: finding.sheet,
            state: finding.state,
            tone: finding.state.tone(),
            anchor: Vec3::new(anchor[0], anchor[1], anchor[2]),
            flat: finding.flat,
            faces: finding.faces.clone(),
            outline,
        })
    }
}

/// The host's DFM state: which run is shown, how fresh it is, which finding is selected, and the
/// marks that follow from those three facts.
#[derive(Clone, Debug)]
pub struct DfmMarksState {
    run_id: Option<String>,
    candidate_digest: Option<String>,
    freshness: Freshness,
    selected: Option<String>,
    display: DfmDisplay,
    marks: Vec<DfmMark>,
    locate: Option<String>,
}

impl Default for DfmMarksState {
    fn default() -> Self {
        Self {
            run_id: None,
            candidate_digest: None,
            freshness: Freshness::Unknown,
            selected: None,
            display: DfmDisplay::All,
            marks: Vec::new(),
            locate: None,
        }
    }
}

impl DfmMarksState {
    /// Take a run's findings. The previous run's marks go, so a re-run never shows two runs at
    /// once. A selected finding the new run does not have is dropped.
    pub fn load(&mut self, run: &RunFindings, freshness: Freshness) {
        self.run_id = Some(run.run_id.clone());
        self.candidate_digest = Some(run.candidate_digest.clone());
        self.freshness = freshness;
        self.marks = run.findings.iter().filter_map(|finding| DfmMark::from_finding(&run.run_id, finding)).collect();
        let selected = self.selected.take().filter(|id| self.marks.iter().any(|mark| &mark.finding_id == id));
        self.selected = selected;
        if !freshness.is_current() {
            self.locate = None;
        }
    }

    /// The report's freshness changed. A non-current report hides every mark and cancels a
    /// running locate, exactly as CONTRACT.md §3 says.
    pub fn set_freshness(&mut self, freshness: Freshness) {
        self.freshness = freshness;
        if !freshness.is_current() {
            self.locate = None;
        }
    }

    pub fn freshness(&self) -> Freshness {
        self.freshness
    }

    pub fn run_id(&self) -> Option<&str> {
        self.run_id.as_deref()
    }

    pub fn candidate_digest(&self) -> Option<&str> {
        self.candidate_digest.as_deref()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The `(run_id, finding_id)` a selection addresses, when the run is current.
    pub fn selected_key(&self) -> Option<(&str, &str)> {
        match (self.run_id.as_deref(), self.selected.as_deref()) {
            (Some(run), Some(id)) if self.freshness.is_current() => Some((run, id)),
            _ => None,
        }
    }

    pub fn display(&self) -> DfmDisplay {
        self.display
    }

    /// The marks currently drawable: none for an off or stale run, and only the selection in
    /// [`DfmDisplay::Selected`].
    pub fn visible_marks(&self) -> Vec<&DfmMark> {
        if !self.freshness.is_current() || self.display == DfmDisplay::Off {
            return Vec::new();
        }
        self.marks.iter().filter(|mark| self.display == DfmDisplay::All || self.selected.as_deref() == Some(mark.finding_id.as_str())).collect()
    }

    pub fn has_marks(&self) -> bool {
        !self.marks.is_empty()
    }

    pub fn display_mode(&mut self, display: DfmDisplay) {
        self.display = display;
        if display == DfmDisplay::Off {
            self.locate = None;
        }
    }

    /// Select a finding by id, as a report row or a label click does. Returns whether the id is
    /// in the current run; a stale run refuses to locate anything.
    pub fn select(&mut self, id: Option<&str>) -> bool {
        if !self.freshness.is_current() {
            return false;
        }
        let Some(id) = id.filter(|id| self.marks.iter().any(|mark| mark.finding_id == *id)) else {
            self.selected = None;
            self.locate = None;
            return false;
        };
        self.selected = Some(id.to_string());
        self.locate = Some(id.to_string());
        true
    }

    /// The finding a locate action is working toward, if any. Stale runs have none.
    pub fn locate_target(&self) -> Option<&str> {
        if self.freshness.is_current() { self.locate.as_deref() } else { None }
    }

    pub fn cancel_locate(&mut self) {
        self.locate = None;
    }

    pub fn clear(&mut self) {
        self.run_id = None;
        self.candidate_digest = None;
        self.freshness = Freshness::Unknown;
        self.selected = None;
        self.marks.clear();
        self.locate = None;
    }
}

/// One label's place on the viewport, for drawing and hit-testing.
#[derive(Clone, Debug, PartialEq)]
pub struct DfmLabel {
    pub run_id: String,
    pub finding_id: String,
    pub tone: DfmTone,
    /// Where the mark is on screen.
    pub anchor: Pos2,
    /// The label's screen rectangle.
    pub rect: Rect,
    /// Where the leader line meets the label.
    pub leader_end: Pos2,
}

/// The screen box of the model's bounds. Empty bounds fall back to a point at the view's centre.
pub fn model_screen_rect(bounds: solidcraft_engine::geom::Aabb3, proj: &Proj) -> Rect {
    let mut min = pos2(f32::MAX, f32::MAX);
    let mut max = pos2(f32::MIN, f32::MIN);
    let mut any = false;
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                if let Some(p) = proj.to_screen(Vec3::new(x, y, z)).filter(|p| p.x.is_finite() && p.y.is_finite()) {
                    any = true;
                    min = pos2(min.x.min(p.x), min.y.min(p.y));
                    max = pos2(max.x.max(p.x), max.y.max(p.y));
                }
            }
        }
    }
    if any { Rect::from_min_max(min, max) } else { Rect::from_center_size(proj.rect.center(), vec2(1.0, 1.0)) }
}

/// The Assistant panel is a place labels must not go. Its rectangle is meaningful whenever the
/// panel drew a positive, finite rectangle this frame: a desktop web host draws its native view
/// inside that same panel, and the view covers everything the app draws, so labels stay out of
/// it there too. A closed panel clears its rectangle, so a hidden panel is no obstacle.
pub fn assistant_obstacle(panel: Option<Rect>) -> Option<Rect> {
    panel.filter(|rect| rect.is_finite() && rect.is_positive())
}

/// Place each mark's label left or right of the model's screen box, in anchor-Y order, without
/// overlaps. A label whose anchor is off screen makes no label.
pub fn layout(marks: &[DfmMark], proj: &Proj, view: Rect, model: Rect, avoid: &[Rect]) -> Vec<DfmLabel> {
    let limit = view.shrink(MARGIN);
    if limit.width() <= 0.0 || limit.height() <= 0.0 {
        return Vec::new();
    }
    let mut left: Vec<(&DfmMark, Pos2)> = Vec::new();
    let mut right: Vec<(&DfmMark, Pos2)> = Vec::new();
    for mark in marks {
        let Some(anchor) = proj.to_screen(mark.anchor).filter(|p| p.x.is_finite() && p.y.is_finite() && view.contains(*p)) else { continue };
        if anchor.x <= model.center().x {
            left.push((mark, anchor));
        } else {
            right.push((mark, anchor));
        }
    }
    let by_y = |a: &(&DfmMark, Pos2), b: &(&DfmMark, Pos2)| a.1.y.total_cmp(&b.1.y).then_with(|| a.0.finding_id.cmp(&b.0.finding_id));
    left.sort_by(by_y);
    right.sort_by(by_y);
    let mut out = Vec::new();
    place_column(&left, true, model, limit, avoid, &mut out);
    place_column(&right, false, model, limit, avoid, &mut out);
    out
}

fn place_column(column: &[(&DfmMark, Pos2)], left: bool, model: Rect, limit: Rect, avoid: &[Rect], out: &mut Vec<DfmLabel>) {
    let width = LABEL_WIDTH.min(limit.width());
    if width <= 0.0 {
        return;
    }
    let x = if left { (model.left() - LEADER_GAP - width).max(limit.left()) } else { (model.right() + LEADER_GAP).min(limit.right() - width) };
    let x = x.clamp(limit.left(), (limit.right() - width).max(limit.left()));
    let mut cursor = limit.top();
    for (mark, anchor) in column {
        let mut y = (anchor.y - LABEL_HEIGHT * 0.5).clamp(limit.top(), (limit.bottom() - LABEL_HEIGHT).max(limit.top()));
        y = y.max(cursor);
        let mut rect = Rect::from_min_size(pos2(x, y), vec2(width, LABEL_HEIGHT));
        for _ in 0..4 {
            let Some(block) = avoid.iter().find(|block| block.intersects(rect)) else { break };
            let next = block.bottom() + LABEL_GAP;
            if next + LABEL_HEIGHT > limit.bottom() {
                break;
            }
            rect = Rect::from_min_size(pos2(x, next), vec2(width, LABEL_HEIGHT));
        }
        if rect.bottom() > limit.bottom() {
            rect = Rect::from_min_size(pos2(x, (limit.bottom() - LABEL_HEIGHT).max(limit.top())), vec2(width, LABEL_HEIGHT));
        }
        cursor = rect.bottom() + LABEL_GAP;
        let leader_end = if left { pos2(rect.right(), rect.center().y) } else { pos2(rect.left(), rect.center().y) };
        out.push(DfmLabel { run_id: mark.run_id.clone(), finding_id: mark.finding_id.clone(), tone: mark.tone, anchor: *anchor, rect, leader_end });
    }
}

/// The label under a pointer.
pub fn hit_test(labels: &[DfmLabel], pos: Pos2) -> Option<&DfmLabel> {
    labels.iter().find(|label| label.rect.contains(pos))
}

fn tone_colour(t: &Tokens, tone: DfmTone) -> Color32 {
    match tone {
        DfmTone::Failed => t.error,
        DfmTone::Advisory => t.warning,
        DfmTone::Dimmed => t.text_dim,
    }
}

fn status_word(state: FindingState) -> String {
    match state {
        FindingState::Failed => crate::i18n::tr("ui.dfm_mark_failed").to_string(),
        FindingState::Advisory => crate::i18n::tr("ui.dfm_mark_advisory").to_string(),
        _ => crate::i18n::tr("ui.dfm_mark_dimmed").to_string(),
    }
}

fn mark_text(mark: &DfmMark) -> String {
    format!("{} · {}", status_word(mark.state), mark.finding_id)
}

/// Draw the labels, let a click choose a finding, and keep the layout for hit-testing.
pub fn show(app: &mut SolveApp, ui: &egui::Ui, proj: &Proj, view: Rect) {
    let t = Tokens::get();
    let marks = app.viewport.dfm_marks.clone();
    if marks.is_empty() {
        app.viewport.dfm_labels.clear();
        return;
    }
    let model = model_screen_rect(crate::viewport::view_bounds(app), proj);
    let obstacle = assistant_obstacle(app.agent_chat.panel_rect);
    let avoid: Vec<Rect> = obstacle.into_iter().collect();
    let labels = layout(&marks, proj, view, model, &avoid);
    let painter: &Painter = ui.painter();
    for label in &labels {
        let colour = tone_colour(&t, label.tone);
        let selected = app.viewport.dfm.selected() == Some(label.finding_id.as_str());
        painter.line_segment([label.anchor, label.leader_end], Stroke::new(1.0, colour.gamma_multiply(0.8)));
        painter.circle_filled(label.anchor, 3.0, colour);
        painter.rect_filled(label.rect, t.radius_sm, t.overlay);
        painter.rect_stroke(
            label.rect,
            t.radius_sm,
            Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.sel_edge } else { colour }),
            egui::StrokeKind::Middle,
        );
        let glyph = match label.tone {
            DfmTone::Failed => "!",
            DfmTone::Advisory => "?",
            DfmTone::Dimmed => "i",
        };
        let text_pos = pos2(label.rect.left() + 8.0, label.rect.center().y);
        painter.text(text_pos, Align2::LEFT_CENTER, glyph, FontId::proportional(t.font_label), colour);
        let text = marks
            .iter()
            .find(|mark| mark.run_id == label.run_id && mark.finding_id == label.finding_id)
            .map_or_else(|| label.finding_id.clone(), mark_text);
        painter.text(pos2(label.rect.left() + 20.0, label.rect.center().y), Align2::LEFT_CENTER, &text, FontId::proportional(t.font_label), t.text);
        let response = ui.interact(label.rect, ui.id().with(("dfm_label", &label.run_id, &label.finding_id)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &text));
        if response.clicked() {
            app.viewport.select_dfm_finding(Some(&label.finding_id));
        }
    }
    app.viewport.dfm_labels = labels;
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_engine::render::Camera;

    fn fixture() -> RunFindings {
        serde_json::from_str(include_str!("test_data/dfm_run_findings.json")).expect("the synthetic run findings")
    }

    fn proj(cam: Camera) -> Proj {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        Proj { cam, vp: cam.view_proj(rect.width() as f64 / rect.height() as f64, 100.0), rect }
    }

    fn model_box() -> Rect {
        Rect::from_min_max(pos2(320.0, 200.0), pos2(480.0, 400.0))
    }

    fn loaded() -> (DfmMarksState, Vec<DfmMark>) {
        let mut state = DfmMarksState::default();
        state.load(&fixture(), Freshness::Current);
        let marks: Vec<DfmMark> = state.visible_marks().into_iter().cloned().collect();
        (state, marks)
    }

    /// A report row and a label click agree on the same `(run_id, finding_id)`, even when two
    /// bodies have holes at nearly the same place.
    #[test]
    fn a_list_choice_and_a_label_click_agree_on_the_run_and_finding() {
        let (mut state, marks) = loaded();
        assert_eq!(marks.len(), 3, "the anchorless finding makes no mark");
        assert!(state.select(Some("sheet-1/F2")));
        assert_eq!(state.selected_key(), Some(("run-7", "sheet-1/F2")));
        assert!(state.select(Some("sheet-2/F2")), "the similar hole on the other body is its own mark");
        assert_eq!(state.selected_key(), Some(("run-7", "sheet-2/F2")));

        let labels = layout(&marks, &proj(Camera::default()), Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), model_box(), &[]);
        let label = labels.iter().find(|label| label.finding_id == "sheet-2/F2").expect("a label for the selected mark");
        let hit = hit_test(&labels, label.rect.center()).expect("the label under its own centre");
        assert_eq!((hit.run_id.as_str(), hit.finding_id.as_str()), ("run-7", "sheet-2/F2"));
        assert!(state.select(Some(&hit.finding_id)));
        assert_eq!(state.selected_key(), Some(("run-7", "sheet-2/F2")));
    }

    /// Two findings on one face are still two marks, each with its own id.
    #[test]
    fn two_findings_on_one_face_keep_their_own_identity() {
        let (mut state, marks) = loaded();
        let first = marks.iter().find(|mark| mark.finding_id == "sheet-1/F2").expect("the first finding");
        let second = marks.iter().find(|mark| mark.finding_id == "sheet-1/F4").expect("the second finding");
        assert_eq!(first.faces, second.faces, "both findings mark the same face");
        assert_ne!(first.anchor, second.anchor);
        assert!(state.select(Some("sheet-1/F4")));
        assert_eq!(state.selected_key(), Some(("run-7", "sheet-1/F4")));
        assert!(state.marks.iter().any(|mark| mark.finding_id == "sheet-1/F2"), "the other finding stays");
    }

    /// A stale report hides every mark, clears a locate, and refuses a new one; a current report
    /// brings the marks back.
    #[test]
    fn a_stale_run_hides_marks_and_refuses_locate() {
        let (mut state, _) = loaded();
        assert!(state.select(Some("sheet-1/F2")));
        assert_eq!(state.locate_target(), Some("sheet-1/F2"));
        state.set_freshness(Freshness::StaleModel);
        assert!(state.visible_marks().is_empty());
        assert_eq!(state.locate_target(), None);
        assert!(!state.select(Some("sheet-1/F4")), "a stale run cannot locate a new finding");
        assert_eq!(state.selected_key(), None);
        state.set_freshness(Freshness::Current);
        assert_eq!(state.visible_marks().len(), 3);
    }

    /// A finding without an anchor stays in the report but is never a mark or a label.
    #[test]
    fn a_finding_without_an_anchor_is_never_highlighted() {
        let (state, marks) = loaded();
        assert!(!marks.iter().any(|mark| mark.finding_id == "sheet-1/F9"));
        let labels = layout(&marks, &proj(Camera::default()), Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), model_box(), &[]);
        assert!(!labels.iter().any(|label| label.finding_id == "sheet-1/F9"));
        assert!(state.has_marks());
    }

    /// A re-run replaces the marks: the old run's ids are gone, and a selection it did not keep is
    /// dropped.
    #[test]
    fn a_rerun_shows_only_the_new_runs_marks() {
        let (mut state, _) = loaded();
        assert!(state.select(Some("sheet-1/F2")));
        let mut next = fixture();
        next.run_id = "run-8".into();
        let mut finding = next.findings[0].clone();
        finding.id = "sheet-1/F7".into();
        next.findings = vec![finding];
        state.load(&next, Freshness::Current);
        let marks: Vec<&DfmMark> = state.visible_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].run_id, "run-8");
        assert_eq!(marks[0].finding_id, "sheet-1/F7");
        assert_eq!(state.selected_key(), None, "the old selection is not in the new run");
    }

    /// A mark's label follows its anchor when the camera turns and zooms.
    #[test]
    fn labels_follow_the_anchor_under_rotation_and_zoom() {
        let (_, marks) = loaded();
        let mark = marks.iter().find(|mark| mark.finding_id == "sheet-1/F2").expect("a mark");
        let view = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut cam = Camera::default();
        let labels = layout(std::slice::from_ref(mark), &proj(cam), view, model_box(), &[]);
        assert_eq!(labels.len(), 1, "the anchor starts on screen");
        let first = labels[0].anchor;
        assert_eq!(Some(first), proj(cam).to_screen(mark.anchor));

        cam.orbit(0.12, 0.08);
        let turned = proj(cam);
        let labels = layout(std::slice::from_ref(mark), &turned, view, model_box(), &[]);
        assert_eq!(labels.len(), 1, "the anchor stays on screen after a small turn");
        assert_eq!(Some(labels[0].anchor), turned.to_screen(mark.anchor));
        assert_ne!(labels[0].anchor, first, "the anchor moved with the camera");

        cam.zoom_at(1.3, None);
        let zoomed = proj(cam);
        let labels = layout(std::slice::from_ref(mark), &zoomed, view, model_box(), &[]);
        assert_eq!(Some(labels[0].anchor), zoomed.to_screen(mark.anchor));
    }

    /// Labels in one column never overlap, and each still points at its own anchor.
    #[test]
    fn labels_in_a_column_do_not_overlap() {
        let (_, marks) = loaded();
        let labels = layout(&marks, &proj(Camera::default()), Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), model_box(), &[]);
        for (i, a) in labels.iter().enumerate() {
            for b in labels.iter().skip(i + 1) {
                assert!(!a.rect.intersects(b.rect), "{} and {} overlap", a.finding_id, b.finding_id);
            }
            let on_side =
                if a.anchor.x <= model_box().center().x { a.leader_end.x >= a.rect.center().x } else { a.leader_end.x <= a.rect.center().x };
            assert!(on_side, "the leader meets the model-facing edge");
        }
    }

    /// A label whose natural place is under the Assistant panel moves out of the panel; a panel
    /// that is not shown is no obstacle.
    #[test]
    fn a_label_moves_out_of_the_assistant_panel() {
        let (_, marks) = loaded();
        let mark = marks.iter().find(|mark| mark.finding_id == "sheet-1/F2").expect("a mark");
        let panel = Rect::from_min_max(pos2(500.0, 240.0), pos2(794.0, 300.0));
        let limit = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)).shrink(MARGIN);
        let anchor = pos2(510.0, 250.0);
        let mut natural = Vec::new();
        place_column(&[(mark, anchor)], false, model_box(), limit, &[], &mut natural);
        assert_eq!(natural.len(), 1);
        assert!(natural[0].rect.intersects(panel), "the natural label would sit under the panel");

        let mut moved = Vec::new();
        place_column(&[(mark, anchor)], false, model_box(), limit, &[panel], &mut moved);
        assert_eq!(moved.len(), 1);
        assert!(!moved[0].rect.intersects(panel), "the label moved out of the panel");
        assert!(moved[0].rect.top() >= panel.bottom(), "it moved below the panel");

        assert_eq!(assistant_obstacle(Some(panel)), Some(panel));
        assert_eq!(assistant_obstacle(None), None, "a closed panel leaves no rectangle");
        assert_eq!(assistant_obstacle(Some(Rect::from_min_size(pos2(500.0, 100.0), vec2(0.0, 200.0)))), None);
    }

    /// The viewport's drawable marks follow the host state: a stale run empties them, a current
    /// run brings them back, and a stale run refuses a selection.
    #[test]
    fn the_viewport_marks_follow_the_run_state() {
        let mut viewport = crate::viewport::ViewportState::default();
        viewport.load_dfm_run(&fixture(), Freshness::Current);
        assert_eq!(viewport.dfm_marks.len(), 3);
        assert!(viewport.select_dfm_finding(Some("sheet-1/F2")));
        viewport.set_dfm_freshness(Freshness::StaleModel);
        assert!(viewport.dfm_marks.is_empty(), "a stale run draws no marks");
        assert_eq!(viewport.dfm.locate_target(), None);
        assert!(!viewport.select_dfm_finding(Some("sheet-1/F4")));
        viewport.set_dfm_freshness(Freshness::Current);
        assert_eq!(viewport.dfm_marks.len(), 3, "a current run brings the marks back");
    }
}
