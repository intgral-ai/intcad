//! The flange, web, relief and relief-size rules: `sm.min_flange`,
//! `sm.min_web_parallel_bends`, `sm.u_channel_ratio`, `sm.bend_relief_present`,
//! `sm.bend_relief_width` and `sm.bend_relief_depth`.
//!
//! A port of INTdfm `rules.py`:
//!
//! * `_flanges`, `_ways_to_pass`, `_forms`, `_share`, `_reach`, `_flange_review` and `_no_die`
//!   (`rules.py:1211-1335`), the die-row chart `sm.min_flange` reads;
//! * `_webs_and_channels` (`rules.py:1100-1118`) with `_measured` (`1121-1147`) for
//!   `sm.min_web_parallel_bends` and `sm.u_channel_ratio`;
//! * `_reliefs` and `_line_only` (`rules.py:1345-1407`), which compare no number;
//! * `_per_relief` (`rules.py:1260-1275`) with `_measured` and `_WIDTH`/`_DEPTH`
//!   (`rules.py:153-154`) for the two relief-size rules, which read each notch's width and
//!   depth at the end it sits at.
//!
//! # Faithful, and not
//!
//! Faithful: the status, `error_code`, `reason_code`, `measured` and chosen limit; the die-row
//! algebra (one way to pass per die, a die-less floor checked at share 1, the strictest row for
//! the same V); the `ceil(share * places) - 1` reach index over each flat's runs, then the smaller
//! of the two flats; `FLANGE_NO_DIE` before `FLANGE_CROSSES_CUT`; the `BEND_NOT_90` skip; the
//! `not_applicable` answers for a part with no web or U; and the two relief codes.
//!
//! Messages follow INTdfm's wording and number spelling ([`crate::pyfmt`]). A
//! requirement INTdfm writes with `value: null` (`FLANGE_NO_DIE`, the relief line) carries the
//! selected row's limit here, because [`RequirementRef`] has no optional value; the text says what
//! the number is for. Tests compare the limit only where INTdfm records one.
//!
//! # A flat part
//!
//! A flat part is answered once, by [`crate::bend_rules::flat_finding`]. Each function here
//! returns that same finding when [`is_flat`] is true, so a caller that runs one rule on its own
//! cannot lose the answer, but a caller building a report must add the flat finding once and skip
//! these functions, exactly as `core.py` does.
//!
//! # Why not `choose`
//!
//! `crate::choose` collapses a rule to its single strictest number. `sm.min_flange` cannot use
//! that: each die that forms the bend's radius is a separate way to pass, and a die-less row is a
//! floor beside them. `sm.u_channel_ratio` also needs the `b` term, which the shared selector does
//! not carry yet. [`choices`] therefore returns every applying row, sharing [`crate::limit_mm`]
//! (and repeating its arithmetic only for the `b` term) and the same [`Chosen`]/[`Unsupported`]
//! contract as `choose`.

use std::collections::{BTreeMap, BTreeSet};

pub use crate::bend_rules::{BEND_RELIEF, MIN_FLANGE, MIN_WEB, U_CHANNEL};
use crate::bend_rules::{flat_finding, is_flat};

/// `sm.bend_relief_width`: each notch's width at a bend end.
pub const BEND_RELIEF_WIDTH: &str = "sm.bend_relief_width";
/// `sm.bend_relief_depth`: each notch's depth at a bend end.
pub const BEND_RELIEF_DEPTH: &str = "sm.bend_relief_depth";
use crate::{
    AppState, Applies, BendFact, CheckInput, Chosen, Confirmation, Facts, Feature, Finding, Layer, Limit, Location, Recognition, Requirement,
    RequirementRef, RuleStatus, Unsupported, WebFact, below, limit_mm, pyfmt, round6, state_of,
};

/// A place, from a bend, a web or a relief end, once three finite coordinates are present.
fn location(x: f64, y: f64, z: f64) -> Location {
    Location { x_mm: crate::round3(x), y_mm: crate::round3(y), z_mm: crate::round3(z) }
}

// ---------------------------------------------------------------------------
// sm.min_flange.

/// `sm.min_flange`: each bend's shorter flange, over the share of the bend a die requires, and the
/// floor a number naming no die sets over the whole bend.
pub fn min_flange(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    if is_flat(facts) {
        return vec![flat_finding()];
    }
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(MIN_FLANGE, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, MIN_FLANGE)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, MIN_FLANGE)];
    }
    let sheet = sheet_thickness(input, facts);
    let shut = shut_bends(&found.bends);
    let off = off_90(&found.bends, &shut, &found.webs);
    let mut results = Vec::new();
    let mut met: Vec<(f64, Chosen)> = Vec::new();
    let mut crossed: Vec<usize> = Vec::new();
    for (index, bend) in found.bends.iter().enumerate() {
        let number = index + 1;
        // The number is selected before the 90-degree skip, so an off-90 bend still needs one to
        // exist: INTdfm says `NO_SHOP_NUMBER`, not `BEND_NOT_90`, when there is none.
        let rows = match choices(MIN_FLANGE, Some(bend.inside_radius_mm), None, false, sheet, input) {
            Ok(rows) => rows,
            Err(unsupported) => return vec![chosen_error(MIN_FLANGE, unsupported, false)],
        };
        if shut.contains(&number) {
            continue;
        }
        let (forming, floor) = ways_to_pass(&rows, bend.inside_radius_mm);
        if forming.is_empty() {
            // A die is what is missing, so quote a die's row when there is one.
            let Some(named) = rows.iter().find(|row| row.die.is_some()).or_else(|| rows.first()) else {
                return vec![chosen_error(MIN_FLANGE, no_number(), false)];
            };
            results.push(no_die(number, bend, named));
            continue;
        }
        if bend.crossed {
            crossed.push(number);
            continue;
        }
        let Some(best) = best_forming(bend, &forming) else {
            continue;
        };
        let value = reach(bend, best);
        if below(value, best.limit(), false) {
            results.push(flange_review(number, bend, value, best));
            continue;
        }
        let tightest = match floor {
            Some(floor) => {
                let whole = reach(bend, floor);
                if below(whole, floor.limit(), false) {
                    results.push(flange_review(number, bend, whole, floor));
                    continue;
                }
                // The tighter of the die's own share and the floor over the whole bend.
                if whole - floor.limit() < value - best.limit() { (whole, floor.chosen.clone()) } else { (value, best.chosen.clone()) }
            }
            None => (value, best.chosen.clone()),
        };
        met.push(tightest);
    }
    if let Some(finding) = bends_clear(MIN_FLANGE, "bend", "bend", &met, bends_gate) {
        results.push(finding);
    }
    if let Some(text) = off {
        results.push(not_measured(MIN_FLANGE, "BEND_NOT_90", &text));
    }
    if !crossed.is_empty() {
        results.push(crossed_finding(&crossed));
    }
    results
}

