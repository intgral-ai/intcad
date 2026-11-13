//! The flat test, the bend gate and the first three bend rules.
//!
//! This ports the INTdfm rule layer that a bent part runs before the remaining bend rules:
//!
//! * `is_flat` and `flat_finding` (`rules.py:169-195`), the test that turns a plate into one
//!   `not_applicable` finding covering every bend rule;
//! * `bends_gate` with `_unread` and `_unsupported` (`rules.py:198-270`);
//! * `sm.inside_radius` and `sm.max_bend_length`, through `_per_bend`, `_measured`, `_bends_clear`
//!   and `_not_measured` (`rules.py:1089-1208`);
//! * `sm.bend_radius_consistent` (`rules.py:1410-1458`), which compares no number.
//!
//! # What is faithful, and what is not
//!
//! Faithful: the status, `error_code`, `reason_code`, `measured` and chosen limit of every
//! finding, the order the findings are produced in, the `BEND_NOT_90` skip, the presence-only
//! behaviour of `sm.bend_radius_consistent` (including the 0.05 mm grouping), and the "no shop
//! number" path that leaves a rule unverified rather than failing it.
//!
//! Messages follow INTdfm's wording and number spelling ([`crate::pyfmt`]); the message-parity
//! tests compare them byte for byte. `needs` follows INTdfm's per-code wording.
//! `sm.bend_radius_consistent` writes a requirement value of `0` where INTdfm writes `null`,
//! because [`RequirementRef`] has no optional value; the status and the layer still decide the App
//! state.
//!
//! `crate::hole_finding` is deliberately not used: its feature kind is `hole` and its description
//! is a hole diameter. Bend findings use [`bend_review`] and the other local builders, with the
//! same state mapping.
//!
//! `crate::choose` is used for the two lower-limit rules. It rejects a `max` row by design (the
//! hole rules are all lower limits), so `sm.max_bend_length` needs [`choose_upper`], which shares
//! [`crate::limit_mm`], [`Chosen`] and [`Unsupported`] and mirrors the same source selection.

use std::collections::BTreeSet;

use crate::{
    AppState, Applies, BendFact, CheckInput, Chosen, Confirmation, Facts, Feature, Finding, HOLE_TO_BEND, Layer, Limit, Location, Recognition,
    Requirement, RequirementRef, RuleStatus, SAME_SHEET_MM, Unsupported, WebFact, below, choose, limit_mm, pyfmt, round6, state_of,
};

/// `sm.gate_bends_found`.
pub const BEND_GATE: &str = "sm.gate_bends_found";
/// `sm.inside_radius`.
pub const INSIDE_RADIUS: &str = "sm.inside_radius";
/// `sm.min_flange`.
pub const MIN_FLANGE: &str = "sm.min_flange";
/// `sm.max_bend_length`.
pub const MAX_BEND_LENGTH: &str = "sm.max_bend_length";
/// `sm.bend_relief_present`.
pub const BEND_RELIEF: &str = "sm.bend_relief_present";
/// `sm.min_web_parallel_bends`.
pub const MIN_WEB: &str = "sm.min_web_parallel_bends";
/// `sm.u_channel_ratio`.
pub const U_CHANNEL: &str = "sm.u_channel_ratio";
/// `sm.bend_radius_consistent`.
pub const BEND_RADII: &str = "sm.bend_radius_consistent";

/// Every rule a flat part answers with one finding, in INTdfm's `BEND_RULES` order.
pub const BEND_RULES: &[&str] = &[BEND_GATE, INSIDE_RADIUS, MIN_FLANGE, MAX_BEND_LENGTH, HOLE_TO_BEND, BEND_RELIEF, MIN_WEB, U_CHANNEL, BEND_RADII];

/// `rules._NEAR_90_DEG`: how far off 90 degrees a bend may turn and still be held to a limit.
const NEAR_90_DEG: f64 = 5.0;
/// `rules._SAME_RADIUS_MM`: two inside radii this close, from the smaller, are one.
const SAME_RADIUS_MM: f64 = 0.05;

const SHOPS_NUMBER: &str = "The shop's own number for this rule.";
const SHOPS_NUMBER_ON_BENDS: &str = "The shop's own number for these bends.";
const LOOK_AT_BEND: &str = "A person to look at this bend.";
const COUNT_BY_HAND: &str = "A person to count this part's bends, because this version can't be sure.";
const FLAT_MESSAGE: &str = "All the skins are parallel and every other face runs straight between them, so this part is flat and has no bends.";
const NO_BEND_STEP: &str = "The step that reads bends didn't finish, so nothing is known about them.";

// ---------------------------------------------------------------------------
// The flat test.

/// `rules.is_flat`: a plate cut straight through, with no bend and nothing the recogniser could
/// not label, whose two skins are the sheet's thickness apart.
pub fn is_flat(facts: &Facts) -> bool {
    let (Some(plate_gap), Some(paired)) = (facts.plate_gap_mm, facts.paired_mm) else {
        return false;
    };
    let Some(found) = facts.recognition.as_ref() else {
        return false;
    };
    if !found.bends.is_empty() || found.hems > 0 || found.unknown_faces > 0 || found.sharp_joins > 0 || !found.joined {
        return false;
    }
    same_sheet(plate_gap, paired)
}

