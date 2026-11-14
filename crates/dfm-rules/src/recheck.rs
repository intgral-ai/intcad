//! `recheck.py`: what became of each finding since an earlier report, and the removal guard.
//!
//! This is the pure comparison half of the INTdfm handover: two INTdfm-shaped reports in, the
//! `recheck` object the later report would carry out. It loads no file, starts no process and
//! reads no clock; [`compare`] is a function of the two reports alone.
//!
//! # What is faithful
//!
//! * `readable`: report 1.6 or later, because that is the first with `part`;
//! * hole and bend pairing by place, then by a moved feature when exactly one candidate fits
//!   (`recheck.py:103-162`);
//! * feature mapping from a finding's own location text (`recheck.py:165-176`);
//! * every mark: `fixed`, `still_there`, `new`, `cant_match`, including the order they are made
//!   in and which later finding a mark consumes (`recheck.py:279-432`);
//! * the guard: dropped hole/opening/bend counts, size and volume changes, changed declared
//!   values, a changed shop file and a changed tool (`recheck.py:435-539`), and `not_compared`.
//!
//! The messages are INTdfm's, and the numbers inside them use the same `%g` spelling, so the
//! fixtures compare the whole `Recheck` byte for byte through the report writer. Distances are
//! `sqrt` of the summed squares, where CPython's `math.dist` uses a scaled `fsum`; a last-bit
//! difference can only matter exactly at the 1e-6 rounding used before a comparison, and no
//! fixture reaches such a boundary.
//!
//! Rust's types make `_refuse_odd_values` (`recheck.py:82-91`) unnecessary: a label, feature or
//! place cannot be a number, and a report that does not fit [`Report`] is refused when it is read.
//! A damaged report that still fits the types is refused where Python would raise:
//! [`compare`] returns an error for a bend with no place and for a size with a different number
//! of axes.

use std::collections::{BTreeMap, BTreeSet};

use crate::intdfm_report::{Finding, Guard, GuardFlag, Mark, MarkCounts, MarkWord, Report, Status, python_float_text, python_round};
use crate::{DfmError, Layer};

/// A re-export's own noise, from `recheck.py`.
pub const SAME_PLACE_MM: f64 = 0.05;
/// How far a feature may have moved and still be followed, in sheets of thickness.
pub const MOVED_REACH_SHEETS: f64 = 5.0;
/// How equal two holes' diameters, or two bends' radii or lengths, must be to be the same one.
pub const SAME_SIZE_MM: f64 = 0.05;
/// How equal two bends' angles must be.
pub const SAME_ANGLE_DEG: f64 = 1.0;
/// A change larger than this share of the earlier value is raised by the guard.
pub const LARGE_CHANGE_SHARE: f64 = 0.20;

const FIRST_VERSION: u8 = 6;
const CLEAR_OF_PUBLIC: &str = "ABOVE_PUBLIC_VALUE";
const BEND_GATE: &str = "sm.gate_bends_found";
const RELIEF_RULE: &str = "sm.bend_relief_present";
const RADII_RULE: &str = "sm.bend_radius_consistent";
const PERSON_DECIDES: &str = "A person decides whether the part's purpose changed.";

/// A report this version can compare with: 1.6 or later, because older ones list no holes.
pub fn readable(report: &Report) -> bool {
    version_is_readable(&report.report_version)
}

fn version_is_readable(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("1.") else { return false };
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    // Compare the number without parsing it: a very long digit string is simply "10 or more".
    let trimmed = rest.trim_start_matches('0');
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.len() > 1 {
        return true;
    }
    trimmed.as_bytes()[0] >= b'0' + FIRST_VERSION
}

/// What became of each finding since an earlier report.
pub fn compare(earlier: &Report, later: &Report) -> Result<crate::intdfm_report::Recheck, DfmError> {
    let guard = guard(earlier, later)?;
    let unsure = !guard.flags.is_empty() || !guard.not_compared.is_empty();
    let marks = marks(earlier, later, unsure)?;
    let mut counts = MarkCounts::default();
    for mark in &marks {
        match mark.mark {
            MarkWord::Fixed => counts.fixed += 1,
            MarkWord::StillThere => counts.still_there += 1,
            MarkWord::New => counts.new += 1,
            MarkWord::CantMatch => counts.cant_match += 1,
        }
    }
    Ok(crate::intdfm_report::Recheck {
        earlier_snapshot_id: earlier.snapshot_id.clone(),
        earlier_file: earlier.traceability.declared.file.clone(),
        counts,
        marks,
        guard,
    })
}

// ---------------------------------------------------------------------------
// Spots and pairing.