/// `rules._ways_to_pass`: every die that forms this radius is a way to pass, one row per V, plus
/// the die-less floor. A row naming no die holds over the whole bend, so beside dies it is a floor
/// checked at share 1; if no row names a die, the strictest floor is the only way to pass.
fn ways_to_pass(rows: &[Row], radius: f64) -> (Vec<&Row>, Option<&Row>) {
    let floors: Vec<&Row> = rows.iter().filter(|row| row.die.is_none()).collect();
    // First-wins on a tie, as Python's `max` does.
    let mut floor: Option<&Row> = None;
    for row in &floors {
        if floor.is_none_or(|current| row.limit() > current.limit()) {
            floor = Some(row);
        }
    }
    if let Some(floor) = floor
        && floors.len() == rows.len()
    {
        return (vec![floor], None);
    }
    let mut strictest: Vec<(f64, &Row)> = Vec::new();
    for row in rows {
        let Some(die) = row.die.as_ref() else {
            continue;
        };
        if !forms(die, radius) {
            continue;
        }
        match strictest.iter_mut().find(|(opening, _)| *opening == die.v_mm) {
            Some((_, current)) => {
                if row.limit() > current.limit() {
                    *current = row;
                }
            }
            None => strictest.push((die.v_mm, row)),
        }
    }
    (strictest.into_iter().map(|(_, row)| row).collect(), floor)
}

/// `rules._forms`: the die's radius range, rounded before the test so a radius drawn at the end of
/// the range does not read past it.
fn forms(die: &crate::Die, radius: f64) -> bool {
    match (die.forms_radius_mm.first(), die.forms_radius_mm.get(1)) {
        (Some(low), Some(high)) => *low <= round6(radius) && round6(radius) <= *high,
        _ => false,
    }
}

/// `rules._share`: a row naming no die applies over the whole bend.
fn share(row: &Row) -> f64 {
    row.die.as_ref().map_or(1.0, |die| die.required_share)
}

/// `rules._reach`: the length both flats reach the limit over, at least `share` of the bend's
/// length. Each flat's runs are ranked longest first, and the value at index
/// `ceil(share * places) - 1` is the one at least that many places reach; the shorter flat decides.
fn reach(bend: &BendFact, row: &Row) -> f64 {
    let share = share(row);
    let mut shortest: Option<f64> = None;
    for runs in &bend.runs_mm {
        if runs.is_empty() {
            continue;
        }
        let mut ranked = runs.clone();
        ranked.sort_by(|left, right| right.total_cmp(left));
        let index = reach_index(share, ranked.len());
        let value = ranked[index];
        shortest = Some(shortest.map_or(value, |current| current.min(value)));
    }
    shortest.unwrap_or(0.0)
}

fn reach_index(share: f64, places: usize) -> usize {
    if places == 0 {
        return 0;
    }
    let scaled = (share * places as f64).ceil();
    if !scaled.is_finite() || scaled <= 0.0 {
        return 0;
    }
    ((scaled as usize).saturating_sub(1)).min(places - 1)
}

/// `rules._flange_review`: a bend whose flange or floor is short, quoting the die when there is one.
fn flange_review(number: usize, bend: &BendFact, value: f64, row: &Row) -> Finding {
    let on_die = row.die.as_ref().map(|die| format!(" on the {} mm die", pyfmt::g(die.v_mm))).unwrap_or_default();
    let rest = if row.die.is_some() { " No die on this list does better." } else { "" };
    let message = format!(
        "Bend {number}: its shorter flange reaches {} mm over {:.0}% of its length, below {} of {} mm{on_die}.{rest}",
        pyfmt::g(value),
        share(row) * 100.0,
        whose(row),
        pyfmt::g(row.limit())
    );
    review(MIN_FLANGE, "bend", "FLANGE_BELOW_LIMIT", &format!("bend {number}"), Some(bend_location(bend)), value, row, message, None)
}

/// `rules._no_die`: no die on the list forms the bend's inside radius, so the radius is measured
/// and a die is what is missing.
fn no_die(number: usize, bend: &BendFact, row: &Row) -> Finding {
    let radius = bend.inside_radius_mm;
    let requirement =
        RequirementRef { text: format!("A die that forms {} mm. {}", pyfmt::g(radius), row.measuring_line), ..row.chosen.requirement.clone() };
    let message = format!("Bend {number}: no die on this list forms its inside radius of {} mm.", pyfmt::g(radius));
    review(MIN_FLANGE, "bend", "FLANGE_NO_DIE", &format!("bend {number}"), Some(bend_location(bend)), radius, row, message, Some(requirement))
}

