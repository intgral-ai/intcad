//! `sm.hole_spacing`, `sm.tolerance_across_bends` and the three countersink rules.
//!
//! This ports the INTdfm rule layer that reads holes against each other and against a declared
//! dimension, and the three rules that read a countersink from its rim:
//!
//! * `sm.hole_spacing` (`rules.py:458-518`): the wall between each round hole and its nearest one
//!   on the same flat, measured once per pair and reported at the lower-numbered hole. A limit may
//!   be written in `d`, the mean of the two diameters, which turns a shop's centre-to-centre
//!   number into this wall;
//! * `sm.tolerance_across_bends` (`rules.py:574-741`): each declared hole-to-hole dimension, the
//!   nearest hole within `MATCH_MM` plus half the sheet's depth, the fewest bends between the two
//!   holes' flats, and the shop's own table row for that many bends;
//! * `sm.countersink_depth`, `sm.countersink_to_edge` and `sm.countersink_to_bend`
//!   (`rules.py:808-1074`): a countersink from its rim, with no land, no edge or no bend flat
//!   reported rather than read as zero.
//!
//! # What is faithful, and what is not
//!
//! Faithful: every status, `error_code`, `reason_code`, measured value, chosen limit and rule list,
//! the order the findings are produced in, the merge-sharing reason codes (one `not_verified`
//! cause per wording), the thickness-gate block, the bend-gate "same cause" path, `d` in a limit,
//! and the loosest table row across bends.
//!
//! Messages follow INTdfm's wording and number spelling ([`crate::pyfmt`]); the message-parity
//! tests compare them byte for byte. `needs` follows INTdfm's per-code wording.
//!
//! Project overrides are **not** applied here: INTdfm's `_choices` has none, and FD-7's
//! confirmation semantics are still a proposal. `crate::choose` applies them for the three hole
//! rules, so a project that overrides one of these five numbers is not reflected yet.
//!
//! The fact layer is the reason a rule can be missing: `CountersinkFact` is only as good as what
//! the App measured, and a countersink the recogniser found off a round hole is reported as
//! unmeasured, never as a pass.

use crate::{
    AppState, CheckInput, Chosen, CountersinkFact, Facts, Feature, Finding, HoleFact, Layer, Limit, Location, Point3, Requirement, RequirementRef,
    RuleStatus, SAME_SHEET_MM, below, pyfmt, round6, state_of,
};

/// `sm.hole_spacing`.
pub const HOLE_SPACING: &str = "sm.hole_spacing";
/// `sm.tolerance_across_bends`.
pub const TOLERANCE_ACROSS_BENDS: &str = "sm.tolerance_across_bends";
/// `sm.countersink_depth`.
pub const COUNTERSINK_DEPTH: &str = "sm.countersink_depth";
/// `sm.countersink_to_edge`.
pub const COUNTERSINK_TO_EDGE: &str = "sm.countersink_to_edge";
/// `sm.countersink_to_bend`.
pub const COUNTERSINK_TO_BEND: &str = "sm.countersink_to_bend";

/// The three countersink rules, in INTdfm's `COUNTERSINK_RULES` order.
pub const COUNTERSINK_RULES: &[&str] = &[COUNTERSINK_DEPTH, COUNTERSINK_TO_EDGE, COUNTERSINK_TO_BEND];

/// `rules._UPPER_LIMITS`: a measure that must stay at or under its limit.
const UPPER_LIMITS: &[&str] = &["sm.max_bend_length", "sm.u_channel_ratio", COUNTERSINK_DEPTH];

/// `rules._MATCH_MM`: how near a declared point must be to a hole's centre.
const MATCH_MM: f64 = 1.0;

const SHOP_NUMBER: &str = "The shop's own number for this rule.";
const COUNT_BY_HAND: &str = "A person to count this part's bends, because this version can't be sure.";
const LOOK_AT_HOLE: &str = "A person to look at this hole.";
const LOOK_AT_SINK: &str = "A person to look at this countersink.";
const CHOOSE_A_FIX: &str = "A person to choose: move a hole, have them cut after bending, or loosen the tolerance.";
const CHOOSE_ON_ONE_FLAT: &str = "A person to choose: ask the shop for a tighter process, or loosen the tolerance.";
const NO_HOLE_STEP: &str = "The step that finds holes failed, so no hole was checked.";
const NO_PAIR: &str = "No two round holes this version can measure are on the same flat, so this rule has nothing to measure.";
const NO_FLAT_TO_A_BEND: &str = "A hole here isn't on a flat that ends at a bend, so its distance to a bend wasn't measured.";
const SINK_OFF_A_HOLE: &str = "A countersink here isn't on a round hole this version can read, so it wasn't measured.";
const FLAT_PART: &str = "All the skins are parallel and every other face runs straight between them, so this part is flat and has no bends.";

// ---------------------------------------------------------------------------
// sm.hole_spacing.