/// One hole or bend the pairing can follow.
#[derive(Debug, Clone)]
struct Spot {
    number: usize,
    at: [f64; 3],
    /// A hole's diameter; a bend's inside radius, length and angle.
    sizes: Vec<f64>,
}

#[derive(Debug, Default)]
struct Pairs {
    /// An earlier feature's number: the later one's, and how far it moved.
    later: BTreeMap<usize, (usize, f64)>,
    /// Earlier features with no one match.
    two_fit: BTreeSet<usize>,
}

#[derive(Debug)]
struct PairsByKind {
    hole: Pairs,
    bend: Pairs,
}

fn hole_spots(report: &Report) -> Vec<Spot> {
    report
        .part
        .as_ref()
        .and_then(|part| part.holes.as_ref())
        .map(|holes| {
            holes.iter().map(|hole| Spot { number: hole.number, at: [hole.mm.x, hole.mm.y, hole.mm.z], sizes: vec![hole.diameter_mm] }).collect()
        })
        .unwrap_or_default()
}

fn bend_spots(report: &Report) -> Result<Vec<Spot>, DfmError> {
    let mut spots = Vec::with_capacity(report.bends.len());
    for bend in &report.bends {
        let mm = bend.location.mm.as_ref().ok_or_else(|| DfmError::Invalid("recheck: a bend has no place".to_string()))?;
        spots.push(Spot { number: bend.number, at: [mm.x, mm.y, mm.z], sizes: vec![bend.inside_radius_mm, bend.length_mm, bend.angle_deg] });
    }
    Ok(spots)
}

fn same_hole(one: &Spot, other: &Spot) -> Result<bool, DfmError> {
    within((one.sizes[0] - other.sizes[0]).abs(), SAME_SIZE_MM)
}

/// Fixing a radius or a length moves a bend's place, so one of those may differ, not both.
fn same_bend(one: &Spot, other: &Spot) -> Result<bool, DfmError> {
    Ok(within((one.sizes[2] - other.sizes[2]).abs(), SAME_ANGLE_DEG)?
        && (within((one.sizes[0] - other.sizes[0]).abs(), SAME_SIZE_MM)? || within((one.sizes[1] - other.sizes[1]).abs(), SAME_SIZE_MM)?))
}

/// In the same place, then a vanished feature with an appeared one, if each fits only the other.
fn pair(earlier: &[Spot], later: &[Spot], same: &dyn Fn(&Spot, &Spot) -> Result<bool, DfmError>, reach: Option<f64>) -> Result<Pairs, DfmError> {
    let mut pairs = Pairs::default();
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let earlier_refs: Vec<&Spot> = earlier.iter().collect();
    let later_refs: Vec<&Spot> = later.iter().collect();
    pair_up(&earlier_refs, &later_refs, &|one, other| within(distance(one.at, other.at), SAME_PLACE_MM), &mut pairs, &mut used)?;
    if let Some(reach) = reach {
        // A later feature in the same place as an earlier one can't be another that moved.
        let settled: BTreeSet<usize> = pairs.later.keys().copied().chain(pairs.two_fit.iter().copied()).collect();
        let ones: Vec<&Spot> = earlier.iter().filter(|spot| !settled.contains(&spot.number)).collect();
        let others: Vec<&Spot> = later.iter().filter(|spot| !used.contains(&spot.number)).collect();
        pair_up(&ones, &others, &|one, other| Ok(within(distance(one.at, other.at), reach)? && same(one, other)?), &mut pairs, &mut used)?;
    }
    Ok(pairs)
}