/// `rules._flanges`'s `FLANGE_CROSSES_CUT`: one finding naming every bend whose flange crosses a cut.
fn crossed_finding(crossed: &[usize]) -> Finding {
    let names: Vec<String> = crossed.iter().map(|number| format!("bend {number}")).collect();
    let message = format!(
        "{}: on the way to the far edge, a flange crosses a hole, an opening or a side cut, so how far the sheet really reaches wasn't measured.",
        capitalise(&names.join(", "))
    );
    not_measured(MIN_FLANGE, "FLANGE_CROSSES_CUT", &message)
}

// ---------------------------------------------------------------------------
// sm.min_web_parallel_bends and sm.u_channel_ratio.

/// `sm.min_web_parallel_bends`: the flat between two parallel bends, tangent to tangent.
pub fn min_web_parallel_bends(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    if is_flat(facts) {
        return vec![flat_finding()];
    }
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(MIN_WEB, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, MIN_WEB)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, MIN_WEB)];
    }
    let shut = shut_bends(&found.bends);
    let (webs, _) = webs_and_channels(found, &shut);
    let off = off_90(&found.bends, &shut, &found.webs);
    measured(
        MIN_WEB,
        "web",
        "web",
        "width",
        false,
        webs,
        bends_gate,
        off.as_deref(),
        input,
        facts,
        "This part has no parallel bends with a flat between them, so this rule has nothing to measure.",
    )
}

/// `sm.u_channel_ratio`: the taller flange of two parallel bends turning the same way, against a
/// limit that may depend on the base between them.
pub fn u_channel_ratio(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    if is_flat(facts) {
        return vec![flat_finding()];
    }
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(U_CHANNEL, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, U_CHANNEL)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, U_CHANNEL)];
    }
    let shut = shut_bends(&found.bends);
    let (_, channels) = webs_and_channels(found, &shut);
    let off = off_90(&found.bends, &shut, &found.webs);
    measured(
        U_CHANNEL,
        "U",
        "U",
        "taller flange",
        true,
        channels,
        bends_gate,
        off.as_deref(),
        input,
        facts,
        "This part has no parallel bends turning the same way with a flat between them, so this rule has nothing to measure.",
    )
}

/// One web, U or relief notch to measure, with the radius and base its limit may be written in terms of.
struct Measure {
    name: String,
    at: Option<Location>,
    value: f64,
    radius: Option<f64>,
    base: Option<f64>,
    skip: bool,
}

/// `rules._webs_and_channels`: each web, and each U (`same_side`), with the larger of the two bend
/// radii and, for a U, the web as its base.
fn webs_and_channels(found: &Recognition, shut: &BTreeSet<usize>) -> (Vec<Measure>, Vec<Measure>) {
    let mut webs = Vec::new();
    let mut channels = Vec::new();
    for web in &found.webs {
        let (Some(&first), Some(&second)) = (web.bends.first(), web.bends.get(1)) else {
            continue;
        };
        let skip = shut.contains(&first) || shut.contains(&second);
        let widest = widest_radius(found, first, second);
        let at = Some(web_location(web));
        webs.push(Measure {
            name: format!("the web between bends {first} and {second}"),
            at: at.clone(),
            value: web.web_mm,
            radius: Some(widest),
            base: None,
            skip,
        });
        if web.same_side {
            let taller = web.flanges_mm.iter().copied().fold(f64::MIN, f64::max);
            channels.push(Measure {
                name: format!("the U between bends {first} and {second}"),
                at,
                value: if taller == f64::MIN { 0.0 } else { taller },
                radius: Some(widest),
                base: Some(web.web_mm),
                skip,
            });
        }
    }
    (webs, channels)
}

/// The larger of the two bends' inside radii, or 0 when a web names a bend that is not there.
fn widest_radius(found: &Recognition, first: usize, second: usize) -> f64 {
    [first, second]
        .into_iter()
        .filter_map(|number| number.checked_sub(1).and_then(|index| found.bends.get(index)))
        .map(|bend| bend.inside_radius_mm)
        .fold(f64::MIN, f64::max)
        .max(0.0)
}

/// `rules._measured` for web and U: one review per item that misses, one line for the rest, and
/// `BEND_NOT_90` where a bend the limits do not cover bounds the item.
#[allow(clippy::too_many_arguments)]
fn measured(
    rule: &str,
    kind: &str,
    noun: &str,
    what: &str,
    upper: bool,
    items: Vec<Measure>,
    gate: &Finding,
    off: Option<&str>,
    input: &CheckInput,
    facts: &Facts,
    nothing: &str,
) -> Vec<Finding> {
    if items.is_empty() {
        if gate.status != RuleStatus::Pass {
            return vec![same_as_gate(gate, rule)];
        }
        return vec![crate::not_applicable(rule, nothing)];
    }
    let sheet = sheet_thickness(input, facts);
    let mut results = Vec::new();
    let mut met: Vec<(f64, Chosen)> = Vec::new();
    let mut skipped = false;
    for item in &items {
        let rows = match choices(rule, item.radius, item.base, upper, sheet, input) {
            Ok(rows) => rows,
            Err(unsupported) => return vec![chosen_error(rule, unsupported, upper)],
        };
        let Some(row) = strictest(&rows, upper) else {
            return vec![chosen_error(rule, no_number(), upper)];
        };
        if item.skip {
            skipped = true;
            continue;
        }
        if below(item.value, row.limit(), upper) {
            let body = plain_body(&item.name, what, item.value, row, upper);
            results.push(review(rule, kind, rule_error_code(rule), &item.name, item.at.clone(), item.value, row, body, None));
        } else {
            met.push((item.value, row.chosen.clone()));
        }
    }
    if let Some(finding) = bends_clear(rule, kind, noun, &met, gate) {
        results.push(finding);
    }
    if let Some(text) = off
        && skipped
    {
        results.push(not_measured(rule, "BEND_NOT_90", text));
    }
    results
}