/// The wall between each round hole and its nearest one on the same flat, edge to edge.
pub fn hole_spacing(input: &CheckInput, thickness_gate: &Finding) -> Vec<Finding> {
    let rules: &[&str] = &[HOLE_SPACING];
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(rules, thickness_gate)];
    }
    let facts = &input.facts;
    let Some(paired) = facts.paired_mm else {
        return vec![unsupported_with(
            rules,
            "THICKNESS_NOT_MEASURED",
            "The model's sheet thickness was not measured, so no wall between holes was checked.",
            vec!["A model whose sheet thickness can be measured.".to_string()],
        )];
    };
    let Some(holes) = facts.holes.as_ref() else {
        return vec![unrecognised(rules, NO_HOLE_STEP)];
    };
    let (usable, why_not) = usable_holes_with_gap(facts, paired, holes);
    // An unread feature could be the nearer neighbour, so it's said as the hole rules say it.
    let mut results = Vec::new();
    if let Some(why_not) = why_not {
        results.push(unrecognised(rules, &why_not));
    }
    let placed: Vec<(usize, &HoleFact)> = usable.iter().copied().filter(|(_, hole)| hole.flat.is_some()).collect();
    let unplaced: Vec<(usize, &HoleFact)> = usable.iter().copied().filter(|(_, hole)| hole.flat.is_none()).collect();
    if !unplaced.is_empty() {
        results.push(flat_unknown(&unplaced));
    }
    // Each hole's nearest neighbour on its own flat; a pair is stored once, lower number first.
    let mut pairs: Vec<((usize, usize), f64)> = Vec::new();
    for (number, hole) in &placed {
        let mut walls: Vec<(f64, usize)> = Vec::new();
        for (other, near) in &placed {
            if other != number && near.flat == hole.flat {
                walls.push((wall(hole, near), *other));
            }
        }
        let nearest = walls.into_iter().min_by(|left, right| left.0.total_cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        if let Some((wall_mm, other)) = nearest {
            let pair = ((*number).min(other), (*number).max(other));
            if !pairs.iter().any(|(known, _)| *known == pair) {
                pairs.push((pair, wall_mm));
            }
        }
    }
    if pairs.is_empty() {
        return if results.is_empty() { vec![not_applicable_with(rules, NO_PAIR)] } else { results };
    }
    pairs.sort_by_key(|(pair, _)| *pair);
    let holes_by_number: Vec<(usize, &HoleFact)> = placed.clone();
    let sheet = sheet_thickness(input, facts);
    let mut met: Vec<(f64, f64, Chosen)> = Vec::new();
    for ((first, second), wall_mm) in pairs {
        let Some(hole_first) = holes_by_number.iter().find(|(number, _)| *number == first).map(|(_, hole)| *hole) else { continue };
        let Some(hole_second) = holes_by_number.iter().find(|(number, _)| *number == second).map(|(_, hole)| *hole) else { continue };
        let mean_diameter = (hole_first.diameter_mm + hole_second.diameter_mm) / 2.0;
        let list = match choices(input, HOLE_SPACING, sheet, None, None, Some(mean_diameter)) {
            Ok(list) => list,
            Err(finding) => {
                results.push(finding);
                return results;
            }
        };
        let Some(chosen) = strictest(list) else { continue };
        if below(wall_mm, chosen.chosen.limit_mm, chosen.upper) {
            results.push(spacing_review(first, hole_first, wall_mm, &chosen.chosen, second));
        } else {
            met.push((wall_mm - chosen.chosen.limit_mm, wall_mm, chosen.chosen));
        }
    }
    if let Some((_, value, chosen)) = met.iter().enumerate().min_by(|left, right| left.1.0.total_cmp(&right.1.0)).map(|(_, entry)| entry.clone()) {
        results.push(holes_met(
            HOLE_SPACING,
            value,
            met.len(),
            "wall",
            &chosen,
            "These holes are clear of the public value, so nothing is known about them: only a shop's own number can say a hole is fine.",
        ));
    }
    results
}

/// Centre to centre, less both radii.
fn wall(one: &HoleFact, other: &HoleFact) -> f64 {
    let centres = distance((one.x_mm, one.y_mm, one.z_mm), (other.x_mm, other.y_mm, other.z_mm));
    (centres - (one.diameter_mm + other.diameter_mm) / 2.0).max(0.0)
}

fn spacing_review(first: usize, hole: &HoleFact, value: f64, chosen: &Chosen, second: usize) -> Finding {
    let needs = if chosen.decides { vec![LOOK_AT_HOLE.to_string()] } else { vec![SHOP_NUMBER.to_string()] };
    located(
        &[HOLE_SPACING],
        RuleStatus::NeedsReview,
        Some("HOLE_SPACING_BELOW_LIMIT"),
        None,
        format!(
            "Hole {first}: its wall to hole {second} is {} mm, below {} of {} mm. A person decides.",
            pyfmt::g(value),
            whose(chosen),
            pyfmt::g(chosen.limit_mm)
        ),
        (hole.x_mm, hole.y_mm, hole.z_mm),
        Some(value),
        Some(chosen),
        needs,
    )
}

fn flat_unknown(holes: &[(usize, &HoleFact)]) -> Finding {
    let places: Vec<String> = holes
        .iter()
        .map(|(number, hole)| {
            format!(
                "hole {number}, at x {}, y {}, z {}",
                pyfmt::g(pyfmt::round(hole.x_mm, 3)),
                pyfmt::g(pyfmt::round(hole.y_mm, 3)),
                pyfmt::g(pyfmt::round(hole.z_mm, 3))
            )
        })
        .collect();
    let names: Vec<String> = holes.iter().map(|(number, _)| format!("hole {number}")).collect();
    unsupported_with(
        &[HOLE_SPACING],
        "HOLES_UNRECOGNISED",
        &format!("The flat can't be told for {}, so no wall to a nearest hole was measured: {}.", many(holes.len(), "round hole"), places.join("; ")),
        vec![format!("A person to measure the wall from {} to the nearest round hole.", names.join(", "))],
    )
}

// ---------------------------------------------------------------------------
// sm.tolerance_across_bends.

/// Whether the shop holds each declared hole-to-hole tolerance across the bends between them.
pub fn tolerance_across_bends(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, flat: bool) -> Vec<Finding> {
    let rules: &[&str] = &[TOLERANCE_ACROSS_BENDS];
    if input.critical.is_empty() {
        return vec![unsupported_with(
            rules,
            "CRITICAL_NOT_DECLARED",
            "No critical dimension was declared, so no tolerance was checked.",
            vec!["The hole-to-hole dimensions that matter, with their plus or minus, as [[critical]] in intdfm.toml or in the call.".to_string()],
        )];
    }
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(rules, thickness_gate)];
    }
    let facts = &input.facts;
    let Some(paired) = facts.paired_mm else {
        return vec![unsupported_with(
            rules,
            "THICKNESS_NOT_MEASURED",
            "The model's sheet thickness was not measured, so no tolerance was checked.",
            vec!["A model whose sheet thickness can be measured.".to_string()],
        )];
    };
    let Some(holes) = facts.holes.as_ref() else {
        return vec![unrecognised(rules, NO_HOLE_STEP)];
    };
    if !flat && bends_gate.status != RuleStatus::Pass {
        return vec![same_as_gate(bends_gate, rules)];
    }
    let usable: Vec<(usize, &HoleFact)> =
        holes.iter().enumerate().map(|(index, hole)| (index + 1, hole)).filter(|(_, hole)| usable_hole(hole, paired)).collect();
    let links: Vec<Vec<usize>> = facts
        .recognition
        .as_ref()
        .map(|found| found.bends.iter().map(|bend| bend.flats.iter().flatten().copied().collect()).collect())
        .unwrap_or_default();
    let sheet = sheet_thickness(input, facts);
    input.critical.iter().map(|dimension| one_dimension(dimension, &usable, &links, flat, sheet, input)).collect()
}