/// `gate.same_sheet`: the flat test and a hole's depth use their own 0.1 mm, not the gate's match.
fn same_sheet(one_mm: f64, other_mm: f64) -> bool {
    round6((one_mm - other_mm).abs()) <= SAME_SHEET_MM
}

/// `rules.flat_finding`: one `not_applicable` finding for every bend rule.
pub fn flat_finding() -> Finding {
    Finding {
        id: format!("{BEND_GATE}/not-applicable"),
        rules: BEND_RULES.iter().map(|rule| (*rule).to_string()).collect(),
        status: RuleStatus::NotApplicable,
        state: AppState::Checked,
        error_code: None,
        reason_code: None,
        message: FLAT_MESSAGE.into(),
        feature: part_feature(),
        location: None,
        measured_mm: None,
        requirement: None,
        needs: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// The bend gate.

/// `rules.bends_gate`: whether every bend was found. It only ever lowers an answer, so it never
/// gives a violation.
pub fn bends_gate(input_gate: &Finding, facts: &Facts) -> Finding {
    // INTdfm reaches this branch when the input gate left no usable solid, which is the same
    // condition as an input gate that did not pass.
    if input_gate.status != RuleStatus::Pass {
        return gate_finding(
            RuleStatus::NotVerified,
            None,
            input_gate.reason_code.as_deref(),
            input_gate.message.clone(),
            input_gate.measured_mm,
            input_gate.needs.clone(),
        );
    }
    let Some(found) = facts.recognition.as_ref() else {
        return gate_finding(RuleStatus::NotVerified, None, Some("BEND_UNRECOGNISED"), NO_BEND_STEP.into(), None, vec![COUNT_BY_HAND.into()]);
    };
    let count = found.bends.len();
    let measured = Some(count as f64);
    let unread = unread(found);
    let unsupported = unsupported(found);
    if !unread.is_empty() {
        let also = if unsupported.is_empty() { String::new() } else { format!(" Unsupported: {}.", unsupported.join(", ")) };
        let message = format!("{} found, and there may be more: {}.{also}", many(count, "bend"), unread.join(", "));
        return gate_finding(RuleStatus::NotVerified, None, Some("BEND_UNRECOGNISED"), message, measured, vec![COUNT_BY_HAND.into()]);
    }
    if !unsupported.is_empty() {
        let message = format!("{} found. Unsupported: {}.", many(count, "bend"), unsupported.join(", "));
        let mut finding = crate::unsupported_finding(BEND_GATE, Unsupported { code: "FORMS_UNSUPPORTED", message });
        finding.measured_mm = measured;
        return finding;
    }
    gate_finding(
        RuleStatus::Pass,
        None,
        None,
        format!(
            "{} found. Every face has a label, and skins that aren't in one plane join only through bends. This only says the bends were found.",
            many(count, "bend")
        ),
        measured,
        Vec::new(),
    )
}

/// `rules._unread`: why the bends may not all have been found.
fn unread(found: &Recognition) -> Vec<String> {
    let mut reasons = Vec::new();
    if found.unknown_faces > 0 {
        reasons.push(format!("{} with no label", many(found.unknown_faces, "face")));
    }
    if found.sharp_joins > 0 {
        reasons.push(format!("{} of skins meeting with no bend", many(found.sharp_joins, "pair")));
    }
    if !found.joined {
        reasons.push("flats that no bend joins".into());
    }
    if found.bends.is_empty() && found.hems == 0 {
        reasons.push("no bend, on a part that isn't a plain flat plate".into());
    }
    reasons
}

/// `rules._unsupported`: features no rule here covers.
fn unsupported(found: &Recognition) -> Vec<String> {
    [(found.hems, "hem"), (found.cones, "cone feature"), (found.forms, "form")]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, noun)| many(count, noun))
        .collect()
}

// ---------------------------------------------------------------------------
// sm.inside_radius and sm.max_bend_length.

/// `sm.inside_radius`: each bend's inside radius against the strictest number that applies.
pub fn inside_radius(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    measured_rule(
        INSIDE_RADIUS,
        "inside radius",
        "RADIUS_BELOW_LIMIT",
        false,
        |bend: &BendFact| bend.inside_radius_mm,
        input,
        thickness_gate,
        bends_gate,
        facts,
    )
}

/// `sm.max_bend_length`: each bend's length against an upper limit.
pub fn max_bend_length(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    measured_rule(
        MAX_BEND_LENGTH,
        "length",
        "BEND_LENGTH_ABOVE_LIMIT",
        true,
        |bend: &BendFact| bend.length_mm,
        input,
        thickness_gate,
        bends_gate,
        facts,
    )
}