fn rule_error_code(rule: &str) -> &'static str {
    match rule {
        MIN_WEB => "WEB_BELOW_LIMIT",
        U_CHANNEL => "U_FLANGE_ABOVE_LIMIT",
        BEND_RELIEF_WIDTH => "RELIEF_WIDTH_BELOW_LIMIT",
        BEND_RELIEF_DEPTH => "RELIEF_DEPTH_BELOW_LIMIT",
        _ => "FLANGE_BELOW_LIMIT",
    }
}

// ---------------------------------------------------------------------------
// sm.bend_relief_present.

const NO_NOTCH: &str = "the sheet carries on past this end of the bend with no notch.";
const NO_MARGIN: &str =
    "past this end of the bend a flat starts right on the bend's line, with no notch past it. No sheet lies beside the bend itself.";

/// `sm.bend_relief_present`: presence only, so no number is compared. A bend end with no notch
/// asks for a look even with no shop, since no number is involved.
pub fn bend_relief_present(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    if is_flat(facts) {
        return vec![flat_finding()];
    }
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(BEND_RELIEF, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, BEND_RELIEF)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, BEND_RELIEF)];
    }
    let sheet = sheet_thickness(input, facts);
    // A missing number does not stop this rule; a coverage gap or an unknown requirement does.
    let shops = match choices(BEND_RELIEF, None, None, false, sheet, input) {
        Ok(rows) => strictest(&rows, false).filter(|row| row.decides()).cloned(),
        Err(unsupported) if unsupported.code == "NO_SHOP_NUMBER" => None,
        Err(unsupported) => return vec![chosen_error(BEND_RELIEF, unsupported, false)],
    };
    let requirement = shops.as_ref().map(line_only);
    let mut results = Vec::new();
    for (index, bend) in found.bends.iter().enumerate() {
        let number = index + 1;
        let ends = [("RELIEF_MISSING", NO_NOTCH, &bend.unrelieved), ("RELIEF_NO_MARGIN", NO_MARGIN, &bend.no_margin)];
        for (code, saw, points) in ends {
            for point in points {
                results.push(relief_end(number, code, saw, point, requirement.clone(), shops.is_some()));
            }
        }
    }
    if !results.is_empty() {
        return results;
    }
    let Some(row) = shops else {
        return vec![chosen_error(BEND_RELIEF, no_number(), false)];
    };
    if bends_gate.status != RuleStatus::Pass {
        return vec![same_as_gate(bends_gate, BEND_RELIEF)];
    }
    vec![relief_pass(found.bends.len(), line_only(&row))]
}

/// `rules._line_only`: a rule with no number quotes only what the shop's answer rests on.
fn line_only(row: &Row) -> RequirementRef {
    RequirementRef {
        value_mm: row.limit(),
        text: row.measuring_line.clone(),
        source: row.chosen.requirement.source.clone(),
        layer: row.chosen.requirement.layer,
        confirmation: row.chosen.requirement.confirmation,
        overridden: row.chosen.requirement.overridden,
    }
}

fn relief_end(number: usize, code: &str, saw: &str, point: &[f64], requirement: Option<RequirementRef>, shops: bool) -> Finding {
    let at = match (point.first(), point.get(1), point.get(2)) {
        (Some(x), Some(y), Some(z)) => Some(location(*x, *y, *z)),
        _ => None,
    };
    Finding {
        id: format!("{BEND_RELIEF}/{code}@{number}"),
        rules: vec![BEND_RELIEF.to_string()],
        status: RuleStatus::NeedsReview,
        state: state_of(RuleStatus::NeedsReview, shops),
        error_code: Some(code.into()),
        reason_code: None,
        message: format!("Bend {number}: {saw} A person decides."),
        feature: Feature { kind: "bend".into(), number: Some(number), description: format!("an end of bend {number}") },
        location: at,
        measured_mm: None,
        requirement,
        needs: vec![if shops {
            "A person to look at this bend.".into()
        } else {
            "The shop's answer on whether it rejects this or adds the notch.".into()
        }],
    }
}