fn one_dimension(
    dimension: &crate::Critical,
    holes: &[(usize, &HoleFact)],
    links: &[Vec<usize>],
    flat: bool,
    sheet: f64,
    input: &CheckInput,
) -> Finding {
    let rules: &[&str] = &[TOLERANCE_ACROSS_BENDS];
    let name = &dimension.name;
    let first = nearest_hole(&dimension.from_mm, holes);
    let second = nearest_hole(&dimension.to_mm, holes);
    let (Some((a, hole_a)), Some((b, hole_b))) = (first, second) else {
        let which = if first.is_none() && second.is_none() { "either point" } else { "one of its points" };
        return unsupported_with(
            rules,
            "CRITICAL_FEATURE_NOT_FOUND",
            &format!(
                "{name}: no round hole this version can measure is within {} mm of {which}, beyond half the sheet's thickness.",
                pyfmt::g(MATCH_MM)
            ),
            vec![format!(
                "For {name}, points within {} mm of two round holes' centres. A report's part.holes lists them; only holes square to the skins and through the sheet count.",
                pyfmt::g(MATCH_MM)
            )],
        );
    };
    if a == b {
        return unsupported_with(
            rules,
            "CRITICAL_SAME_FEATURE",
            &format!("{name}: both points are at hole {a}, so there's no dimension to check."),
            vec![format!("For {name}, two points at two different round holes.")],
        );
    }
    let crossed = if flat { Some(0) } else { bends_between(hole_a.flat, hole_b.flat, links) };
    let Some(crossed) = crossed else {
        return unsupported_with(
            rules,
            "BEND_UNRECOGNISED",
            &format!("{name}: which bends lie between hole {a} and hole {b} can't be told from this part's flats."),
            vec![format!("A person to count the bends between hole {a} and hole {b}.")],
        );
    };
    let list = match choices(input, TOLERANCE_ACROSS_BENDS, sheet, None, None, None) {
        Ok(list) => list,
        Err(finding) => return finding,
    };
    let rows: Vec<Choice> = list.into_iter().filter(|choice| choice.covers(crossed)).collect();
    // Python's `max` keeps the first of two equal limits; `reduce` with `>` does the same.
    let Some(row) = rows.into_iter().reduce(|best, item| if item.chosen.limit_mm > best.chosen.limit_mm { item } else { best }) else {
        return no_number(TOLERANCE_ACROSS_BENDS);
    };
    let wanted = dimension.tolerance_mm;
    let both = format!("hole {a} to hole {b}");
    let middle = Point3 { x: (hole_a.x_mm + hole_b.x_mm) / 2.0, y: (hole_a.y_mm + hole_b.y_mm) / 2.0, z: (hole_a.z_mm + hole_b.z_mm) / 2.0 };
    let location = Some(Location { x_mm: middle.x, y_mm: middle.y, z_mm: middle.z });
    let feature = Feature { kind: "dimension".to_string(), number: None, description: format!("{name}: {both}") };
    let crosses = many(crossed, "bend");
    if below(wanted, row.chosen.limit_mm, row.upper) {
        let fixes = if crossed > 0 {
            format!("Put both holes on one flat, have them cut after bending, or allow plus or minus {} mm.", pyfmt::g(row.chosen.limit_mm))
        } else {
            format!("Ask the shop for a tighter process, or allow plus or minus {} mm.", pyfmt::g(row.chosen.limit_mm))
        };
        let needs = if row.chosen.decides {
            vec![if crossed > 0 { CHOOSE_A_FIX } else { CHOOSE_ON_ONE_FLAT }.to_string()]
        } else {
            vec![format!("The shop's own table for {name}.")]
        };
        return make(
            rules,
            RuleStatus::NeedsReview,
            Some("TOLERANCE_TIGHTER_THAN_SHOP"),
            None,
            format!(
                "{name}: {both} crosses {crosses}. {} plus or minus {} mm there, and this needs plus or minus {} mm. {fixes}",
                if row.chosen.decides { "This shop's table gives" } else { "The public value is" },
                pyfmt::g(row.chosen.limit_mm),
                pyfmt::g(wanted)
            ),
            feature,
            location,
            Some(wanted),
            Some(row.chosen.clone()),
            needs,
            row.chosen.decides,
        );
    }
    if !row.chosen.decides {
        return make(
            rules,
            RuleStatus::NotVerified,
            None,
            Some("ABOVE_PUBLIC_VALUE"),
            format!(
                "{name}: {both} crosses {crosses}. Plus or minus {} mm is no tighter than the public value of {} mm, so nothing is known: only a shop's own table can say it holds.",
                pyfmt::g(wanted),
                pyfmt::g(row.chosen.limit_mm)
            ),
            feature,
            location,
            Some(wanted),
            Some(row.chosen.clone()),
            vec![format!("The shop's own table for {name}.")],
            false,
        );
    }
    make(
        rules,
        RuleStatus::Pass,
        None,
        None,
        format!(
            "{name}: {both} crosses {crosses}. This shop's table gives plus or minus {} mm there, within the plus or minus {} mm needed. This says nothing about anything but this dimension.",
            pyfmt::g(row.chosen.limit_mm),
            pyfmt::g(wanted)
        ),
        feature,
        location,
        Some(wanted),
        Some(row.chosen.clone()),
        Vec::new(),
        true,
    )
}