/// `rules._measured` for one per-bend measure: one review per bend that misses the number, one
/// line for the rest, and `BEND_NOT_90` for the bends the limits do not cover.
fn measured_rule(
    rule: &str,
    what: &str,
    error_code: &str,
    upper: bool,
    value_of: impl Fn(&BendFact) -> f64,
    input: &CheckInput,
    thickness_gate: &Finding,
    bends_gate: &Finding,
    facts: &Facts,
) -> Vec<Finding> {
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(rule, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, rule)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, rule)];
    }
    let chosen = match choose_for(rule, None, upper, input, facts) {
        Ok(chosen) => chosen,
        Err(unsupported) => return vec![chosen_error(rule, unsupported, upper)],
    };
    let shut = shut_bends(&found.bends);
    let off = off_90(&found.bends, &shut, &found.webs);
    let mut results = Vec::new();
    let mut met: Vec<(f64, Chosen)> = Vec::new();
    for (index, bend) in found.bends.iter().enumerate() {
        let number = index + 1;
        if shut.contains(&number) {
            continue;
        }
        let value = value_of(bend);
        if below(value, chosen.limit_mm, upper) {
            results.push(bend_review(rule, error_code, number, bend, value, &chosen, what, upper));
        } else {
            met.push((value, chosen.clone()));
        }
    }
    if let Some(finding) = bends_clear(rule, &met, bends_gate) {
        results.push(finding);
    }
    if let Some(text) = off {
        // `off_90` is `Some` exactly when a bend was skipped, which is INTdfm's
        // `off is not None and any(item.skip)`.
        results.push(not_measured(rule, "BEND_NOT_90", &text));
    }
    results
}

/// `rules._per_bend`'s skip set: a bend more than five degrees off 90 is not held to 90's limits.
fn shut_bends(bends: &[BendFact]) -> BTreeSet<usize> {
    bends.iter().enumerate().filter(|(_, bend)| (bend.angle_deg - 90.0).abs() > NEAR_90_DEG).map(|(index, _)| index + 1).collect()
}

/// `rules._off_90`: why the bends in `shut` are not measured, in one wording.
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
    let text = format!(
        "{}: more than {} degrees off 90, a placeholder. The limits here are written for 90 degrees, so no such bend{also} was measured.",
        names.join(", "),
        pyfmt::g(NEAR_90_DEG)
    );
    Some(capitalise(&text))
}

/// `rules._not_measured`: one finding for a reason that is not a number.
fn not_measured(rule: &str, code: &str, message: &str) -> Finding {
    let needs = match code {
        "BEND_NOT_90" => vec!["The shop's own numbers for bends at these angles.".into()],
        "FLANGE_CROSSES_CUT" => vec!["A person to measure each such flange up to the cut in its way.".into()],
        _ => vec![SHOPS_NUMBER.into()],
    };
    unsupported_like(rule, code, message.to_string(), needs)
}

/// `rules._bends_clear`: one line for every bend that met the number, or the gate's reason when the
/// gate could not say every bend was found.
fn bends_clear(rule: &str, met: &[(f64, Chosen)], bends_gate: &Finding) -> Option<Finding> {
    if met.is_empty() {
        return None;
    }
    if bends_gate.status != RuleStatus::Pass {
        return Some(same_as_gate(bends_gate, rule));
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
            feature: part_feature(),
            location: None,
            measured_mm: Some(*value),
            requirement: Some(chosen.requirement.clone()),
            needs: vec![SHOPS_NUMBER_ON_BENDS.into()],
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
            many(met.len(), "bend"),
            pyfmt::g(*value)
        ),
        feature: Feature { kind: "bend".into(), number: None, description: format!("{} that met the number", many(met.len(), "bend")) },
        location: None,
        measured_mm: Some(*value),
        requirement: Some(chosen.requirement.clone()),
        needs: Vec::new(),
    })
}

/// `rules._bend_review`: one bend's review at its place.
fn bend_review(rule: &str, error_code: &str, number: usize, bend: &BendFact, value: f64, chosen: &Chosen, what: &str, upper: bool) -> Finding {
    let side = if upper { "above" } else { "below" };
    Finding {
        id: format!("{rule}/bend-{number}@{:.3},{:.3},{:.3}", bend.x_mm, bend.y_mm, bend.z_mm),
        rules: vec![rule.to_string()],
        status: RuleStatus::NeedsReview,
        state: state_of(RuleStatus::NeedsReview, chosen.decides),
        error_code: Some(error_code.into()),
        reason_code: None,
        message: format!(
            "Bend {number}: its {what} is {} mm, {side} {} of {} mm. A person decides.",
            pyfmt::g(value),
            whose(chosen),
            pyfmt::g(chosen.limit_mm)
        ),
        feature: bend_feature(number),
        location: Some(Location { x_mm: crate::round3(bend.x_mm), y_mm: crate::round3(bend.y_mm), z_mm: crate::round3(bend.z_mm) }),
        measured_mm: Some(value),
        requirement: Some(chosen.requirement.clone()),
        needs: vec![if chosen.decides { LOOK_AT_BEND } else { SHOPS_NUMBER }.into()],
    }
}

// ---------------------------------------------------------------------------
// sm.bend_radius_consistent.