fn relief_pass(bends: usize, requirement: RequirementRef) -> Finding {
    Finding {
        id: format!("{BEND_RELIEF}/pass"),
        rules: vec![BEND_RELIEF.to_string()],
        status: RuleStatus::Pass,
        state: AppState::Passed,
        error_code: None,
        reason_code: None,
        message: format!(
            "No end of the {} has the sheet carrying on with no notch. This says nothing about a notch's size, or anything but this rule.",
            many(bends, "bend")
        ),
        feature: Feature { kind: "bend".into(), number: None, description: many(bends, "bend") },
        location: None,
        measured_mm: None,
        requirement: Some(requirement),
        needs: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// sm.bend_relief_width and sm.bend_relief_depth.

const NO_RELIEF_TO_SIZE: &str = "No bend end on this part has a notch past it to size, so this rule has nothing to measure.";

/// `sm.bend_relief_width`: each notch's width at the bend end it sits at.
pub fn bend_relief_width(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    relief_size(BEND_RELIEF_WIDTH, "relief's width", |relief| relief.width_mm, input, thickness_gate, bends_gate, facts)
}

/// `sm.bend_relief_depth`: each notch's depth at the bend end it sits at.
pub fn bend_relief_depth(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    relief_size(BEND_RELIEF_DEPTH, "relief's depth", |relief| relief.depth_mm, input, thickness_gate, bends_gate, facts)
}

/// The shared body of the two relief-size rules: one measure per notch, at its own end, with the
/// bend's inside radius for a limit written in `r`.
fn relief_size(
    rule: &str,
    what: &str,
    value_of: impl Fn(&crate::ReliefFact) -> f64,
    input: &CheckInput,
    thickness_gate: &Finding,
    bends_gate: &Finding,
    facts: &Facts,
) -> Vec<Finding> {
    if is_flat(facts) {
        return vec![flat_finding()];
    }
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(rule, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, rule)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, rule)];
    }
    let shut = shut_bends(&found.bends);
    let items = per_relief(&found.bends, value_of, &shut);
    let off = off_90(&found.bends, &shut, &found.webs);
    // `_WIDTH`/`_DEPTH` name the count a "relief" but describe the feature as a bend.
    measured(rule, "bend", "relief", what, false, items, bends_gate, off.as_deref(), input, facts, NO_RELIEF_TO_SIZE)
}

/// `rules._per_relief`: each notch past a bend's end, at that end, with its bend's radius.
fn per_relief(bends: &[BendFact], value_of: impl Fn(&crate::ReliefFact) -> f64, shut: &BTreeSet<usize>) -> Vec<Measure> {
    let mut items = Vec::new();
    for (index, bend) in bends.iter().enumerate() {
        let number = index + 1;
        for relief in &bend.reliefs {
            let at = match (relief.at_mm.first(), relief.at_mm.get(1), relief.at_mm.get(2)) {
                (Some(x), Some(y), Some(z)) => Some(location(*x, *y, *z)),
                _ => None,
            };
            items.push(Measure {
                name: format!("an end of bend {number}"),
                at,
                value: value_of(relief),
                radius: Some(bend.inside_radius_mm),
                base: None,
                skip: shut.contains(&number),
            });
        }
    }
    items
}

// ---------------------------------------------------------------------------
// Row selection.

/// One applying row, with the die it names (if any) and its measuring line.
#[derive(Clone)]
struct Row {
    chosen: Chosen,
    die: Option<crate::Die>,
    measuring_line: String,
}

impl Row {
    fn limit(&self) -> f64 {
        self.chosen.limit_mm
    }
}

/// `rules._choices`, returning every applying row rather than one strictest, because the flange
/// chart's rows are alternatives. The source order, gap codes and limit arithmetic are INTdfm's;
/// the `b` term repeats [`crate::limit_mm`]'s arithmetic because the shared selector has no `b`.
fn choices(rule: &str, radius: Option<f64>, base: Option<f64>, upper: bool, sheet: f64, input: &CheckInput) -> Result<Vec<Row>, Unsupported> {
    let wanted = if upper { Limit::Max } else { Limit::Min };
    if let Some(gate) = input.overrides.iter().find(|gate| gate.rule == rule) {
        if gate.limit != wanted {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: format!("This rule takes {} limit; the override is the other way.", if upper { "an upper" } else { "a lower" }),
            });
        }
        let requirement = Requirement {
            rule: gate.rule.clone(),
            limit: gate.limit,
            value: gate.value.clone(),
            inputs: gate.value.keys().filter(|key| *key != "mm").map(|key| (key.clone(), "mm".into())).collect(),
            measuring_line: "A project override.".into(),
            method: "A project override.".into(),
            source: format!("{}: {}", gate.by, gate.reason),
            rights: "The project's own number.".into(),
            applies: Applies::default(),
            exceptions: Vec::new(),
            validation_samples: Vec::new(),
            die: None,
            bends: None,
            angles_deg: None,
            confirmation: Some(Confirmation::TeamSelfReported),
        };
        let thickness = input.declared.thickness_mm.ok_or_else(|| Unsupported {
            code: "COVERAGE_NOT_DECLARED",
            message: "No thickness was declared, so the override can't be worked out.".into(),
        })?;
        let limit = limit_row(&requirement, thickness, sheet, radius, base, upper).ok_or_else(|| Unsupported {
            code: "REQUIREMENT_INPUT_UNKNOWN",
            message: "The override is not a limit this version can work out.".into(),
        })?;
        return Ok(vec![Row {
            chosen: Chosen {
                requirement: RequirementRef {
                    value_mm: limit,
                    layer: Layer::Project,
                    confirmation: Some(Confirmation::TeamSelfReported),
                    source: requirement.source,
                    text: format!("Project override: {} {limit} mm.", if upper { "at most" } else { "at least" }),
                    overridden: true,
                },
                limit_mm: limit,
                decides: false,
            },
            die: None,
            measuring_line: "A project override.".into(),
        }]);
    }
    let shop_has = input.shop.as_ref().is_some_and(|shop| shop.requirements.iter().any(|row| row.rule == rule));
    let (rows, layer) = match (&input.shop, shop_has) {
        (Some(shop), true) => (&shop.requirements, shop.layer),
        _ => (&input.generic.requirements, input.generic.layer),
    };
    let own: Vec<&Requirement> = rows.iter().filter(|row| row.rule == rule).collect();
    if own.is_empty() {
        return Err(no_number());
    }
    if shop_has && (input.declared.process.is_none() || input.declared.material.is_none()) {
        return Err(Unsupported {
            code: "COVERAGE_NOT_DECLARED",
            message: "The selected data can't be looked up without the declared process and material with temper.".into(),
        });
    }
    let applying: Vec<&Requirement> = own.into_iter().filter(|row| crate::applies(row, &input.declared)).collect();
    if applying.is_empty() {
        return Err(Unsupported {
            code: "COVERAGE_NOT_IN_PROFILE",
            message: "The selected data has this rule, but no row applies at the declared process, material and thickness.".into(),
        });
    }
    let thickness = input.declared.thickness_mm.ok_or_else(|| Unsupported {
        code: "COVERAGE_NOT_DECLARED",
        message: "No thickness was declared, so the number can't be selected.".into(),
    })?;
    let mut chosen = Vec::new();
    for row in applying {
        if row.limit != wanted {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: format!("This rule takes {} limit; the selected row runs the other way.", if upper { "an upper" } else { "a lower" }),
            });
        }
        let Some(limit) = limit_row(row, thickness, sheet, radius, base, upper) else {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "The selected row is not a limit this version can work out.".into(),
            });
        };
        let decides = layer != Layer::Generic && layer != Layer::Development;
        chosen.push(Row {
            chosen: Chosen {
                requirement: RequirementRef {
                    value_mm: limit,
                    layer,
                    confirmation: row.confirmation,
                    source: row.source.clone(),
                    text: format!("At {} {} mm. {}", if upper { "most" } else { "least" }, limit, row.measuring_line),
                    overridden: false,
                },
                limit_mm: limit,
                decides,
            },
            die: row.die.clone(),
            measuring_line: row.measuring_line.clone(),
        });
    }
    Ok(chosen)
}