/// The round hole whose centre is nearest to `point`, if the point is within reach of it. A centre
/// is at mid-depth and a designer may read one off a skin, so half the depth is allowed.
fn nearest_hole<'a>(point: &Point3, holes: &[(usize, &'a HoleFact)]) -> Option<(usize, &'a HoleFact)> {
    let mut best: Option<(f64, usize, &HoleFact)> = None;
    for (number, hole) in holes {
        let away = distance((point.x, point.y, point.z), (hole.x_mm, hole.y_mm, hole.z_mm));
        if away <= MATCH_MM + hole.depth_mm / 2.0 && best.as_ref().is_none_or(|known| away < known.0) {
            best = Some((away, *number, hole));
        }
    }
    best.map(|(_, number, hole)| (number, hole))
}

/// The fewest bends between two flats, or `None` when either flat or a path is unknown.
fn bends_between(start: Option<usize>, end: Option<usize>, links: &[Vec<usize>]) -> Option<usize> {
    let (Some(start), Some(end)) = (start, end) else {
        return None;
    };
    let mut seen: Vec<usize> = vec![start];
    let mut frontier: Vec<usize> = vec![start];
    for crossed in 0..=links.len() {
        if seen.contains(&end) {
            return Some(crossed);
        }
        let mut next: Vec<usize> = Vec::new();
        for flat in &frontier {
            for joined in links {
                if joined.contains(flat) {
                    for other in joined {
                        if !seen.contains(other) && !next.contains(other) {
                            next.push(*other);
                        }
                    }
                }
            }
        }
        if next.is_empty() {
            return None;
        }
        seen.extend(next.iter().copied());
        frontier = next;
    }
    None
}

// ---------------------------------------------------------------------------
// The three countersink rules.

/// A countersink and the round hole it opens out.
struct Sink<'a> {
    hole: usize,
    facts: &'a CountersinkFact,
}

impl Sink<'_> {
    fn place(&self) -> (f64, f64, f64) {
        (self.facts.x_mm, self.facts.y_mm, self.facts.z_mm)
    }
}

/// The three countersink rules, each from the rim. A countersink with no land is reviewed on its
/// own; a flat part's countersink-to-bend says there is no bend to measure.
pub fn countersink_findings(input: &CheckInput, thickness_gate: &Finding, bends_gate: &Finding, flat: bool) -> Vec<Finding> {
    if thickness_gate.status != RuleStatus::Pass {
        return vec![blocked_for(COUNTERSINK_RULES, thickness_gate)];
    }
    let facts = &input.facts;
    let Some(holes) = facts.holes.as_ref() else {
        return vec![unrecognised(COUNTERSINK_RULES, NO_HOLE_STEP)];
    };
    let Some(found) = facts.recognition.as_ref() else {
        return vec![same_as_gate(bends_gate, COUNTERSINK_RULES)];
    };
    let sinks: Vec<Sink<'_>> =
        holes.iter().enumerate().filter_map(|(index, hole)| hole.countersink.as_ref().map(|facts| Sink { hole: index + 1, facts })).collect();
    // A countersink the recogniser found that no hole carries: its wall isn't a round hole.
    let off_a_hole = found.countersinks > sinks.len();
    if sinks.is_empty() {
        if found.cones > 0 {
            return vec![cones_left(COUNTERSINK_RULES)];
        }
        if found.unknown_faces > 0 {
            return vec![same_as_gate(bends_gate, COUNTERSINK_RULES)];
        }
        if off_a_hole {
            return vec![unrecognised(COUNTERSINK_RULES, SINK_OFF_A_HOLE)];
        }
        return vec![not_applicable_with(COUNTERSINK_RULES, "This part has no countersink, so these rules have nothing to measure.")];
    }
    let sheet = sheet_thickness(input, facts);
    // Beside what was measured, what might hide another countersink.
    let mut results = Vec::new();
    if found.cones > 0 {
        results.push(cones_left(&COUNTERSINK_RULES[..2]));
    }
    if found.unknown_faces > 0 {
        results.push(same_as_gate(bends_gate, &COUNTERSINK_RULES[..2]));
    }
    if off_a_hole {
        results.push(unrecognised(if flat { &COUNTERSINK_RULES[..2] } else { COUNTERSINK_RULES }, SINK_OFF_A_HOLE));
    }
    results.extend(sink_depths(input, sheet, &sinks));
    results.extend(sink_edges(input, sheet, &sinks));
    if flat {
        results.push(not_applicable_with(&[COUNTERSINK_TO_BEND], FLAT_PART));
    } else {
        results.extend(sinks_to_bend(input, sheet, &sinks, bends_gate, found));
    }
    results
}

// The `Err` is the finding the rule hands back, wording and needs included; boxing it would only
// move the allocation, so the size is allowed here.
#[allow(clippy::result_large_err)]
fn ask(input: &CheckInput, rule: &str, sheet: f64, sink: Option<&Sink<'_>>) -> Result<Chosen, Finding> {
    let diameter = sink.map(|sink| sink.facts.major_diameter_mm);
    let list = choices(input, rule, sheet, None, None, diameter)?;
    strictest(list).map(|choice| choice.chosen).ok_or_else(|| no_number(rule))
}

fn cones_left(rules: &[&str]) -> Finding {
    unsupported_with(
        rules,
        "FORMS_UNSUPPORTED",
        "A cone on this part isn't a countersink this version can read, so a countersink can't be ruled out.",
        vec!["A person to look for countersinks among those cones.".to_string()],
    )
}