/// `sm.bend_radius_consistent`: whether every bend shares one inside radius, within 0.05 mm.
/// Presence only: no number is compared, so more than one radius asks for a look even with no shop.
pub fn bend_radius_consistent(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, facts: &Facts) -> Vec<Finding> {
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(BEND_RADII, thickness_gate)];
    }
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, BEND_RADII)];
    };
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, BEND_RADII)];
    }
    // A shop's row can back an answer; a public value, or none at all, cannot.
    let shops = match choose_for(BEND_RADII, None, false, input, facts) {
        Ok(chosen) if chosen.decides => Some(chosen),
        Ok(_) => None,
        Err(unsupported) if unsupported.code == "NO_SHOP_NUMBER" => None,
        Err(unsupported) => return vec![chosen_error(BEND_RADII, unsupported, false)],
    };
    let groups = radius_groups(&found.bends);
    if groups.len() > 1 {
        return vec![radii_review(&groups, shops.as_ref())];
    }
    if bends_gate.status != RuleStatus::Pass {
        return vec![same_as_gate(bends_gate, BEND_RADII)];
    }
    let Some(chosen) = shops else {
        return vec![chosen_error(BEND_RADII, Unsupported { code: "NO_SHOP_NUMBER", message: SHOPS_NUMBER.into() }, false)];
    };
    vec![radii_pass(&chosen, found.bends.len(), groups.first().map(|group| group.0).unwrap_or(0.0))]
}

/// `rules._radii`'s grouping: a radius joins the last group when it is within 0.05 mm of that
/// group's smallest radius.
fn radius_groups(bends: &[BendFact]) -> Vec<(f64, Vec<usize>)> {
    let mut order: Vec<usize> = (0..bends.len()).collect();
    order.sort_by(|left, right| bends[*left].inside_radius_mm.total_cmp(&bends[*right].inside_radius_mm));
    let mut groups: Vec<(f64, Vec<usize>)> = Vec::new();
    for index in order {
        let radius = bends[index].inside_radius_mm;
        let joins = groups.last().is_some_and(|group| round6(radius - group.0) <= SAME_RADIUS_MM);
        if joins {
            if let Some(group) = groups.last_mut() {
                group.1.push(index + 1);
            }
        } else {
            groups.push((radius, vec![index + 1]));
        }
    }
    groups
}

/// `rules._radii`'s review: each radius named with its bends.
fn radii_review(groups: &[(f64, Vec<usize>)], shops: Option<&Chosen>) -> Finding {
    let each: Vec<String> =
        groups.iter().map(|(radius, bends)| format!("{} mm at {}", pyfmt::g(pyfmt::round(*radius, 3)), bends_named(bends))).collect();
    Finding {
        id: format!("{BEND_RADII}/BEND_RADII_MIXED"),
        rules: vec![BEND_RADII.to_string()],
        status: RuleStatus::NeedsReview,
        state: state_of(RuleStatus::NeedsReview, shops.is_some()),
        error_code: Some("BEND_RADII_MIXED".into()),
        reason_code: None,
        message: format!(
            "This part's bends have {} inside radii: {}. Each radius can mean another tool. A person decides.",
            groups.len(),
            each.join("; ")
        ),
        feature: part_feature(),
        location: None,
        measured_mm: Some(groups.len() as f64),
        requirement: shops.map(|chosen| chosen.requirement.clone()),
        needs: vec![if shops.is_some() {
            "A person to look at these bends.".into()
        } else {
            "The shop's answer on whether it forms these radii with one tool.".into()
        }],
    }
}

/// `rules._radii`'s pass: every bend shares one inside radius.
fn radii_pass(chosen: &Chosen, bends: usize, radius: f64) -> Finding {
    let who = if bends == 1 { "The one bend has".to_string() } else { format!("All {} have", many(bends, "bend")) };
    Finding {
        id: format!("{BEND_RADII}/pass"),
        rules: vec![BEND_RADII.to_string()],
        status: RuleStatus::Pass,
        state: AppState::Passed,
        error_code: None,
        reason_code: None,
        message: format!("{who} one inside radius, {} mm. This says nothing about anything but this rule.", pyfmt::g(pyfmt::round(radius, 3))),
        feature: Feature { kind: "bend".into(), number: None, description: many(bends, "bend") },
        location: None,
        measured_mm: Some(1.0),
        requirement: Some(chosen.requirement.clone()),
        needs: Vec::new(),
    }
}

/// `rules._bends_named`: "bend 1", or "bends 1, 2 and 3".
fn bends_named(numbers: &[usize]) -> String {
    let mut numbers = numbers.to_vec();
    numbers.sort_unstable();
    let mut names: Vec<String> = numbers.iter().map(|number| number.to_string()).collect();
    let Some(last) = names.pop() else {
        return String::new();
    };
    if names.is_empty() {
        return format!("bend {last}");
    }
    format!("bends {} and {last}", names.join(", "))
}

// ---------------------------------------------------------------------------
// Shared helpers.

/// `rules._sheet`: the model's own t, thicker or thinner than declared, so a limit can be worked
/// out at it too and the stricter kept. Within 0.0001 mm, the 4 decimals a report rounds to, it is
/// the declared t.
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