/// [`crate::limit_mm`] with the `b` term `sm.u_channel_ratio` may use.
fn limit_row(requirement: &Requirement, thickness: f64, sheet: f64, radius: Option<f64>, base: Option<f64>, upper: bool) -> Option<f64> {
    if base.is_none() {
        return limit_mm(requirement, thickness, Some(sheet), radius, upper);
    }
    let at = |t: f64| {
        let mut known = BTreeMap::from([("t", t), ("mm", 1.0)]);
        if let Some(radius) = radius {
            known.insert("r", radius);
        }
        if let Some(base) = base {
            known.insert("b", base);
        }
        let mut total = 0.0;
        for (name, number) in &requirement.value {
            let value = known.get(name.as_str())?;
            total += number * value;
        }
        Some(total)
    };
    let declared = at(thickness)?;
    let measured = at(sheet)?;
    Some(if upper { declared.min(measured) } else { declared.max(measured) })
}

/// `rules._strictest`: the lowest number for an upper limit, the highest otherwise. A tie keeps
/// the first row, as Python's `min`/`max` do.
fn strictest(rows: &[Row], upper: bool) -> Option<&Row> {
    let mut best: Option<&Row> = None;
    for row in rows {
        let better = match best {
            None => true,
            Some(current) => {
                if upper {
                    row.limit() < current.limit()
                } else {
                    row.limit() > current.limit()
                }
            }
        };
        if better {
            best = Some(row);
        }
    }
    best
}

/// The die that comes closest to passing: the largest margin of reach over its limit. A tie keeps
/// the first row, as `_flanges`'s `max` does.
fn best_forming<'a>(bend: &BendFact, forming: &[&'a Row]) -> Option<&'a Row> {
    let mut best: Option<&Row> = None;
    for &row in forming {
        let margin = reach(bend, row) - row.limit();
        let better = match best {
            None => true,
            Some(current) => margin > reach(bend, current) - current.limit(),
        };
        if better {
            best = Some(row);
        }
    }
    best
}

fn no_number() -> Unsupported {
    Unsupported {
        code: "NO_SHOP_NUMBER",
        message: "These rules need a shop's own numbers, and there are none for them here, so nothing is known from them.".into(),
    }
}

// ---------------------------------------------------------------------------
// Shared finding builders.

/// `rules._bends_clear`: one line for everything that met the number, at the one nearest to missing
/// it, or the gate's reason when the gate could not say every bend was found.
fn bends_clear(rule: &str, kind: &str, noun: &str, met: &[(f64, Chosen)], gate: &Finding) -> Option<Finding> {
    if met.is_empty() {
        return None;
    }
    if gate.status != RuleStatus::Pass {
        return Some(same_as_gate(gate, rule));
    }
    let (value, chosen) = met.iter().min_by(|left, right| (left.0 - left.1.limit_mm).abs().total_cmp(&(right.0 - right.1.limit_mm).abs()))?;
    if !chosen.decides {
        return Some(Finding {
            id: format!("{rule}/ABOVE_PUBLIC_VALUE"),
            rules: vec![rule.to_string()],
            status: RuleStatus::NotVerified,
            state: AppState::Checked,
            error_code: None,
            reason_code: Some("ABOVE_PUBLIC_VALUE".into()),
            message: "These are clear of the public value, so nothing is known about them: only a shop's own number can say a bend is fine.".into(),
            feature: Feature { kind: kind.into(), number: None, description: format!("{} that met the number", many(met.len(), noun)) },
            location: None,
            measured_mm: Some(*value),
            requirement: Some(chosen.requirement.clone()),
            needs: vec!["The shop's own number for these bends.".into()],
        });
    }
    Some(Finding {
        id: format!("{rule}/pass"),
        rules: vec![rule.to_string()],
        status: RuleStatus::Pass,
        state: AppState::Passed,
        error_code: None,
        reason_code: None,
        message: format!(
            "{} met this shop's number, the tightest at {} mm. This says nothing about anything but this rule.",
            many(met.len(), noun),
            pyfmt::g(*value)
        ),
        feature: Feature { kind: kind.into(), number: None, description: format!("{} that met the number", many(met.len(), noun)) },
        location: None,
        measured_mm: Some(*value),
        requirement: Some(chosen.requirement.clone()),
        needs: Vec::new(),
    })
}