/// With no land left, one review that says so, and no depth: the depth is the sheet's.
fn sink_depths(input: &CheckInput, sheet: f64, sinks: &[Sink<'_>]) -> Vec<Finding> {
    let mut results: Vec<Finding> = sinks
        .iter()
        .filter(|sink| round4(sink.facts.land_mm) <= 0.0)
        .map(|sink| {
            make(
                &[COUNTERSINK_DEPTH],
                RuleStatus::NeedsReview,
                Some("COUNTERSINK_NO_LAND"),
                None,
                format!(
                    "The countersink of hole {}: its cone runs right through the sheet, so no straight wall is left. A person decides.",
                    sink.hole
                ),
                sink_feature(sink),
                Some(sink_location(sink)),
                Some(sink.facts.land_mm),
                None,
                vec![LOOK_AT_SINK.to_string()],
                false,
            )
        })
        .collect();
    let landed: Vec<&Sink<'_>> = sinks.iter().filter(|sink| round4(sink.facts.land_mm) > 0.0).collect();
    if landed.is_empty() {
        return results;
    }
    let chosen = match ask(input, COUNTERSINK_DEPTH, sheet, None) {
        Ok(chosen) => chosen,
        Err(finding) => {
            results.push(finding);
            return results;
        }
    };
    let mut met: Vec<(f64, Chosen)> = Vec::new();
    for sink in landed {
        let value = sink.facts.depth_mm;
        if below(value, chosen.limit_mm, true) {
            results.push(sink_review(COUNTERSINK_DEPTH, "COUNTERSINK_DEPTH_ABOVE_LIMIT", sink, "depth", value, &chosen));
        } else {
            met.push((value, chosen.clone()));
        }
    }
    if !met.is_empty() {
        results.push(sinks_met(COUNTERSINK_DEPTH, &met, "the deepest"));
    }
    results
}

fn sink_edges(input: &CheckInput, sheet: f64, sinks: &[Sink<'_>]) -> Vec<Finding> {
    let mut results: Vec<Finding> = Vec::new();
    let mut met: Vec<(f64, Chosen)> = Vec::new();
    let mut unclear: Vec<&Sink<'_>> = Vec::new();
    for sink in sinks {
        let chosen = match ask(input, COUNTERSINK_TO_EDGE, sheet, Some(sink)) {
            Ok(chosen) => chosen,
            Err(finding) => return vec![finding],
        };
        match sink.facts.edge_mm {
            Some(value) if value.is_finite() => {
                if below(value, chosen.limit_mm, false) {
                    let what = format!("rim's distance to a cut edge, {},", rim_edge(sink.facts));
                    results.push(sink_review(COUNTERSINK_TO_EDGE, "COUNTERSINK_EDGE_BELOW_LIMIT", sink, &what, value, &chosen));
                } else {
                    met.push((value, chosen));
                }
            }
            _ => unclear.push(sink),
        }
    }
    if !unclear.is_empty() {
        let names: Vec<String> = unclear.iter().map(|sink| format!("the countersink of hole {}", sink.hole)).collect();
        results.push(unsupported_with(
            &[COUNTERSINK_TO_EDGE],
            "HOLE_EDGE_UNCLEAR",
            &format!("The nearest cut edge to the rim can't be told from this part's faces for {}.", names.join(", ")),
            vec![format!("A person to measure the distance from the rim to the nearest cut edge for {}.", names.join(", "))],
        ));
    }
    if !met.is_empty() {
        results.push(sinks_met(COUNTERSINK_TO_EDGE, &met, "the nearest"));
    }
    results
}

/// As `sm.hole_to_bend`: below the number is a review whatever the gate says, and clear of it says
/// something only when the gate found every bend.
fn sinks_to_bend(input: &CheckInput, sheet: f64, sinks: &[Sink<'_>], bends_gate: &Finding, found: &crate::Recognition) -> Vec<Finding> {
    if found.bends.is_empty() {
        return vec![same_as_gate(bends_gate, &[COUNTERSINK_TO_BEND])];
    }
    let mut results: Vec<Finding> = Vec::new();
    let mut met: Vec<(f64, Chosen)> = Vec::new();
    let mut unmeasured = false;
    for sink in sinks {
        let chosen = match ask(input, COUNTERSINK_TO_BEND, sheet, Some(sink)) {
            Ok(chosen) => chosen,
            Err(finding) => return vec![finding],
        };
        if sink.facts.bend_gaps.is_empty() {
            unmeasured = true;
            continue;
        }
        let nearest = sink.facts.bend_gaps.iter().min_by(|left, right| left.gap_mm.total_cmp(&right.gap_mm));
        let Some(nearest) = nearest else { continue };
        let value = nearest.gap_mm;
        if below(value, chosen.limit_mm, false) {
            let what = format!("rim's distance to bend {}", nearest.bend);
            results.push(sink_review(COUNTERSINK_TO_BEND, "COUNTERSINK_BEND_BELOW_LIMIT", sink, &what, value, &chosen));
        } else {
            met.push((value, chosen));
        }
    }
    if unmeasured && bends_gate.status == RuleStatus::Pass {
        results.push(unrecognised(&[COUNTERSINK_TO_BEND], NO_FLAT_TO_A_BEND));
    } else if unmeasured || (!met.is_empty() && bends_gate.status != RuleStatus::Pass) {
        results.push(same_as_gate(bends_gate, &[COUNTERSINK_TO_BEND]));
    } else if !met.is_empty() {
        results.push(sinks_met(COUNTERSINK_TO_BEND, &met, "the nearest"));
    }
    results
}

/// Which edge the rim's distance runs to, as far as the faces show it.
fn rim_edge(facts: &CountersinkFact) -> String {
    if let Some(hole) = facts.edge_hole {
        return if facts.edge_rim { format!("the rim of hole {hole}'s countersink") } else { format!("hole {hole}'s wall") };
    }
    match facts.edge_kind {
        Some(crate::EdgeKind::Outline) => "on this flat's outline".to_string(),
        _ => "on another opening in this flat".to_string(),
    }
}

fn sink_review(rule: &str, error_code: &str, sink: &Sink<'_>, what: &str, value: f64, chosen: &Chosen) -> Finding {
    let side = if UPPER_LIMITS.contains(&rule) { "above" } else { "below" };
    let needs = if chosen.decides { vec![LOOK_AT_SINK.to_string()] } else { vec![sink_need(rule).to_string()] };
    make(
        &[rule],
        RuleStatus::NeedsReview,
        Some(error_code),
        None,
        format!(
            "The countersink of hole {}: its {what} is {} mm, {side} {} of {} mm. A person decides.",
            sink.hole,
            pyfmt::g(value),
            whose(chosen),
            pyfmt::g(chosen.limit_mm)
        ),
        sink_feature(sink),
        Some(sink_location(sink)),
        Some(value),
        Some(chosen.clone()),
        needs,
        chosen.decides,
    )
}

/// One line for every countersink that met its number, at the one nearest to missing it.
fn sinks_met(rule: &str, met: &[(f64, Chosen)], tightest: &str) -> Finding {
    let Some((value, chosen)) =
        met.iter().min_by(|left, right| (left.0 - left.1.limit_mm).abs().total_cmp(&(right.0 - right.1.limit_mm).abs())).cloned()
    else {
        return no_number(rule);
    };
    if !chosen.decides {
        return make(
            &[rule],
            RuleStatus::NotVerified,
            None,
            Some("ABOVE_PUBLIC_VALUE"),
            "These countersinks are clear of the public value, so nothing is known about them: only a shop's own number can say a countersink is fine."
                .to_string(),
            part_feature(),
            None,
            Some(value),
            Some(chosen),
            vec![sink_need(rule).to_string()],
            false,
        );
    }
    make(
        &[rule],
        RuleStatus::Pass,
        None,
        None,
        format!(
            "{} met this shop's number, {tightest} at {} mm. This says nothing about anything but this rule.",
            many(met.len(), "countersink"),
            pyfmt::g(value)
        ),
        sink_feature_group(&format!("{} that met the number", many(met.len(), "countersink"))),
        None,
        Some(value),
        Some(chosen),
        Vec::new(),
        true,
    )
}

fn sink_need(rule: &str) -> &'static str {
    match rule {
        COUNTERSINK_DEPTH => "The shop's own number for a countersink's depth.",
        COUNTERSINK_TO_EDGE => "The shop's own number for a countersink's distance to a cut edge.",
        _ => "The shop's own number for a countersink's distance to a bend.",
    }
}

fn sink_feature(sink: &Sink<'_>) -> Feature {
    Feature {
        kind: "countersink".to_string(),
        number: Some(sink.hole),
        description: format!("countersink, {} mm across", pyfmt::g(sink.facts.major_diameter_mm)),
    }
}

fn sink_feature_group(description: &str) -> Feature {
    Feature { kind: "countersink".to_string(), number: None, description: description.to_string() }
}

fn sink_location(sink: &Sink<'_>) -> Location {
    let (x, y, z) = sink.place();
    Location { x_mm: x, y_mm: y, z_mm: z }
}

// ---------------------------------------------------------------------------
// Finding builders shared by the three rule groups.

#[allow(clippy::too_many_arguments)]
fn make(
    rules: &[&str],
    status: RuleStatus,
    error_code: Option<&str>,
    reason_code: Option<&str>,
    message: String,
    feature: Feature,
    location: Option<Location>,
    measured_mm: Option<f64>,
    requirement: Option<Chosen>,
    needs: Vec<String>,
    decides: bool,
) -> Finding {
    let code = error_code.or(reason_code).unwrap_or("result");
    // Reaching a public value is "checked": the rule ran and only a shop's own number could say
    // the part is fine.
    let state =
        if status == RuleStatus::NotVerified && reason_code == Some("ABOVE_PUBLIC_VALUE") { AppState::Checked } else { state_of(status, decides) };
    Finding {
        id: format!("{}/{code}", rules.first().copied().unwrap_or("rule")),
        rules: rules.iter().map(|rule| (*rule).to_string()).collect(),
        status,
        state,
        error_code: error_code.map(str::to_string),
        reason_code: reason_code.map(str::to_string),
        message,
        feature,
        location,
        measured_mm,
        requirement: requirement.map(|chosen| chosen.requirement),
        needs,
    }
}

#[allow(clippy::too_many_arguments)]
fn located(
    rules: &[&str],
    status: RuleStatus,
    error_code: Option<&str>,
    reason_code: Option<&str>,
    message: String,
    at: (f64, f64, f64),
    measured_mm: Option<f64>,
    requirement: Option<&Chosen>,
    needs: Vec<String>,
) -> Finding {
    make(
        rules,
        status,
        error_code,
        reason_code,
        message,
        Feature { kind: "hole".to_string(), number: None, description: format!("round hole at {:.3}, {:.3}, {:.3}", at.0, at.1, at.2) },
        Some(Location { x_mm: crate::round3(at.0), y_mm: crate::round3(at.1), z_mm: crate::round3(at.2) }),
        measured_mm,
        requirement.cloned(),
        needs,
        requirement.is_some_and(|chosen| chosen.decides),
    )
}

fn unsupported_with(rules: &[&str], code: &str, message: &str, needs: Vec<String>) -> Finding {
    make(rules, RuleStatus::NotVerified, None, Some(code), message.to_string(), part_feature(), None, None, None, needs, false)
}

fn not_applicable_with(rules: &[&str], message: &str) -> Finding {
    make(rules, RuleStatus::NotApplicable, None, None, message.to_string(), part_feature(), None, None, None, Vec::new(), false)
}

fn blocked_for(rules: &[&str], gate: &Finding) -> Finding {
    let violation = gate.status == RuleStatus::Violation;
    let code =
        if violation { "THICKNESS_MISMATCH".to_string() } else { gate.reason_code.clone().unwrap_or_else(|| "THICKNESS_NOT_DECLARED".to_string()) };
    let message = if violation {
        "The thickness gate found a mismatch, so the rules that depend on thickness did not run.".to_string()
    } else {
        gate.message.clone()
    };
    make(
        rules,
        RuleStatus::NotVerified,
        None,
        Some(&code),
        message,
        part_feature(),
        None,
        if violation { None } else { gate.measured_mm },
        None,
        gate.needs.clone(),
        false,
    )
}

fn same_as_gate(bends_gate: &Finding, rules: &[&str]) -> Finding {
    let code = bends_gate.reason_code.clone().unwrap_or_else(|| "BEND_UNRECOGNISED".to_string());
    let needs = if bends_gate.needs.is_empty() { vec![COUNT_BY_HAND.to_string()] } else { bends_gate.needs.clone() };
    make(
        rules,
        RuleStatus::NotVerified,
        None,
        Some(&code),
        bends_gate.message.clone(),
        part_feature(),
        None,
        bends_gate.measured_mm,
        None,
        needs,
        false,
    )
}

fn no_number(rule: &str) -> Finding {
    unsupported_with(
        &[rule],
        "NO_SHOP_NUMBER",
        "These rules need a shop's own numbers, and there are none for them here, so nothing is known from them.",
        vec![SHOP_NUMBER.to_string()],
    )
}

fn coverage_not_in_profile(rule: &str) -> Finding {
    unsupported_with(
        &[rule],
        "COVERAGE_NOT_IN_PROFILE",
        "The selected data has this rule, but no row applies at the declared process, material and thickness.",
        vec![SHOP_NUMBER.to_string()],
    )
}

fn requirement_input_unknown(rule: &str, upper: bool) -> Finding {
    let words = if upper { "an upper" } else { "a lower" };
    unsupported_with(
        &[rule],
        "REQUIREMENT_INPUT_UNKNOWN",
        "The selected data states this rule in terms this version can't work out.",
        vec![format!("The requirement as {words} limit in millimetres, or in terms of t in mm.")],
    )
}

fn part_feature() -> Feature {
    Feature { kind: "part".to_string(), number: None, description: "whole part".to_string() }
}

fn unrecognised(rules: &[&str], message: &str) -> Finding {
    unsupported_with(
        rules,
        "HOLES_UNRECOGNISED",
        message,
        vec!["A person to look at those faces, because this version can't check them.".to_string()],
    )
}

fn holes_met(rule: &str, value: f64, count: usize, noun: &str, chosen: &Chosen, public_message: &str) -> Finding {
    if !chosen.decides {
        return make(
            &[rule],
            RuleStatus::NotVerified,
            None,
            Some("ABOVE_PUBLIC_VALUE"),
            public_message.to_string(),
            hole_feature(&format!("{} that met the number", many(count, noun))),
            None,
            Some(value),
            Some(chosen.clone()),
            vec![SHOP_NUMBER.to_string()],
            false,
        );
    }
    make(
        &[rule],
        RuleStatus::Pass,
        None,
        None,
        format!("{} met this shop's number, the tightest at {value} mm. This says nothing about anything but this rule.", many(count, noun)),
        hole_feature(&format!("{} that met the number", many(count, noun))),
        None,
        Some(value),
        Some(chosen.clone()),
        Vec::new(),
        true,
    )
}

fn hole_feature(description: &str) -> Feature {
    Feature { kind: "hole".to_string(), number: None, description: description.to_string() }
}

fn whose(chosen: &Chosen) -> &'static str {
    if chosen.decides { "this shop's number" } else { "a public value" }
}

// ---------------------------------------------------------------------------
// Number selection.

/// One applicable row and its number, with the bend range a tolerance row covers.
struct Choice {
    chosen: Chosen,
    upper: bool,
    bends: Option<Vec<Option<i64>>>,
}

impl Choice {
    /// `sm.tolerance_across_bends`: whether this row covers this many bends crossed.
    fn covers(&self, crossed: usize) -> bool {
        let Some(bends) = self.bends.as_ref() else { return false };
        let Some(low) = bends.first().copied().flatten() else { return false };
        if low < 0 || low as usize > crossed {
            return false;
        }
        match bends.get(1).copied().flatten() {
            None => true,
            Some(high) => high >= 0 && crossed as i64 <= high,
        }
    }
}

/// Every number that applies, or the finding that says why there is none. `r` is the inside radius
/// of the bend being measured against, `d` a countersink's major diameter or the mean of two holes'
/// diameters.
#[allow(clippy::result_large_err)]
fn choices(input: &CheckInput, rule: &str, sheet: f64, r: Option<f64>, b: Option<f64>, d: Option<f64>) -> Result<Vec<Choice>, Finding> {
    let Some(thickness) = input.declared.thickness_mm else {
        return Err(unsupported_with(
            &[rule],
            "COVERAGE_NOT_DECLARED",
            "The profile can't be looked up without the process, the material with its temper and the thickness.",
            vec!["The declared process, material with temper and thickness.".to_string()],
        ));
    };
    let shop_has = input.shop.as_ref().is_some_and(|shop| shop.requirements.iter().any(|row| row.rule == rule));
    let (rows, layer) = match (&input.shop, shop_has) {
        (Some(shop), true) => (&shop.requirements, shop.layer),
        _ => (&input.generic.requirements, input.generic.layer),
    };
    if shop_has && (input.declared.process.is_none() || input.declared.material.is_none()) {
        return Err(unsupported_with(
            &[rule],
            "COVERAGE_NOT_DECLARED",
            "The selected data can't be looked up without the declared process and material with temper.",
            vec!["The declared process and material with temper.".to_string()],
        ));
    }
    let own: Vec<&Requirement> = rows.iter().filter(|row| row.rule == rule).collect();
    if own.is_empty() {
        return Err(no_number(rule));
    }
    let applying: Vec<&Requirement> = own.into_iter().filter(|row| crate::applies(row, &input.declared)).collect();
    if applying.is_empty() {
        return Err(coverage_not_in_profile(rule));
    }
    let upper = UPPER_LIMITS.contains(&rule);
    let mut chosen: Vec<Choice> = Vec::new();
    for row in applying {
        let Some(declared_limit) = limit_at(row, thickness, r, b, d, upper) else {
            return Err(requirement_input_unknown(rule, upper));
        };
        let Some(model_limit) = limit_at(row, sheet, r, b, d, upper) else {
            return Err(requirement_input_unknown(rule, upper));
        };
        let limit = if upper { declared_limit.min(model_limit) } else { declared_limit.max(model_limit) };
        let model_used = limit != declared_limit;
        let decides = !never_decides(layer);
        let text = if model_used {
            format!(
                "At {} {limit} mm. {} Its t is the model's {sheet} mm, {} than the declared {thickness}.",
                if upper { "most" } else { "least" },
                row.measuring_line,
                if sheet > thickness { "thicker" } else { "thinner" }
            )
        } else {
            format!("At {} {limit} mm. {}", if upper { "most" } else { "least" }, row.measuring_line)
        };
        chosen.push(Choice {
            chosen: Chosen {
                requirement: RequirementRef {
                    value_mm: limit,
                    layer,
                    confirmation: row.confirmation,
                    source: row.source.clone(),
                    text,
                    overridden: false,
                },
                limit_mm: limit,
                decides,
            },
            upper,
            bends: row.bends.clone(),
        });
    }
    Ok(chosen)
}

/// A limit in mm running this way, or `None` for anything else.
fn limit_at(row: &Requirement, t: f64, r: Option<f64>, b: Option<f64>, d: Option<f64>, upper: bool) -> Option<f64> {
    if row.limit != if upper { Limit::Max } else { Limit::Min } {
        return None;
    }
    if row.inputs.values().any(|unit| unit != "mm") {
        return None;
    }
    let mut total = 0.0;
    for (name, number) in &row.value {
        let value = match name.as_str() {
            "t" => t,
            "mm" => 1.0,
            "r" => r?,
            "b" => b?,
            "d" => d?,
            _ => return None,
        };
        total += number * value;
    }
    Some(total)
}

/// When ranges overlap, the strictest number wins and nothing says they overlapped.
fn strictest(mut list: Vec<Choice>) -> Option<Choice> {
    let upper = list.first()?.upper;
    list.drain(..).reduce(|best, item| {
        let take = if upper { item.chosen.limit_mm < best.chosen.limit_mm } else { item.chosen.limit_mm > best.chosen.limit_mm };
        if take { item } else { best }
    })
}

/// The model's own t, thicker or thinner than declared, so a limit can be worked out at both.
fn sheet_thickness(input: &CheckInput, facts: &Facts) -> f64 {
    let Some(declared) = input.declared.thickness_mm else {
        return facts.paired_mm.unwrap_or(0.0);
    };
    let measured = facts.paired_mm.unwrap_or(declared);
    if round4(measured - declared) != 0.0 { measured } else { declared }
}

// ---------------------------------------------------------------------------
// Small helpers.

fn never_decides(layer: Layer) -> bool {
    matches!(layer, Layer::Generic | Layer::Development)
}

fn usable_holes_with_gap<'a>(facts: &Facts, paired: f64, holes: &'a [HoleFact]) -> (Vec<(usize, &'a HoleFact)>, Option<String>) {
    let numbered: Vec<(usize, &HoleFact)> = holes.iter().enumerate().map(|(index, hole)| (index + 1, hole)).collect();
    let usable: Vec<(usize, &HoleFact)> = numbered.iter().copied().filter(|(_, hole)| usable_hole(hole, paired)).collect();
    let unusable = numbered.len() - usable.len();
    let faces = match facts.recognition.as_ref() {
        None => facts.unknown_faces,
        Some(found) => found.unknown_faces + found.cone_faces + found.form_faces,
    };
    if unusable == 0 && facts.other_openings == 0 && faces == 0 {
        return (usable, None);
    }
    let why = format!(
        "{} not square to the skins or not through the sheet, {} in the skins, and {} no rule here can read, so those can't be checked.",
        many(unusable, "round hole"),
        many(facts.other_openings, "other opening"),
        many(faces, "face")
    );
    (usable, Some(why))
}

fn usable_hole(hole: &HoleFact, paired: f64) -> bool {
    let numbers = [hole.diameter_mm, hole.x_mm, hole.y_mm, hole.z_mm, hole.depth_mm];
    numbers.iter().all(|value| value.is_finite()) && hole.square && round6(hole.depth_mm - paired).abs() <= SAME_SHEET_MM
}

fn distance(one: (f64, f64, f64), other: (f64, f64, f64)) -> f64 {
    let (dx, dy, dz) = (one.0 - other.0, one.1 - other.1, one.2 - other.2);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn many(count: usize, noun: &str) -> String {
    if count == 1 { format!("{count} {noun}") } else { format!("{count} {noun}s") }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hole(number: usize, x: f64, diameter: f64, flat: Option<usize>) -> (usize, HoleFact) {
        (number, HoleFact { diameter_mm: diameter, x_mm: x, y_mm: 0.0, z_mm: 0.0, depth_mm: 1.5, square: true, flat, ..HoleFact::default() })
    }

    #[test]
    fn a_wall_is_centre_distance_less_both_radii() {
        let (_, one) = hole(1, 0.0, 3.0, Some(1));
        let (_, other) = hole(2, 10.0, 0.5, Some(1));
        assert_eq!(wall(&one, &other), 8.25);
    }

    #[test]
    fn a_bend_range_covers_both_ends() {
        let chosen = Chosen {
            requirement: RequirementRef {
                value_mm: 0.2,
                layer: Layer::Factory,
                confirmation: None,
                source: String::new(),
                text: String::new(),
                overridden: false,
            },
            limit_mm: 0.2,
            decides: true,
        };
        let choice = Choice { chosen, upper: false, bends: Some(vec![Some(1), Some(2)]) };
        assert!(choice.covers(1));
        assert!(choice.covers(2));
        assert!(!choice.covers(0));
        assert!(!choice.covers(3));
        let unbounded = Choice { bends: Some(vec![Some(2), None]), ..choice };
        assert!(unbounded.covers(9));
        assert!(!unbounded.covers(1));
        assert!(!Choice { bends: None, ..unbounded }.covers(2));
    }

    #[test]
    fn bends_between_counts_the_shortest_path() {
        let links = vec![vec![1, 7], vec![1, 8]];
        assert_eq!(bends_between(Some(7), Some(1), &links), Some(1));
        assert_eq!(bends_between(Some(7), Some(8), &links), Some(2));
        assert_eq!(bends_between(Some(99), Some(1), &links), None);
        assert_eq!(bends_between(None, Some(1), &links), None);
    }
}
