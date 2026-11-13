//! `recheck.py` against the INTdfm comparison layer's own answers.
//!
//! Each fixture holds two INTdfm-shaped reports and the `recheck` object `recheck.compare` made
//! from them. No OCP is installed in this environment, so a real before/after STEP pair cannot be
//! recorded: the earlier report is a committed recording's report, and the later report is the
//! real INTdfm rule and report layer on one edited measurement (a moved or lost hole, a changed
//! table, a changed tool version). Every pair is labelled `TEST-ONLY`, every expected value comes
//! from the Python function, and no pass is invented.
//!
//! Status, marks, guards and `not_compared` are compared exactly, and the whole `Recheck` is also
//! compared as canonical report bytes, so a message that drifts shows as a byte difference. The
//! only thing not compared is the two reports themselves: their bytes are the report layer's
//! business (`report_parity.rs`), and both sides of this test embed the same `later` report.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use solidcraft_dfm_rules::intdfm_report::{Report, canonical_json};
use solidcraft_dfm_rules::recheck::{compare, readable};

#[derive(Deserialize)]
struct Fixture {
    source: Source,
    case: Case,
    earlier: Report,
    later: Report,
    expected: solidcraft_dfm_rules::intdfm_report::Recheck,
}

#[derive(Deserialize)]
struct Source {
    intdfm_commit: String,
    core_source_sha256: String,
    base_recording: String,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    description: String,
    test_only: bool,
    why_test_only: String,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/recheck/v1")
}