/// The shared selector, with the model's thickness already collapsed to `sheet`.
fn choose_for(rule: &str, radius: Option<f64>, upper: bool, input: &CheckInput, facts: &Facts) -> Result<Chosen, Unsupported> {
    let sheet = sheet_thickness(input, facts);
    let mut with_sheet = input.clone();
    with_sheet.facts.paired_mm = Some(sheet);
    if upper { choose_upper(rule, radius, &with_sheet) } else { choose(rule, radius, false, &with_sheet) }
}

/// `crate::choose` for an upper limit. Its lower-limit guard rejects a `max` row, which is right for
/// the hole rules and wrong for `sm.max_bend_length`, so the selection is repeated here with the
/// same source order, applicability checks and gap codes.
fn choose_upper(rule: &str, radius: Option<f64>, input: &CheckInput) -> Result<Chosen, Unsupported> {
    if let Some(gate) = input.overrides.iter().find(|gate| gate.rule == rule) {
        if gate.limit != Limit::Max {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "This rule takes an upper limit; the override is a lower limit.".into(),
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
            confirmation: Some(Confirmation::TeamSelfReported),
        };
        let thickness = input.declared.thickness_mm.ok_or_else(|| Unsupported {
            code: "COVERAGE_NOT_DECLARED",
            message: "No thickness was declared, so the override can't be worked out.".into(),
        })?;
        let limit = limit_mm(&requirement, thickness, input.facts.paired_mm, radius, true).ok_or_else(|| Unsupported {
            code: "REQUIREMENT_INPUT_UNKNOWN",
            message: "The override is not a limit this version can work out.".into(),
        })?;
        return Ok(Chosen {
            requirement: RequirementRef {
                value_mm: limit,
                layer: Layer::Project,
                confirmation: Some(Confirmation::TeamSelfReported),
                source: requirement.source,
                text: format!("Project override: at most {limit} mm."),
                overridden: true,
            },
            limit_mm: limit,
            decides: false,
        });
    }
    let shop_has = input.shop.as_ref().is_some_and(|shop| shop.requirements.iter().any(|row| row.rule == rule));
    let (rows, layer) = match (&input.shop, shop_has) {
        (Some(shop), true) => (&shop.requirements, shop.layer),
        _ => (&input.generic.requirements, input.generic.layer),
    };
    let own: Vec<&Requirement> = rows.iter().filter(|row| row.rule == rule).collect();
    if own.is_empty() {
        return Err(Unsupported { code: "NO_SHOP_NUMBER", message: SHOPS_NUMBER.into() });
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
    let mut chosen: Vec<Chosen> = Vec::new();
    for row in applying {
        if row.limit != Limit::Max {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "This rule takes an upper limit; the selected row is a lower limit.".into(),
            });
        }
        let Some(limit) = limit_mm(row, thickness, input.facts.paired_mm, radius, true) else {
            return Err(Unsupported {
                code: "REQUIREMENT_INPUT_UNKNOWN",
                message: "The selected row is not a limit this version can work out.".into(),
            });
        };
        let decides = layer != Layer::Generic && layer != Layer::Development;
        chosen.push(Chosen {
            requirement: RequirementRef {
                value_mm: limit,
                layer,
                confirmation: row.confirmation,
                source: row.source.clone(),
                text: format!("At most {limit} mm. {}", row.measuring_line),
                overridden: false,
            },
            limit_mm: limit,
            decides,
        });
    }
    // An upper limit's strictest number is the lowest.
    chosen
        .into_iter()
        .min_by(|left, right| left.limit_mm.total_cmp(&right.limit_mm))
        .ok_or_else(|| Unsupported { code: "NO_SHOP_NUMBER", message: SHOPS_NUMBER.into() })
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
    unsupported_like(rule, unsupported.code, unsupported.message, needs)
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
        feature: part_feature(),
        location: None,
        measured_mm: bends_gate.measured_mm,
        requirement: None,
        needs: if bends_gate.needs.is_empty() { vec![COUNT_BY_HAND.into()] } else { bends_gate.needs.clone() },
    }
}

/// `rules._blocked`: the thickness gate's answer, or `THICKNESS_MISMATCH`, for every rule that
/// depends on thickness.
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

fn gate_finding(
    status: RuleStatus,
    error_code: Option<&str>,
    reason_code: Option<&str>,
    message: String,
    measured_mm: Option<f64>,
    needs: Vec<String>,
) -> Finding {
    let state = match status {
        RuleStatus::Pass => AppState::Checked,
        RuleStatus::Violation => AppState::Failed,
        _ => AppState::Unsupported,
    };
    Finding {
        id: format!("{BEND_GATE}/{}", error_code.or(reason_code).unwrap_or("pass")),
        rules: vec![BEND_GATE.to_string()],
        status,
        state,
        error_code: error_code.map(str::to_string),
        reason_code: reason_code.map(str::to_string),
        message,
        feature: part_feature(),
        location: None,
        measured_mm,
        requirement: None,
        needs,
    }
}