/// `rules._bend_review`: one item's review at its place, with `what` naming the measure.
#[allow(clippy::too_many_arguments)]
fn review(
    rule: &str,
    kind: &str,
    error_code: &str,
    name: &str,
    at: Option<Location>,
    value: f64,
    row: &Row,
    body: String,
    requirement: Option<RequirementRef>,
) -> Finding {
    Finding {
        id: format!("{rule}/{}", name.replace(' ', "-")),
        rules: vec![rule.to_string()],
        status: RuleStatus::NeedsReview,
        state: state_of(RuleStatus::NeedsReview, row.decides()),
        error_code: Some(error_code.into()),
        reason_code: None,
        message: format!("{body} A person decides."),
        feature: Feature { kind: kind.into(), number: None, description: name.into() },
        location: at,
        measured_mm: Some(value),
        requirement: requirement.or_else(|| Some(row.chosen.requirement.clone())),
        needs: vec![if row.decides() { "A person to look at this bend." } else { SHOPS_NUMBER }.into()],
    }
}

/// The default review sentence, before `A person decides.` is appended.
fn plain_body(name: &str, what: &str, value: f64, row: &Row, upper: bool) -> String {
    format!("{}: its {} is {} mm, {} {} of {} mm.", capitalise(name), what, pyfmt::g(value), side_of(upper), whose(row), pyfmt::g(row.limit()))
}

impl Row {
    fn decides(&self) -> bool {
        self.chosen.decides
    }
}

/// The side a rule's message uses: an upper limit is missed above it, everything else below.
fn side_of(upper: bool) -> &'static str {
    if upper { "above" } else { "below" }
}

fn whose(row: &Row) -> &'static str {
    if row.decides() { "this shop's number" } else { "a public value" }
}

/// `rules._same_as_gate`: one cause, so the report groups this with the gate that found it.
fn same_as_gate(bends_gate: &Finding, rule: &str) -> Finding {
    let code = bends_gate.reason_code.clone().unwrap_or_else(|| "BEND_UNRECOGNISED".into());
    Finding {
        id: format!("{rule}/{code}"),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(code),
        message: bends_gate.message.clone(),
        feature: Feature { kind: "part".into(), number: None, description: "whole part".into() },
        location: None,
        measured_mm: bends_gate.measured_mm,
        requirement: None,
        needs: if bends_gate.needs.is_empty() {
            vec!["A person to count this part's bends, because this version can't be sure.".into()]
        } else {
            bends_gate.needs.clone()
        },
    }
}

/// `rules._blocked`: the thickness gate's answer, keeping its measured value unless it was a
/// mismatch.
fn blocked_for(rule: &str, thickness_gate: &Finding) -> Finding {
    let violation = thickness_gate.status == RuleStatus::Violation;
    let code = if violation {
        "THICKNESS_MISMATCH".to_string()
    } else {
        thickness_gate.reason_code.clone().unwrap_or_else(|| "THICKNESS_NOT_DECLARED".to_string())
    };
    Finding {
        id: format!("{rule}/{code}"),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(code),
        message: if violation {
            "The thickness gate found a mismatch, so the rules that depend on thickness did not run.".to_string()
        } else {
            thickness_gate.message.clone()
        },
        feature: Feature { kind: "part".into(), number: None, description: "whole part".into() },
        location: None,
        measured_mm: if violation { None } else { thickness_gate.measured_mm },
        requirement: None,
        needs: thickness_gate.needs.clone(),
    }
}

/// The finding `_choices` gives instead of a number.
fn chosen_error(rule: &str, unsupported: Unsupported, upper: bool) -> Finding {
    // `_no_number` says the same thing for every rule; the selector's message is about the row.
    let unsupported = if unsupported.code == "NO_SHOP_NUMBER" {
        Unsupported {
            code: "NO_SHOP_NUMBER",
            message: "These rules need a shop's own numbers, and there are none for them here, so nothing is known from them.".into(),
        }
    } else {
        unsupported
    };
    let needs = match unsupported.code {
        "NO_SHOP_NUMBER" => vec![SHOPS_NUMBER.into()],
        "REQUIREMENT_INPUT_UNKNOWN" => {
            let words = if upper { "an upper" } else { "a lower" };
            vec![format!("The requirement as {words} limit in millimetres, or in terms of t in mm.")]
        }
        "COVERAGE_NOT_DECLARED" => vec!["The declared process, material with temper and thickness.".into()],
        _ => vec!["A declared value the profile covers.".into()],
    };
    Finding {
        id: format!("{rule}/{}", unsupported.code),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(unsupported.code.into()),
        message: unsupported.message,
        feature: Feature { kind: "part".into(), number: None, description: "whole part".into() },
        location: None,
        measured_mm: None,
        requirement: None,
        needs,
    }
}