fn fixture(id: &str) -> Fixture {
    let path = fixtures_dir().join(format!("{id}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// One line of context when the bytes differ, instead of two multi-kilobyte strings.
fn first_difference(id: &str, actual: &str, expected: &str) -> String {
    let mut message = String::new();
    for (index, (left, right)) in actual.lines().zip(expected.lines()).enumerate() {
        if left != right {
            let _ = write!(message, "{id}: canonical bytes differ at line {}:\n  rust:   {left}\n  python: {right}", index + 1);
            return message;
        }
    }
    format!("{id}: canonical bytes differ: rust {} bytes, python {} bytes", actual.len(), expected.len())
}

fn check(id: &str) {
    let fixture = fixture(id);
    assert_eq!(fixture.case.id, id);
    assert!(!fixture.source.intdfm_commit.is_empty());
    assert_eq!(fixture.source.core_source_sha256.len(), 64);
    assert!(!fixture.source.base_recording.is_empty());
    assert!(!fixture.case.description.is_empty());
    assert!(readable(&fixture.earlier), "{id}: the earlier report is not readable");
    assert!(readable(&fixture.later), "{id}: the later report is not readable");

    let produced = compare(&fixture.earlier, &fixture.later).unwrap_or_else(|error| panic!("{id}: compare: {error}"));
    // Marks, guards, not_compared and every message: exact.
    assert_eq!(produced, fixture.expected, "{id}: the recheck object differs");

    let mut wanted = fixture.later.clone();
    wanted.recheck = Some(fixture.expected.clone());
    let mut actual = fixture.later.clone();
    actual.recheck = Some(produced);
    let expected_bytes = canonical_json(&wanted).unwrap_or_else(|error| panic!("{id}: canonical expected: {error}"));
    let actual_bytes = canonical_json(&actual).unwrap_or_else(|error| panic!("{id}: canonical actual: {error}"));
    if actual_bytes != expected_bytes {
        panic!("{}", first_difference(id, &actual_bytes, &expected_bytes));
    }
}

macro_rules! fixture_cases {
    ($($test:ident => $id:literal),* $(,)?) => {
        $(#[test]
        fn $test() {
            check($id);
        })*

        const FIXTURE_IDS: &[&str] = &[$($id),*];
    };
}

fixture_cases! {
    synthetic_same_part => "synthetic.same_part",
    synthetic_two_holes_hole_moved => "synthetic.two_holes_hole_moved",
    synthetic_row_hole_moved => "synthetic.row_hole_moved",
    synthetic_feature_cannot_follow => "synthetic.feature_cannot_follow",
    synthetic_ambiguous_feature => "synthetic.ambiguous_feature",
    synthetic_lost_hole => "synthetic.lost_hole",
    synthetic_new_edge_review => "synthetic.new_edge_review",
    synthetic_fixed_review => "synthetic.fixed_review",
    synthetic_profile_changed => "synthetic.profile_changed",
    synthetic_tool_changed => "synthetic.tool_changed",
    synthetic_declared_changed => "synthetic.declared_changed",
}

/// A new fixture without a test, or a test without its fixture, fails here.
#[test]
fn every_fixture_has_exactly_one_case() {
    let mut found: Vec<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures folder")
        .map(|entry| entry.expect("a directory entry").file_name().into_string().expect("a UTF-8 file name"))
        .filter(|name| name.ends_with(".json"))
        .map(|name| name.trim_end_matches(".json").to_string())
        .collect();
    found.sort();
    let mut named: Vec<String> = FIXTURE_IDS.iter().map(|id| (*id).to_string()).collect();
    named.sort();
    assert_eq!(found, named);
}

/// Every pair here is built from recorded facts, so every one says so.
#[test]
fn every_pair_is_labelled_test_only() {
    for id in FIXTURE_IDS {
        let fixture = fixture(id);
        assert!(fixture.case.test_only, "{id}: a pair built from edited facts must say TEST-ONLY");
        assert!(fixture.case.description.starts_with("TEST-ONLY:"), "{id}: description");
        assert!(!fixture.case.why_test_only.is_empty(), "{id}: say why it is synthetic");
    }
}

/// The counts in a fixture add up to its marks, for every word.
#[test]
fn counts_add_up_to_the_marks() {
    for id in FIXTURE_IDS {
        let expected = fixture(id).expected;
        let mut fixed = 0;
        let mut still_there = 0;
        let mut new = 0;
        let mut cant_match = 0;
        for mark in &expected.marks {
            match mark.mark {
                solidcraft_dfm_rules::intdfm_report::MarkWord::Fixed => fixed += 1,
                solidcraft_dfm_rules::intdfm_report::MarkWord::StillThere => still_there += 1,
                solidcraft_dfm_rules::intdfm_report::MarkWord::New => new += 1,
                solidcraft_dfm_rules::intdfm_report::MarkWord::CantMatch => cant_match += 1,
            }
        }
        assert_eq!(expected.counts, solidcraft_dfm_rules::intdfm_report::MarkCounts { fixed, still_there, new, cant_match }, "{id}");
    }
}

/// Comparing a report with itself, which `core` does to validate an earlier report, can never say
/// a finding was fixed or is new. (It can still say `cant_match`: two features within the
/// same-place bound are ambiguous even for one report, see the next test.)
#[test]
fn a_report_compared_with_itself_never_says_fixed_or_new() {
    for id in FIXTURE_IDS {
        let report = fixture(id).later;
        let found = compare(&report, &report).expect("a report can be compared with itself");
        assert_eq!(found.counts.fixed, 0, "{id}");
        assert_eq!(found.counts.new, 0, "{id}");
        assert!(found.guard.flags.is_empty(), "{id}: {:?}", found.guard.flags);
        assert!(found.guard.not_compared.is_empty(), "{id}");
        assert_eq!(found.earlier_snapshot_id, report.snapshot_id, "{id}");
        assert_eq!(found.earlier_file, report.traceability.declared.file, "{id}");
    }
}

/// Two later holes within 0.05 mm of one earlier hole are structurally ambiguous: no feature is
/// picked, so this can't be called fixed even when the report is compared with itself.
#[test]
fn two_near_identical_holes_are_ambiguous_even_with_themselves() {
    let fixture = fixture("synthetic.ambiguous_feature");
    let found = compare(&fixture.later, &fixture.later).expect("a report can be compared with itself");
    assert!(found.counts.cant_match > 0, "a two_fit must not be guessed");
    assert!(found.marks.iter().any(|mark| mark.why.contains("More than one hole")), "{:?}", found.marks);
}

/// The same-part fixture is the plain case: nothing changed, so nothing is fixed or new.
#[test]
fn a_same_part_pair_is_all_still_there() {
    let expected = fixture("synthetic.same_part").expected;
    assert_eq!(expected.counts.fixed, 0);
    assert_eq!(expected.counts.new, 0);
    assert_eq!(expected.counts.cant_match, 0);
    assert!(expected.counts.still_there > 0);
    assert!(expected.guard.flags.is_empty());
    assert!(expected.guard.not_compared.is_empty());
}

// ---------------------------------------------------------------------------
// Negative cases: the test must be able to fail.

/// A changed expectation must change the bytes.
#[test]
fn a_wrong_mark_changes_the_bytes() {
    let mut fixture = fixture("synthetic.fixed_review");
    let expected_bytes = {
        let mut report = fixture.later.clone();
        report.recheck = Some(fixture.expected.clone());
        canonical_json(&report).expect("json")
    };
    let first =
        fixture.expected.marks.iter_mut().find(|mark| mark.mark == solidcraft_dfm_rules::intdfm_report::MarkWord::Fixed).expect("a fixed mark");
    first.why.push(' ');
    let mut report = fixture.later.clone();
    report.recheck = Some(fixture.expected.clone());
    assert_ne!(canonical_json(&report).expect("json"), expected_bytes);
}

/// Moving a hole in the later report must change what the comparison says.
#[test]
fn a_changed_measurement_changes_the_result() {
    let mut fixture = fixture("synthetic.row_hole_moved");
    let expected = compare(&fixture.earlier, &fixture.later).expect("the recorded pair");
    let hole = fixture.later.part.as_mut().and_then(|part| part.holes.as_mut()).expect("holes").first_mut().expect("a hole");
    hole.mm.x += 5.0;
    let produced = compare(&fixture.earlier, &fixture.later).expect("the changed pair");
    assert_ne!(produced, expected);
}

/// A bend with no place cannot be paired; Python asserts, Rust refuses.
#[test]
fn a_bend_without_a_place_is_refused() {
    let mut fixture = fixture("synthetic.two_holes_hole_moved");
    let bend = fixture.earlier.bends.first_mut().expect("a bend");
    bend.location.mm = None;
    assert!(compare(&fixture.earlier, &fixture.later).is_err());
}

/// Python's strict zip over the two size lists raises; Rust refuses instead of comparing short.
#[test]
fn a_size_with_a_different_number_of_axes_is_refused() {
    let mut fixture = fixture("synthetic.lost_hole");
    fixture.earlier.part.as_mut().expect("part").size_mm = Some(vec![40.0]);
    assert!(compare(&fixture.earlier, &fixture.later).is_err());
}

/// `readable` follows the report version, not the shape of the value.
#[test]
fn an_old_report_is_not_readable() {
    let mut fixture = fixture("synthetic.same_part");
    fixture.earlier.report_version = "1.5".to_string();
    assert!(!readable(&fixture.earlier));
    fixture.earlier.report_version = "1.6".to_string();
    assert!(readable(&fixture.earlier));
    fixture.earlier.report_version = "2.0".to_string();
    assert!(!readable(&fixture.earlier));
}