fn unsupported_like(rule: &str, code: &str, message: String, needs: Vec<String>) -> Finding {
    Finding {
        id: format!("{rule}/{code}"),
        rules: vec![rule.to_string()],
        status: RuleStatus::NotVerified,
        state: AppState::Unsupported,
        error_code: None,
        reason_code: Some(code.into()),
        message,
        feature: part_feature(),
        location: None,
        measured_mm: None,
        requirement: None,
        needs,
    }
}

fn part_feature() -> Feature {
    Feature { kind: "part".into(), number: None, description: "whole part".into() }
}

fn bend_feature(number: usize) -> Feature {
    Feature { kind: "bend".into(), number: Some(number), description: format!("bend {number}") }
}

fn whose(chosen: &Chosen) -> &'static str {
    if chosen.decides { "this shop's number" } else { "a public value" }
}

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

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn many_uses_the_plural_only_after_one() {
        assert_eq!(many(1, "bend"), "1 bend");
        assert_eq!(many(2, "bend"), "2 bends");
        assert_eq!(many(0, "face"), "0 faces");
    }

    #[test]
    fn numbers_in_messages_use_python_spelling() {
        assert_eq!(pyfmt::g(90.0), "90");
        assert_eq!(pyfmt::g(3.5), "3.5");
        assert_eq!(pyfmt::g(-0.0), "-0");
        assert_eq!(pyfmt::g(1_234_567.0), "1.23457e+06");
    }

    #[test]
    fn same_sheet_rounds_before_comparing() {
        assert!(same_sheet(1.5, 1.4));
        assert!(!same_sheet(1.5, 1.39));
    }

    #[test]
    fn radius_groups_chain_from_the_smallest() {
        let bend = |radius: f64| BendFact { inside_radius_mm: radius, ..BendFact::default() };
        let groups = radius_groups(&[bend(3.08), bend(3.0), bend(3.04)]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], (3.0, vec![2, 3]));
        assert_eq!(groups[1], (3.08, vec![1]));
    }

    fn chosen(limit: f64, decides: bool) -> Chosen {
        Chosen {
            requirement: RequirementRef {
                value_mm: limit,
                layer: if decides { Layer::Factory } else { Layer::Generic },
                confirmation: None,
                source: "TEST-ONLY".into(),
                text: String::new(),
                overridden: false,
            },
            limit_mm: limit,
            decides,
        }
    }

    fn bend(radius: f64, angle: f64, length: f64) -> BendFact {
        BendFact { inside_radius_mm: radius, angle_deg: angle, length_mm: length, x_mm: 1.0, y_mm: 2.0, z_mm: 3.0, ..BendFact::default() }
    }

    // The corpus has no shop table for the bend rules, so these messages are pinned to `rules.py`
    // itself rather than to a recording. Labels say so.

    #[test]
    fn inside_radius_review_is_the_python_sentence() {
        let finding = bend_review(INSIDE_RADIUS, "RADIUS_BELOW_LIMIT", 2, &bend(2.5, 90.0, 20.0), 2.5, &chosen(3.0, false), "inside radius", false);
        assert_eq!(finding.message, "Bend 2: its inside radius is 2.5 mm, below a public value of 3 mm. A person decides.");
    }

    #[test]
    fn bend_length_review_is_the_python_sentence() {
        let finding = bend_review(MAX_BEND_LENGTH, "BEND_LENGTH_ABOVE_LIMIT", 1, &bend(2.0, 90.0, 40.0), 40.0, &chosen(30.0, true), "length", true);
        assert_eq!(finding.message, "Bend 1: its length is 40 mm, above this shop's number of 30 mm. A person decides.");
    }

    #[test]
    fn bends_clear_is_the_python_sentence() {
        let gate = gate_finding(RuleStatus::Pass, None, None, "2 bends found.".into(), Some(2.0), Vec::new());
        let met = [(2.5, chosen(3.0, true))];
        let finding = bends_clear(INSIDE_RADIUS, &met, &gate).expect("a clear line");
        assert_eq!(finding.message, "1 bend met this shop's number, the tightest at 2.5 mm. This says nothing about anything but this rule.");
    }

    #[test]
    fn off_90_uses_four_significant_digits_for_the_angle() {
        let bends = [bend(3.0, 92.12345, 20.0), bend(3.0, 95.0, 20.0)];
        let shut = BTreeSet::from([1, 2]);
        let text = off_90(&bends, &shut, &[]).expect("both bends are off 90");
        assert_eq!(
            text,
            "Bend 1 at 92.12 degrees, bend 2 at 95 degrees: more than 5 degrees off 90, a placeholder. The limits here are written for 90 degrees, so no such bend was measured."
        );
    }

    #[test]
    fn radii_messages_are_the_python_sentences() {
        let review = radii_review(&[(3.0, vec![1]), (1.0, vec![2])], None);
        assert_eq!(
            review.message,
            "This part's bends have 2 inside radii: 3 mm at bend 1; 1 mm at bend 2. Each radius can mean another tool. A person decides."
        );
        let one = radii_pass(&chosen(3.0, true), 1, 3.0);
        assert_eq!(one.message, "The one bend has one inside radius, 3 mm. This says nothing about anything but this rule.");
        let two = radii_pass(&chosen(3.0, true), 2, 3.0);
        assert_eq!(two.message, "All 2 bends have one inside radius, 3 mm. This says nothing about anything but this rule.");
    }

    #[test]
    fn flat_needs_the_gap_to_be_the_sheet() {
        let mut facts = Facts {
            plate_gap_mm: Some(1.5),
            paired_mm: Some(1.5),
            recognition: Some(Recognition { joined: true, ..Recognition::default() }),
            ..Facts::default()
        };
        assert!(is_flat(&facts));
        facts.plate_gap_mm = Some(1.7);
        assert!(!is_flat(&facts));
        facts.plate_gap_mm = Some(1.5);
        facts.recognition = Some(Recognition { sharp_joins: 1, joined: true, ..Recognition::default() });
        assert!(!is_flat(&facts));
    }

    #[test]
    fn a_bend_off_ninety_is_not_held_to_a_limit() {
        let bend = BendFact { angle_deg: 75.0, inside_radius_mm: 3.0, ..BendFact::default() };
        assert_eq!(shut_bends(&[bend]), BTreeSet::from([1]));
        assert!(off_90(&[BendFact { angle_deg: 90.0, ..BendFact::default() }], &BTreeSet::new(), &[]).is_none());
    }
}