fn pair_up(
    ones: &[&Spot],
    others: &[&Spot],
    fits: &dyn Fn(&Spot, &Spot) -> Result<bool, DfmError>,
    pairs: &mut Pairs,
    used: &mut BTreeSet<usize>,
) -> Result<(), DfmError> {
    for one in ones {
        let mut mine: Vec<&Spot> = Vec::new();
        for other in others {
            if fits(one, other)? {
                mine.push(other);
            }
        }
        for other in &mine {
            used.insert(other.number);
        }
        if mine.len() == 1 {
            let mut fitting = 0usize;
            for item in ones {
                if fits(item, mine[0])? {
                    fitting += 1;
                }
            }
            if fitting == 1 {
                pairs.later.insert(one.number, (mine[0].number, distance(one.at, mine[0].at)));
                continue;
            }
        }
        if !mine.is_empty() {
            pairs.two_fit.insert(one.number);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// What a finding is about.

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FeatureRef {
    Hole(usize),
    Bend(usize),
}

/// The hole or bends a finding's own location names: `None` when a located finding names none,
/// an empty list for a whole-part finding.
fn features(finding: &Finding) -> Option<Vec<FeatureRef>> {
    if finding.location.mm.is_none() {
        return Some(Vec::new());
    }
    let text = &finding.location.text;
    if let Some(holes) = hole_features(text) {
        return Some(holes);
    }
    bend_features(text)
}

/// `^hole (\d+)(?: and hole (\d+))?, at `
fn hole_features(text: &str) -> Option<Vec<FeatureRef>> {
    let rest = text.strip_prefix("hole ")?;
    let (first, rest) = leading_number(rest)?;
    if rest.starts_with(", at ") {
        return Some(vec![FeatureRef::Hole(first)]);
    }
    let rest = rest.strip_prefix(" and hole ")?;
    let (second, rest) = leading_number(rest)?;
    rest.starts_with(", at ").then(|| vec![FeatureRef::Hole(first), FeatureRef::Hole(second)])
}

/// `bends? (\d+)(?: and (\d+))?, at `, searched anywhere.
fn bend_features(text: &str) -> Option<Vec<FeatureRef>> {
    let mut start = 0;
    while let Some(index) = text[start..].find("bend") {
        let at = start + index;
        let rest = &text[at + "bend".len()..];
        if let Some(rest) = rest.strip_prefix('s')
            && let Some(found) = bend_names(rest)
        {
            return Some(found);
        }
        if let Some(found) = bend_names(rest) {
            return Some(found);
        }
        start = at + "bend".len();
    }
    None
}

fn bend_names(rest: &str) -> Option<Vec<FeatureRef>> {
    let rest = rest.strip_prefix(' ')?;
    let (first, rest) = leading_number(rest)?;
    if rest.starts_with(", at ") {
        return Some(vec![FeatureRef::Bend(first)]);
    }
    let rest = rest.strip_prefix(" and ")?;
    let (second, rest) = leading_number(rest)?;
    rest.starts_with(", at ").then(|| vec![FeatureRef::Bend(first), FeatureRef::Bend(second)])
}

fn leading_number(text: &str) -> Option<(usize, &str)> {
    let end = text.bytes().take_while(u8::is_ascii_digit).count();
    if end == 0 {
        return None;
    }
    let number = text[..end].parse::<usize>().ok()?;
    Some((number, &text[end..]))
}

fn is_marked(finding: &Finding) -> bool {
    problems(finding.status) || (finding.status == Status::NotVerified && finding.reason_code.as_deref() != Some(CLEAR_OF_PUBLIC))
}

fn problems(status: Status) -> bool {
    matches!(status, Status::NeedsReview | Status::Violation)
}

fn name(finding: &Finding) -> String {
    let head = finding.location.text.split(", at ").next().unwrap_or("");
    let mut chars = head.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn same_place(one: &Finding, other: &Finding) -> Result<bool, DfmError> {
    match (&one.location.mm, &other.location.mm) {
        (None, None) => Ok(true),
        (Some(first), Some(second)) => within(distance([first.x, first.y, first.z], [second.x, second.y, second.z]), SAME_PLACE_MM),
        _ => Ok(false),
    }
}

fn within(value: f64, limit: f64) -> Result<bool, DfmError> {
    // Rounded, because 10.05 - 10 is a little over 0.05 in floating point.
    Ok(python_round(value, 6)? <= limit)
}

fn distance(one: [f64; 3], other: [f64; 3]) -> f64 {
    let mut sum = 0.0;
    for axis in 0..3 {
        let delta = one[axis] - other[axis];
        sum += delta * delta;
    }
    sum.sqrt()
}

// ---------------------------------------------------------------------------
// Why a mark says what it says.

/// Where a finding's features are now and how far the furthest moved, or why it can't say.
enum Followed {
    Named(Vec<FeatureRef>, f64),
    Why(String),
}

fn followed(was: &Finding, pairs: &PairsByKind) -> Followed {
    let Some(mut features) = features(was) else {
        return Followed::Why("This finding names no hole or bend that can be followed.".to_string());
    };
    let mut found: Vec<FeatureRef> = Vec::with_capacity(features.len());
    let mut furthest = 0.0f64;
    for feature in features.drain(..) {
        let (kind, number, pairs) = match feature {
            FeatureRef::Hole(number) => ("hole", number, &pairs.hole),
            FeatureRef::Bend(number) => ("bend", number, &pairs.bend),
        };
        if let Some((later_number, moved)) = pairs.later.get(&number) {
            found.push(if kind == "hole" { FeatureRef::Hole(*later_number) } else { FeatureRef::Bend(*later_number) });
            furthest = furthest.max(*moved);
        } else if pairs.two_fit.contains(&number) {
            return Followed::Why(format!(
                "More than one {kind} here could be the earlier report's {kind} {number}, so none is picked. A person decides."
            ));
        } else {
            return Followed::Why(format!(
                "The earlier report's {kind} {number} isn't where it was, and nothing here can be told to be it, so this can't be called fixed. If it was removed, the guard says so. A person decides."
            ));
        }
    }
    Followed::Named(found, furthest)
}

fn still(was: &Finding, now: &Finding, moved: f64) -> String {
    let mut words = format!("{} still has this finding.", name(was));
    if moved > SAME_PLACE_MM {
        words.push_str(&format!(" Its feature moved {} mm, and it's the only one that fits.", py_g(moved, 3)));
    }
    if let (Some(before), Some(after)) = (&was.measured, &now.measured) {
        words.push_str(&format!(" It read {} {}, and now reads {}.", py_g(before.value, 6), before.unit, py_g(after.value, 6)));
    }
    words
}

fn fixed(was: &Finding, moved: f64) -> String {
    let rule = was.rules.first().map(String::as_str).unwrap_or("this rule");
    let mut words = format!("{} is still there, and {rule} no longer says this. The guard flagged nothing.", name(was));
    if moved > SAME_PLACE_MM {
        words.push_str(&format!(" Its feature moved {} mm, and it's the only one that fits.", py_g(moved, 3)));
    }
    if let Some(requirement) = &was.requirement
        && never_decides(requirement.layer)
    {
        words.push_str(" That was against a public value, so this only says it's clear of that value, and nothing about a shop's number.");
    }
    words.push_str(" This says nothing about anything but this rule.");
    words
}

fn never_decides(layer: Layer) -> bool {
    matches!(layer, Layer::Generic | Layer::Development)
}

fn on_rules(one: &Finding, other: &Finding) -> bool {
    one.rules.iter().any(|rule| other.rules.iter().any(|name| name == rule))
}

/// The rule said something about this feature, so its finding going away means something.
fn rule_answers(was: &Finding, later: &Report) -> bool {
    let Some(rule) = was.rules.first() else { return false };
    let answers: Vec<&Finding> = later.findings.iter().filter(|finding| finding.rules.iter().any(|name| name == rule)).collect();
    let looked = rule == RELIEF_RULE || rule == RADII_RULE;
    !answers.is_empty()
        && answers.iter().all(|finding| {
            finding.status == Status::Pass
                || problems(finding.status)
                || (finding.status == Status::NotVerified
                    && finding.reason_code.as_deref().is_some_and(|code| code == CLEAR_OF_PUBLIC || (looked && code == "NO_SHOP_NUMBER")))
        })
}

/// A web rule's answer may be about another web, so this pair must still be a web or a U.
fn web_stands(was: &Finding, features: &[FeatureRef], later: &Report) -> bool {
    let kind = was.feature.kind.as_str();
    if kind != "web" && kind != "U" {
        return true;
    }
    let pair: BTreeSet<usize> = features
        .iter()
        .map(|feature| match feature {
            FeatureRef::Hole(number) | FeatureRef::Bend(number) => *number,
        })
        .collect();
    later.part.as_ref().is_some_and(|part| {
        part.webs.iter().any(|web| {
            let bends: BTreeSet<usize> = web.bends.iter().copied().collect();
            bends == pair && (kind == "web" || web.same_side)
        })
    })
}

fn gate_answers(report: &Report) -> Vec<Status> {
    report.findings.iter().filter(|finding| finding.rules.iter().any(|rule| rule == BEND_GATE)).map(|finding| finding.status).collect()
}

// ---------------------------------------------------------------------------
// The marks.

type MarkKey = (Vec<String>, String, Option<BTreeSet<FeatureRef>>);

fn mark_key(finding: &Finding, features: &Option<Vec<FeatureRef>>) -> MarkKey {
    // A set, because bends are numbered by place, so a web's two bends can swap numbers.
    let pair = features.as_ref().map(|list| list.iter().cloned().collect::<BTreeSet<_>>());
    (finding.rules.clone(), finding.feature.kind.clone(), pair)
}

fn marks(earlier: &Report, later: &Report, unsure: bool) -> Result<Vec<Mark>, DfmError> {
    let thickness = earlier.traceability.declared.thickness_mm;
    let same_t = thickness == later.traceability.declared.thickness_mm;
    let reach = match thickness {
        Some(thickness) if same_t => Some(thickness * MOVED_REACH_SHEETS),
        _ => None,
    };
    let earlier_holes = hole_spots(earlier);
    let later_holes = hole_spots(later);
    let earlier_bends = bend_spots(earlier)?;
    let later_bends = bend_spots(later)?;
    let pairs =
        PairsByKind { hole: pair(&earlier_holes, &later_holes, &same_hole, reach)?, bend: pair(&earlier_bends, &later_bends, &same_bend, reach)? };
    // One gone and one come with neither followed may be the whole part moved, which can clear a
    // finding's place and leave its feature elsewhere, so no place is proof of anything.
    let unsure = unsure
        || pairs.hole.later.len() < earlier_holes.len().min(later_holes.len())
        || pairs.bend.later.len() < earlier_bends.len().min(later_bends.len());

    let mut marks: Vec<Mark> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut unmatched: Vec<usize> = Vec::new();
    let mut waiting: Vec<(usize, Vec<FeatureRef>, f64)> = Vec::new();

    let later_marked: Vec<usize> = later.findings.iter().enumerate().filter(|(_, finding)| is_marked(finding)).map(|(index, _)| index).collect();
    let mut same_cause: BTreeMap<(Option<String>, Vec<String>), usize> = BTreeMap::new();
    for &index in &later_marked {
        let finding = &later.findings[index];
        if finding.status == Status::NotVerified {
            same_cause.insert((finding.reason_code.clone(), finding.needs.clone()), index);
        }
    }
    let mut groups: BTreeMap<MarkKey, Vec<usize>> = BTreeMap::new();
    for &index in &later_marked {
        let finding = &later.findings[index];
        if problems(finding.status) {
            groups.entry(mark_key(finding, &features(finding))).or_default().push(index);
        }
    }

    for (index, was) in earlier.findings.iter().enumerate() {
        if !is_marked(was) {
            continue;
        }
        if was.status == Status::NotVerified {
            // It was never found wrong, so going away can't be a fix.
            let cause = same_cause.get(&(was.reason_code.clone(), was.needs.clone())).copied();
            match cause {
                Some(cause) => push_mark(
                    &mut marks,
                    &mut taken,
                    MarkWord::StillThere,
                    Some(was),
                    Some(&later.findings[cause]),
                    format!("Still couldn't check: {}.", was.reason_code.clone().unwrap_or_default()),
                    None,
                )?,
                None => {
                    unmatched.push(index);
                    push_mark(
                        &mut marks,
                        &mut taken,
                        MarkWord::CantMatch,
                        Some(was),
                        None,
                        format!(
                            "This said it couldn't check ({}), so there's nothing to call fixed. The findings here say what those rules say now.",
                            was.reason_code.clone().unwrap_or_default()
                        ),
                        None,
                    )?;
                }
            }
            continue;
        }
        match followed(was, &pairs) {
            Followed::Why(why) => {
                unmatched.push(index);
                push_mark(&mut marks, &mut taken, MarkWord::CantMatch, Some(was), None, why, None)?;
            }
            Followed::Named(features_found, moved) => {
                let key = mark_key(was, &Some(features_found.clone()));
                let mut here: Vec<usize> = Vec::new();
                if let Some(group) = groups.get(&key) {
                    for &candidate in group {
                        if same_place(was, &later.findings[candidate])? {
                            here.push(candidate);
                        }
                    }
                }
                if here.len() == 1 {
                    // Before any other match, so none can take another's.
                    if let Some(group) = groups.get_mut(&key) {
                        group.retain(|known| *known != here[0]);
                    }
                    let moved_mm = if features_found.is_empty() { None } else { Some(moved) };
                    push_mark(
                        &mut marks,
                        &mut taken,
                        MarkWord::StillThere,
                        Some(was),
                        Some(&later.findings[here[0]]),
                        still(was, &later.findings[here[0]], moved),
                        moved_mm,
                    )?;
                } else {
                    waiting.push((index, features_found, moved));
                }
            }
        }
    }

    let mut maybe_fixed: Vec<(usize, Vec<FeatureRef>, f64)> = Vec::new();
    for (index, features_found, moved) in waiting {
        let was = &earlier.findings[index];
        let key = mark_key(was, &Some(features_found.clone()));
        let count = groups.get(&key).map_or(0, Vec::len);
        if count == 1 && was.rules.as_slice() != [RELIEF_RULE] {
            let now_index = groups.get_mut(&key).and_then(Vec::pop);
            if let Some(now_index) = now_index {
                let moved_mm = if features_found.is_empty() { None } else { Some(moved) };
                push_mark(
                    &mut marks,
                    &mut taken,
                    MarkWord::StillThere,
                    Some(was),
                    Some(&later.findings[now_index]),
                    still(was, &later.findings[now_index], moved),
                    moved_mm,
                )?;
            }
        } else if count > 0 {
            let label = was.rules.first().map(String::as_str).unwrap_or("this rule");
            push_mark(
                &mut marks,
                &mut taken,
                MarkWord::CantMatch,
                Some(was),
                None,
                format!("{label} has a finding on this rule, but not at this place, so this can't say whether it's the same one."),
                None,
            )?;
        } else if unsure {
            push_mark(
                &mut marks,
                &mut taken,
                MarkWord::CantMatch,
                Some(was),
                None,
                "The guard flagged a change or couldn't compare something, or a feature went and another came that can't be followed, so this going away may be down to that and not to a fix. A person decides."
                    .to_string(),
                None,
            )?;
        } else if !(rule_answers(was, later) && web_stands(was, &features_found, later)) {
            let label = was.rules.first().map(String::as_str).unwrap_or("this rule");
            push_mark(
                &mut marks,
                &mut taken,
                MarkWord::CantMatch,
                Some(was),
                None,
                format!("{label} has no answer that covers this now, so this can't be called fixed."),
                None,
            )?;
        } else {
            maybe_fixed.push((index, features_found, moved));
        }
    }

    // A part moved or turned whole clears each finding's place and leaves its finding elsewhere, so
    // a later finding that nothing explains stops a fixed on its rule.
    let left_over: Vec<usize> = groups.values().flatten().copied().collect();
    for (index, features_found, moved) in maybe_fixed {
        let was = &earlier.findings[index];
        if left_over.iter().any(|&other| on_rules(was, &later.findings[other])) {
            let label = was.rules.first().map(String::as_str).unwrap_or("this rule");
            push_mark(
                &mut marks,
                &mut taken,
                MarkWord::CantMatch,
                Some(was),
                None,
                format!(
                    "{label} raises something here that nothing else explains, and it may be this one, so this can't be called fixed. A person decides."
                ),
                None,
            )?;
        } else {
            let moved_mm = if features_found.is_empty() { None } else { Some(moved) };
            push_mark(&mut marks, &mut taken, MarkWord::Fixed, Some(was), None, fixed(was, moved), moved_mm)?;
        }
    }

    for &index in &later_marked {
        let now = &later.findings[index];
        if taken.contains(&now.label) {
            continue;
        }
        if unmatched.iter().any(|&other| on_rules(now, &earlier.findings[other])) {
            let label = now.rules.first().map(String::as_str).unwrap_or("this rule");
            push_mark(
                &mut marks,
                &mut taken,
                MarkWord::CantMatch,
                None,
                Some(now),
                format!("An earlier finding on {label} can't be matched, and this may be the same one, so this can't be called new."),
                None,
            )?;
        } else {
            push_mark(
                &mut marks,
                &mut taken,
                MarkWord::New,
                None,
                Some(now),
                format!("{} has this now, and the earlier report didn't.", name(now)),
                None,
            )?;
        }
    }

    Ok(marks)
}

fn push_mark(
    marks: &mut Vec<Mark>,
    taken: &mut BTreeSet<String>,
    word: MarkWord,
    was: Option<&Finding>,
    now: Option<&Finding>,
    why: String,
    moved: Option<f64>,
) -> Result<(), DfmError> {
    let Some(shown) = now.or(was) else {
        return Err(DfmError::Invalid("recheck: a mark has neither an earlier nor a later finding".to_string()));
    };
    if let Some(now) = now {
        taken.insert(now.label.clone());
    }
    let moved_mm = match moved {
        None => None,
        Some(moved) => Some(python_round(moved, 3)?),
    };
    marks.push(Mark {
        mark: word,
        earlier: was.map(|finding| finding.label.clone()),
        later: now.map(|finding| finding.label.clone()),
        rules: shown.rules.clone(),
        feature: shown.feature.kind.clone(),
        place: shown.location.text.clone(),
        moved_mm,
        why,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// The guard.

fn flag(code: &str, earlier: Option<String>, later: Option<String>, message: String) -> GuardFlag {
    GuardFlag { code: code.to_string(), earlier, later, message }
}

fn dropped(code: &str, noun: &str, before: usize, after: usize) -> Vec<GuardFlag> {
    if after >= before {
        return Vec::new();
    }
    vec![flag(
        code,
        Some(before.to_string()),
        Some(after.to_string()),
        format!("The number of {noun} dropped from {before} to {after}. {PERSON_DECIDES}"),
    )]
}

fn far_apart(before: f64, after: f64) -> bool {
    (after - before).abs() > LARGE_CHANGE_SHARE * before.abs()
}

/// `" x "`-joined, each value in `%g` spelling.
fn box_join(size: &[f64]) -> String {
    size.iter().map(|value| py_g(*value, 6)).collect::<Vec<_>>().join(" x ")
}

fn guard(earlier: &Report, later: &Report) -> Result<Guard, DfmError> {
    let mut flags: Vec<GuardFlag> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    match (earlier.part.as_ref(), later.part.as_ref()) {
        (Some(before), Some(after)) => {
            match (before.holes.as_ref(), after.holes.as_ref()) {
                (Some(holes_then), Some(holes_now)) => {
                    flags.extend(dropped("HOLE_COUNT_DROPPED", "round holes", holes_then.len(), holes_now.len()));
                }
                _ => missing.push("holes".to_string()),
            }
            flags.extend(dropped("OPENING_COUNT_DROPPED", "other openings", before.other_openings, after.other_openings));
            let bends = dropped("BEND_COUNT_DROPPED", "bends", earlier.bends.len(), later.bends.len());
            let dropped_bends = !bends.is_empty();
            flags.extend(bends);
            let known = |report: &Report| {
                let answers = gate_answers(report);
                answers.as_slice() == [Status::Pass].as_slice() || answers.as_slice() == [Status::NotApplicable].as_slice()
            };
            if !dropped_bends && !(known(earlier) && known(later)) {
                missing.push("bends".to_string());
            }
            match (before.size_mm.as_ref(), after.size_mm.as_ref()) {
                (Some(size_then), Some(size_now)) => {
                    if size_then.len() != size_now.len() {
                        return Err(DfmError::Invalid("recheck: the part's size has a different number of axes".to_string()));
                    }
                    if size_then.iter().zip(size_now.iter()).any(|(then, now)| far_apart(*then, *now)) {
                        let message =
                            format!("The part's size changed from {} mm to {} mm. {PERSON_DECIDES}", box_join(size_then), box_join(size_now));
                        flags.push(flag("SIZE_CHANGED", Some(box_join(size_then)), Some(box_join(size_now)), message));
                    }
                }
                _ => missing.push("size".to_string()),
            }
            if far_apart(before.volume_mm3, after.volume_mm3) {
                let message =
                    format!("The volume changed from {} to {} mm3. {PERSON_DECIDES}", py_g(before.volume_mm3, 6), py_g(after.volume_mm3, 6));
                flags.push(flag("VOLUME_CHANGED", Some(py_g(before.volume_mm3, 6)), Some(py_g(after.volume_mm3, 6)), message));
            }
        }
        _ => {
            missing = ["holes", "other openings", "bends", "size", "volume"].into_iter().map(str::to_string).collect();
        }
    }
    flags.extend(changed_inputs(earlier, later));
    Ok(Guard { flags, not_compared: missing })
}

fn changed_inputs(earlier: &Report, later: &Report) -> Vec<GuardFlag> {
    let then = &earlier.traceability;
    let now = &later.traceability;
    let was = &then.declared;
    let is_now = &now.declared;
    let mut flags: Vec<GuardFlag> = Vec::new();
    declared_flag(&mut flags, "process", was.process.clone(), is_now.process.clone(), "");
    declared_flag(&mut flags, "material", was.material.clone(), is_now.material.clone(), " It may be softer now, which the file can't show.");
    if was.thickness_mm != is_now.thickness_mm {
        flags.push(flag(
            "DECLARED_CHANGED",
            number_flag(was.thickness_mm),
            number_flag(is_now.thickness_mm),
            format!(
                "The declared thickness changed from {} to {}. {PERSON_DECIDES}",
                text_or_none_number(was.thickness_mm),
                text_or_none_number(is_now.thickness_mm)
            ),
        ));
    }
    declared_flag(&mut flags, "shop", was.shop.clone(), is_now.shop.clone(), "");
    if was.shop == is_now.shop && then.profile.hash != now.profile.hash {
        flags.push(flag(
            "SHOP_NUMBERS_CHANGED",
            then.profile.version.clone(),
            now.profile.version.clone(),
            "The shop's profile isn't the same file as before, so its numbers may differ.".to_string(),
        ));
    }
    if (then.core_version.as_str(), then.ocp_version.as_str(), then.occt_package.as_str())
        != (now.core_version.as_str(), now.ocp_version.as_str(), now.occt_package.as_str())
    {
        flags.push(flag(
            "TOOL_CHANGED",
            Some(then.core_version.clone()),
            Some(now.core_version.clone()),
            "A different version of INTdfm or its engine made the earlier report, so a finding may have changed for that reason alone.".to_string(),
        ));
    }
    flags
}

fn declared_flag(flags: &mut Vec<GuardFlag>, word: &str, before: Option<String>, after: Option<String>, extra: &str) {
    if before != after {
        flags.push(flag(
            "DECLARED_CHANGED",
            before.clone(),
            after.clone(),
            format!(
                "The declared {word} changed from {} to {}.{extra} {PERSON_DECIDES}",
                text_or_none(before.as_deref()),
                text_or_none(after.as_deref())
            ),
        ));
    }
}

fn text_or_none(value: Option<&str>) -> String {
    value.map_or_else(|| "None".to_string(), str::to_string)
}

/// `str(value)` for a float: Python's shortest round-trip spelling, not `%g`.
fn text_or_none_number(value: Option<f64>) -> String {
    value.map_or_else(|| "None".to_string(), |number| python_float_text(number).unwrap_or_else(|_| number.to_string()))
}

fn number_flag(value: Option<f64>) -> Option<String> {
    value.map(|number| python_float_text(number).unwrap_or_else(|_| number.to_string()))
}

// ---------------------------------------------------------------------------
// Python's `%g` spelling.

/// CPython's `format(value, ".<precision>g")`: `precision` significant digits, trailing zeros
/// dropped, exponent form outside `-4 <= exponent < precision`.
fn py_g(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let negative = value.is_sign_negative();
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return if negative { "-0" } else { "0" }.to_string();
    }
    let precision = precision.max(1);
    let text = format!("{:.*e}", precision - 1, magnitude);
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return text;
    };
    let digits: String = mantissa.chars().filter(|character| *character != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let body = if exponent < -4 || exponent >= precision as i32 {
        let mut number = String::new();
        number.push_str(&digits[..1]);
        if digits.len() > 1 {
            number.push('.');
            number.push_str(&digits[1..]);
        }
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{number}e{sign}{:02}", exponent.unsigned_abs())
    } else if exponent >= 0 {
        let point = exponent as usize + 1;
        if digits.len() <= point {
            let padding = "0".repeat(point - digits.len());
            format!("{digits}{padding}")
        } else {
            format!("{}.{}", &digits[..point], &digits[point..])
        }
    } else {
        let padding = "0".repeat((-exponent - 1) as usize);
        format!("0.{padding}{digits}")
    };
    if negative { format!("-{body}") } else { body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_versions_are_one_point_six_or_later() {
        for (version, expected) in [
            ("1.5", false),
            ("1.6", true),
            ("1.8", true),
            ("1.10", true),
            ("1.06", true),
            ("2.0", false),
            ("1.6x", false),
            ("1.", false),
            ("", false),
            ("1.0000000000000000000006", true),
        ] {
            assert_eq!(version_is_readable(version), expected, "{version}");
        }
    }

    #[test]
    fn percent_g_matches_python() {
        let cases: &[(f64, usize, &str)] = &[
            (0.0, 6, "0"),
            (-0.0, 6, "-0"),
            (1.5, 6, "1.5"),
            (90.0, 6, "90"),
            (1500.0, 6, "1500"),
            (1_234_567.0, 6, "1.23457e+06"),
            (2_835.290_358_249_673, 6, "2835.29"),
            (0.0001, 6, "0.0001"),
            (0.00001, 6, "1e-05"),
            (0.05, 6, "0.05"),
            (1.0499999999999998, 6, "1.05"),
            (0.000_046, 6, "4.6e-05"),
            (1_000_000.0, 6, "1e+06"),
            (999_999.5, 6, "1e+06"),
            (0.199_999_999_999_999_96, 3, "0.2"),
            (0.300_000_000_000_000_04, 6, "0.3"),
            (123.456, 6, "123.456"),
            (123.456, 3, "123"),
            (0.0001234, 3, "0.000123"),
            (1234.0, 3, "1.23e+03"),
            (1.0, 3, "1"),
            (1000.0, 3, "1e+03"),
            (1500.0, 3, "1.5e+03"),
        ];
        for (value, precision, expected) in cases {
            assert_eq!(py_g(*value, *precision), *expected, "{} at {precision}", value);
        }
    }

    #[test]
    fn feature_names_follow_the_location_text() {
        assert_eq!(hole_features("hole 3, at x 1, y 2, z 3"), Some(vec![FeatureRef::Hole(3)]));
        assert_eq!(hole_features("hole 3 and hole 7, at x 1"), Some(vec![FeatureRef::Hole(3), FeatureRef::Hole(7)]));
        assert_eq!(hole_features("hole 3 and 7, at x 1"), None);
        assert_eq!(bend_features("bend 2, at x 1"), Some(vec![FeatureRef::Bend(2)]));
        assert_eq!(bend_features("the web between bends 1 and 2, at x 1"), Some(vec![FeatureRef::Bend(1), FeatureRef::Bend(2)]));
        assert_eq!(bend_features("the U between bends 1 and 2, at x 1"), Some(vec![FeatureRef::Bend(1), FeatureRef::Bend(2)]));
        assert_eq!(bend_features("hole 1, at x 1"), None);
        assert_eq!(bend_features("bends 1 and 2, at x 1"), Some(vec![FeatureRef::Bend(1), FeatureRef::Bend(2)]));
    }

    #[test]
    fn a_distance_is_the_square_root_of_the_summed_squares() {
        assert_eq!(distance([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]), 5.0);
        assert_eq!(distance([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]), 0.0);
    }
}