/// `rules._not_measured`: one finding for a reason that is not a number.
fn not_measured(rule: &str, code: &str, message: &str) -> Finding {
    let needs = match code {
        "BEND_NOT_90" => vec!["The shop's own numbers for bends at these angles.".into()],
        "FLANGE_CROSSES_CUT" => vec!["A person to measure each such flange up to the cut in its way.".into()],
        _ => vec![SHOPS_NUMBER.into()],
    };
    Finding {
        id: format!("{rule}/{code}"),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(code.into()),
        message: message.into(),
        feature: Feature { kind: "part".into(), number: None, description: "whole part".into() },
        location: None,
        measured_mm: None,
        requirement: None,
        needs,
    }
}

// ---------------------------------------------------------------------------
// Bend geometry shared with the other bend rules.

fn shut_bends(bends: &[BendFact]) -> BTreeSet<usize> {
    bends.iter().enumerate().filter(|(_, bend)| (bend.angle_deg - 90.0).abs() > 5.0).map(|(index, _)| index + 1).collect()
}

/// `rules._off_90`: why the bends in `shut` are not measured.
fn off_90(bends: &[BendFact], shut: &BTreeSet<usize>, webs: &[WebFact]) -> Option<String> {
    if shut.is_empty() {
        return None;
    }
    let names: Vec<String> = bends
        .iter()
        .enumerate()
        .filter(|(index, _)| shut.contains(&(index + 1)))
        .map(|(index, bend)| format!("bend {} at {} degrees", index + 1, pyfmt::g4(bend.angle_deg)))
        .collect();
    let bounds_shut_web = webs.iter().any(|web| web.bends.iter().any(|bend| shut.contains(bend)));
    let also = if bounds_shut_web { ", or a web or U it bounds," } else { "" };
    Some(capitalise(&format!(
        "{}: more than 5 degrees off 90, a placeholder. The limits here are written for 90 degrees, so no such bend{also} was measured.",
        names.join(", ")
    )))
}

/// `rules._sheet`: the model's own t, or the declared one when they round the same at 4 decimals.
fn sheet_thickness(input: &CheckInput, facts: &Facts) -> f64 {
    let Some(declared) = input.declared.thickness_mm else {
        return facts.paired_mm.unwrap_or(0.0);
    };
    let measured = facts.paired_mm.unwrap_or(declared);
    if round4(measured - declared) != 0.0 { measured } else { declared }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn bend_location(bend: &BendFact) -> Location {
    location(bend.x_mm, bend.y_mm, bend.z_mm)
}

fn web_location(web: &WebFact) -> Location {
    location(web.x_mm, web.y_mm, web.z_mm)
}

const SHOPS_NUMBER: &str = "The shop's own number for this rule.";

fn many(count: usize, noun: &str) -> String {
    if count == 1 { format!("{count} {noun}") } else { format!("{count} {noun}s") }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The corpus has no shop table for the flange, web or relief rules, so these messages are pinned
/// to `rules.py` itself rather than to a recording. Labels say so.
#[cfg(test)]
mod message_parity {
    use super::*;

    fn chosen(limit: f64, decides: bool) -> Chosen {
        Chosen {
            requirement: RequirementRef {
                value_mm: limit,
                layer: if decides { crate::Layer::Factory } else { crate::Layer::Generic },
                confirmation: None,
                source: "TEST-ONLY".into(),
                text: String::new(),
                overridden: false,
            },
            limit_mm: limit,
            decides,
        }
    }

    fn bend(radius: f64) -> BendFact {
        BendFact { inside_radius_mm: radius, angle_deg: 90.0, length_mm: 20.0, x_mm: 1.0, y_mm: 2.0, z_mm: 3.0, ..BendFact::default() }
    }

    fn row(limit: f64, decides: bool, die: Option<crate::Die>) -> Row {
        Row { chosen: chosen(limit, decides), die, measuring_line: "TEST-ONLY.".into() }
    }

    #[test]
    fn flange_review_is_the_python_sentence() {
        let die = Some(crate::Die { v_mm: 16.0, forms_radius_mm: vec![2.0, 4.0], required_share: 0.5 });
        let finding = flange_review(1, &bend(3.0), 12.5, &row(15.0, true, die));
        assert_eq!(
            finding.message,
            "Bend 1: its shorter flange reaches 12.5 mm over 50% of its length, below this shop's number of 15 mm on the 16 mm die. No die on this list does better. A person decides."
        );
    }

    #[test]
    fn no_die_is_the_python_sentence() {
        let finding = no_die(1, &bend(3.0), &row(15.0, true, None));
        assert_eq!(finding.message, "Bend 1: no die on this list forms its inside radius of 3 mm. A person decides.");
    }

    #[test]
    fn web_body_is_the_python_sentence() {
        let body = plain_body("the web between bends 1 and 2", "width", 5.0, &row(10.0, false, None), false);
        assert_eq!(body, "The web between bends 1 and 2: its width is 5 mm, below a public value of 10 mm.");
        let upper = plain_body("the U between bends 1 and 2", "taller flange", 40.0, &row(30.0, true, None), true);
        assert_eq!(upper, "The U between bends 1 and 2: its taller flange is 40 mm, above this shop's number of 30 mm.");
    }

    #[test]
    fn relief_pass_is_the_python_sentence() {
        let finding = relief_pass(2, line_only(&row(0.0, true, None)));
        assert_eq!(
            finding.message,
            "No end of the 2 bends has the sheet carrying on with no notch. This says nothing about a notch's size, or anything but this rule."
        );
    }

    #[test]
    fn crossed_flange_messages_name_every_bend() {
        let finding = crossed_finding(&[1, 2]);
        assert_eq!(
            finding.message,
            "Bend 1, bend 2: on the way to the far edge, a flange crosses a hole, an opening or a side cut, so how far the sheet really reaches wasn't measured."
        );
    }
}