/// Message parity with INTdfm. The oracle is the pre-merge finding list `rules.py` produced, so a
/// merged `not_verified` cause still has its own rule's message to compare. Findings whose message
/// INTdfm delegates to the thickness gate are the one exception: that gate's text lives in
/// `lib.rs`, outside this batch, so the test checks the delegation and marks them not compared.
#[cfg(test)]
mod message_parity {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde::Deserialize;

    use super::*;
    use crate::sink_rules;
    use crate::{CheckInput, Finding, RuleStatus, catalog_findings};

    const IN_SCOPE: &[&str] = &[
        BEND_GATE,
        INSIDE_RADIUS,
        MIN_FLANGE,
        MAX_BEND_LENGTH,
        BEND_RELIEF,
        MIN_WEB,
        U_CHANNEL,
        BEND_RADII,
        sink_rules::HOLE_SPACING,
        sink_rules::TOLERANCE_ACROSS_BENDS,
        sink_rules::COUNTERSINK_DEPTH,
        sink_rules::COUNTERSINK_TO_EDGE,
        sink_rules::COUNTERSINK_TO_BEND,
    ];

    #[derive(Deserialize)]
    struct PyQuantity {
        value: f64,
    }

    #[derive(Deserialize)]
    struct PyFinding {
        rules: Vec<String>,
        status: RuleStatus,
        error_code: Option<String>,
        reason_code: Option<String>,
        #[serde(default)]
        measured: Option<PyQuantity>,
        #[serde(default)]
        message: String,
        #[serde(default)]
        needs: Vec<String>,
        #[serde(default)]
        #[allow(dead_code)]
        feature: Option<serde_json::Value>,
        #[serde(default)]
        #[allow(dead_code)]
        location: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    struct CatalogFixture {
        input: CheckInput,
        #[serde(rename = "inputGate")]
        input_gate: PyFinding,
    }

    #[derive(Deserialize)]
    struct Oracle {
        cases: BTreeMap<String, Vec<PyFinding>>,
    }

    #[derive(Deserialize)]
    struct SinkFixture {
        case: SinkCase,
        input: SinkInput,
        expected: SinkExpected,
    }

    #[derive(Deserialize)]
    struct SinkCase {
        id: String,
        group: String,
        flat: bool,
    }

    #[derive(Deserialize)]
    struct SinkInput {
        declared: crate::Declared,
        generic: crate::Profile,
        shop: Option<crate::Profile>,
        facts: Facts,
        #[serde(default)]
        critical: Vec<crate::Critical>,
        thickness_gate: PyFinding,
        bends_gate: PyFinding,
    }

    #[derive(Deserialize)]
    struct SinkExpected {
        findings: Vec<PyFinding>,
    }

    fn fixtures(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(relative)
    }

    fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
        let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
    }

    fn gate(finding: &PyFinding) -> Finding {
        Finding {
            id: String::new(),
            rules: finding.rules.clone(),
            status: finding.status,
            state: AppState::Unsupported,
            error_code: finding.error_code.clone(),
            reason_code: finding.reason_code.clone(),
            message: finding.message.clone(),
            feature: Feature { kind: "part".into(), number: None, description: "whole part".into() },
            location: None,
            measured_mm: finding.measured.as_ref().map(|quantity| quantity.value),
            requirement: None,
            needs: finding.needs.clone(),
        }
    }

    fn in_scope(finding: &PyFinding) -> bool {
        finding.rules.iter().any(|rule| IN_SCOPE.contains(&rule.as_str()))
    }

    fn gate_delegated(finding: &PyFinding) -> bool {
        matches!(finding.reason_code.as_deref(), Some("THICKNESS_NOT_DECLARED" | "THICKNESS_NOT_MEASURED" | "THICKNESS_MISMATCH"))
    }

    fn same_measured(left: Option<f64>, right: Option<&PyQuantity>) -> bool {
        match (left, right) {
            (None, None) => true,
            (Some(left), Some(right)) => (left - right.value).abs() <= 1e-9,
            _ => false,
        }
    }

    /// Compare every in-scope Python finding with the Rust finding it names. Returns
    /// `(compared, exact, skipped, differences)`.
    fn compare(case: &str, python: &[PyFinding], rust: &[Finding], delegated_message: Option<&str>) -> (usize, usize, usize, Vec<String>) {
        let mut used = vec![false; rust.len()];
        let mut compared = 0;
        let mut exact = 0;
        let mut skipped = 0;
        let mut differences = Vec::new();
        for want in python.iter().filter(|finding| in_scope(finding)) {
            let position = rust.iter().enumerate().position(|(index, got)| {
                !used[index]
                    && got.rules == want.rules
                    && got.status == want.status
                    && got.error_code == want.error_code
                    && got.reason_code == want.reason_code
                    // A blocked finding's measured value is the thickness gate's, which is outside
                    // this batch; the rule contributes only the wording and the reason code.
                    && (gate_delegated(want) || same_measured(got.measured_mm, want.measured.as_ref()))
            });
            let Some(index) = position else {
                differences.push(format!("{case}: no Rust finding matches {:?} {:?}/{:?}", want.rules, want.error_code, want.reason_code));
                continue;
            };
            used[index] = true;
            let got = &rust[index];
            if gate_delegated(want) {
                skipped += 1;
                if let Some(message) = delegated_message
                    && got.message != message
                {
                    differences.push(format!("{case}: {:?} does not echo the thickness gate's message", want.rules));
                }
                continue;
            }
            compared += 1;
            if got.message == want.message {
                exact += 1;
            } else {
                differences.push(format!("{case}: {:?}\n  python: {}\n  rust:   {}", want.rules, want.message, got.message));
            }
        }
        (compared, exact, skipped, differences)
    }

    #[test]
    fn catalog_messages_match_python() {
        let dir = fixtures("catalog/v1");
        let oracle: Oracle = read(&dir.join("messages.json"));
        let mut cases = 0;
        let mut total_compared = 0;
        let mut total_exact = 0;
        let mut total_skipped = 0;
        let mut differences = Vec::new();
        for (case, python) in &oracle.cases {
            let fixture: CatalogFixture = read(&dir.join(format!("{case}.json")));
            let rust = catalog_findings(&fixture.input, &gate(&fixture.input_gate)).expect(case);
            let gate_message =
                rust.iter().find(|finding| finding.rules == [crate::THICKNESS_GATE.to_string()]).map(|finding| finding.message.clone());
            let (compared, exact, skipped, mut diffs) = compare(case, python, &rust, gate_message.as_deref());
            cases += 1;
            total_compared += compared;
            total_exact += exact;
            total_skipped += skipped;
            differences.append(&mut diffs);
        }
        eprintln!("catalog message parity: {cases} cases, {total_exact}/{total_compared} byte-exact, {total_skipped} gate-delegated (not compared)");
        assert_eq!(cases, 11, "the catalog corpus has 11 cases");
        assert!(total_compared > 0, "nothing was compared");
        assert!(differences.is_empty(), "{}", differences.join("\n"));
        assert_eq!(total_exact, total_compared);
    }

    #[test]
    fn sink_fixture_messages_match_python() {
        let dir = fixtures("sink");
        let mut total_compared = 0;
        let mut total_exact = 0;
        let mut total_skipped = 0;
        let mut cases = 0;
        let mut differences = Vec::new();
        for entry in fs::read_dir(&dir).expect("sink fixtures") {
            let path = entry.expect("a directory entry").path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let fixture: SinkFixture = read(&path);
            let input = CheckInput {
                declared: fixture.input.declared,
                generic: fixture.input.generic,
                shop: fixture.input.shop,
                overrides: Vec::new(),
                facts: fixture.input.facts,
                model: Default::default(),
                critical: fixture.input.critical,
            };
            let thickness = gate(&fixture.input.thickness_gate);
            let bends = gate(&fixture.input.bends_gate);
            let rust = match fixture.case.group.as_str() {
                "spacing" => sink_rules::hole_spacing(&input, &thickness),
                "tolerance" => sink_rules::tolerance_across_bends(&input, &thickness, &bends, fixture.case.flat),
                "countersinks" => sink_rules::countersink_findings(&input, &thickness, &bends, fixture.case.flat),
                other => panic!("{}: unknown group {other}", fixture.case.id),
            };
            let (compared, exact, skipped, mut diffs) = compare(&fixture.case.id, &fixture.expected.findings, &rust, Some(&thickness.message));
            cases += 1;
            total_compared += compared;
            total_exact += exact;
            total_skipped += skipped;
            differences.append(&mut diffs);
        }
        eprintln!("sink message parity: {cases} cases, {total_exact}/{total_compared} byte-exact, {total_skipped} gate-delegated (not compared)");
        assert_eq!(cases, 18, "the sink corpus has 18 cases");
        assert!(differences.is_empty(), "{}", differences.join("\n"));
        assert_eq!(total_exact, total_compared);
    }
}
