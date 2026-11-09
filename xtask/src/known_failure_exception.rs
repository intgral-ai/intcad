//! `cargo xtask known-failure-exception --junit PATH... [--exceptions FILE] [--today YYYY-MM-DD]
//! [--raw-exit CODE] [--report PATH]`: grade a nextest run against the reviewed, expiring
//! exceptions in `tests/KNOWN_FAILURE_EXCEPTIONS`.
//!
//! This is deliberately narrower than `cargo xtask known-failing`. An entry names one nextest
//! test, its issue, an owner, the day it stops being valid, the proptest seed, the exact failure
//! signature and the fixed-input markers. The raw run still happens; CI only excuses a failure
//! when the same test really ran, really failed, and its failure text contains the recorded
//! signature and every input marker. A missing test, a skipped test, an excused test that passed,
//! an expired entry, an unmatched failure, or any other failed test fails the gate.
//!
//! Entry format, one test a line, sorted by test id exactly once:
//!
//! ```text
//! <nextest test id> | issue=#N | owner=<who accepted it> | expires=YYYY-MM-DD | seed=<proptest seed> | signature=<marker>[;<marker>...] | input=<marker>[;<marker>...] | why=<why it is accepted>
//! ```
//!
//! `signature` and `input` markers are compared after whitespace is collapsed, so a multi-line
//! proptest minimal input can be written on one line. `expires` is the first invalid day (UTC),
//! so the last valid day is the day before it. There is no renewal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};

pub const LIST: &str = "tests/KNOWN_FAILURE_EXCEPTIONS";

const FIELDS: &[&str] = &["issue", "owner", "expires", "seed", "signature", "input", "why"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Date {
    year: i32,
    month: u32,
    day: u32,
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

#[derive(Debug)]
struct Exception {
    test: String,
    issue: String,
    owner: String,
    expires: Date,
    seed: String,
    signature: Vec<String>,
    input: Vec<String>,
    why: String,
}

#[derive(Debug, Default)]
struct Outcome {
    failed: bool,
    text: String,
}

pub fn run(root: &Path, rest: &[&str]) -> Result<(), String> {
    let mut reports = Vec::new();
    let mut exceptions = root.join(LIST);
    let mut today = today_utc();
    let mut report_path = None;
    let mut raw_exit = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--junit" => reports.push(PathBuf::from(args.next().ok_or("--junit needs a junit file or a directory of them")?)),
            "--exceptions" => exceptions = PathBuf::from(args.next().ok_or("--exceptions needs a file")?),
            "--today" => {
                let text = args.next().ok_or("--today needs YYYY-MM-DD")?;
                today = parse_date(text).map_err(|e| format!("--today {text}: {e}"))?;
            }
            "--raw-exit" => {
                let text = args.next().ok_or("--raw-exit needs nextest's exit code")?;
                raw_exit = Some(text.parse::<i32>().map_err(|_| format!("--raw-exit `{text}` is not a number"))?);
            }
            "--report" => report_path = Some(PathBuf::from(args.next().ok_or("--report needs a file")?)),
            other => return Err(format!("known-failure-exception: unknown argument `{other}`")),
        }
    }
    if reports.is_empty() {
        return Err("known-failure-exception: give the nextest junit report with --junit FILE or DIR".into());
    }
    let mut files = Vec::new();
    for path in &reports {
        xml_files(path, &mut files)?;
    }
    if files.is_empty() {
        return Err(format!(
            "known-failure-exception: no junit (.xml) file in {}",
            reports.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
        ));
    }
    let mut outcomes = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        merge(&mut outcomes, &text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    let text = std::fs::read_to_string(&exceptions).map_err(|e| format!("{}: {e}", exceptions.display()))?;
    let parsed = parse_exceptions(&text).map_err(|e| format!("{}: {e}", exceptions.display()))?;
    let result = check_raw_exit(raw_exit, &outcomes).and_then(|()| grade(&parsed, &outcomes, today));
    let body = match &result {
        Ok(report) => report.clone(),
        Err(error) => format!("known-failure-exception: FAILED\n{error}"),
    };
    if let Some(path) = &report_path {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(path, &body).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    match result {
        Ok(report) => {
            println!("{report}");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// `cargo xtask lists`: the exception list is sorted, each test once, every field present and
/// non-empty, and every expiry a real ISO date.
pub fn check_list(root: &Path) -> Result<(), String> {
    let file = root.join(LIST);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    parse_exceptions(&text).map_err(|e| format!("{LIST}: {e}"))?;
    Ok(())
}

fn parse_exceptions(text: &str) -> Result<Vec<Exception>, String> {
    let mut out: Vec<Exception> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let mut parts = line.split(" | ");
        let test = parts.next().unwrap_or("").trim();
        if test.is_empty() {
            return Err(format!("line {number}: no test id"));
        }
        if test.split_whitespace().count() != 1 || !test.contains("::") {
            return Err(format!("line {number}: `{test}` is not a nextest id (`<binary id>::<test name>`)"));
        }
        let mut values: BTreeMap<&str, &str> = BTreeMap::new();
        for part in parts {
            let part = part.trim();
            let (key, value) = part.split_once('=').ok_or_else(|| format!("line {number}: `{part}` is not `field=value`"))?;
            let (key, value) = (key.trim(), value.trim());
            if !FIELDS.contains(&key) {
                return Err(format!("line {number}: unknown field `{key}`"));
            }
            if value.is_empty() {
                return Err(format!("line {number}: `{key}` is empty"));
            }
            if values.insert(key, value).is_some() {
                return Err(format!("line {number}: `{key}` appears twice"));
            }
        }
        for field in FIELDS {
            if !values.contains_key(field) {
                return Err(format!("line {number}: missing `{field}`"));
            }
        }
        let issue = values["issue"];
        let digits = issue.strip_prefix('#').unwrap_or("");
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("line {number}: issue `{issue}` is not `#N`"));
        }
        let expires = parse_date(values["expires"]).map_err(|e| format!("line {number}: expires: {e}"))?;
        let signatures = markers(values["signature"], number, "signature")?;
        let input = markers(values["input"], number, "input")?;
        if let Some(last) = out.last()
            && last.test.as_str() >= test
        {
            return Err(format!(
                "line {number}: {test} is {} {}: keep the list sorted, each test once",
                if last.test == test { "a repeat of" } else { "out of order after" },
                last.test
            ));
        }
        out.push(Exception {
            test: test.to_string(),
            issue: issue.to_string(),
            owner: values["owner"].to_string(),
            expires,
            seed: values["seed"].to_string(),
            signature: signatures.into_iter().map(str::to_string).collect(),
            input: input.into_iter().map(str::to_string).collect(),
            why: values["why"].to_string(),
        });
    }
    Ok(out)
}

/// A `;`-separated field of non-empty markers, each required in the failure text.
fn markers<'a>(text: &'a str, line: usize, field: &str) -> Result<Vec<&'a str>, String> {
    let markers: Vec<&str> = text.split(';').map(str::trim).collect();
    if markers.iter().any(|marker| marker.is_empty()) {
        return Err(format!("line {line}: {field} has an empty marker"));
    }
    Ok(markers)
}

fn parse_date(text: &str) -> Result<Date, String> {
    let mut parts = text.split('-');
    let year_text = parts.next().ok_or("date needs YYYY-MM-DD")?;
    let month_text = parts.next().ok_or("date needs YYYY-MM-DD")?;
    let day_text = parts.next().ok_or("date needs YYYY-MM-DD")?;
    if parts.next().is_some() || year_text.len() != 4 || month_text.len() != 2 || day_text.len() != 2 {
        return Err("date needs YYYY-MM-DD".into());
    }
    let year: i32 = year_text.parse().map_err(|_| "year is not a number")?;
    let month: u32 = month_text.parse().map_err(|_| "month is not a number")?;
    let day: u32 = day_text.parse().map_err(|_| "day is not a number")?;
    if !(1..=9999).contains(&year) {
        return Err("year is out of range".into());
    }
    if !(1..=12).contains(&month) {
        return Err("month is out of range".into());
    }
    let days = days_in_month(year, month);
    if day == 0 || day > days {
        return Err(format!("day is out of range for {year:04}-{month:02}"));
    }
    Ok(Date { year, month, day })
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn today_utc() -> Date {
    let days = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs() / 86_400);
    civil_from_days(days as i64)
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (mp + if mp < 10 { 3 } else { -9 }) as u32;
    Date { year: (if month <= 2 { year + 1 } else { year }) as i32, month, day }
}

/// `path` itself, or every `.xml` file under it (sorted), into `out`.
fn xml_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    let entries = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            xml_files(&path, out)?;
        } else if path.extension().is_some_and(|extension| extension == "xml") {
            out.push(path);
        }
    }
    Ok(())
}

fn merge(all: &mut BTreeMap<String, Outcome>, xml: &str) -> Result<(), String> {
    for (test, outcome) in junit_outcomes(xml)? {
        let entry = all.entry(test).or_default();
        entry.failed |= outcome.failed;
        if !outcome.text.is_empty() {
            if !entry.text.is_empty() {
                entry.text.push('\n');
            }
            entry.text.push_str(&outcome.text);
        }
    }
    Ok(())
}

/// Every test a junit report ran, as `<suite>::<test case>`, with whether it failed and the text
/// of its `failure`/`error` elements. A skipped case did not run and is left out. A case that
/// passed on a retry (nextest's `flakyFailure`) passed.
fn junit_outcomes(xml: &str) -> Result<BTreeMap<String, Outcome>, String> {
    let mut reader = Reader::from_str(xml);
    let mut suite: Option<String> = None;
    let mut case: Option<(String, Outcome, bool)> = None;
    let mut in_failure = false;
    let mut out: BTreeMap<String, Outcome> = BTreeMap::new();
    loop {
        let event = reader.read_event().map_err(|e| format!("not a junit report: {e}"))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"testsuite" if !empty => suite = Some(attribute(&e, "name")?),
                b"testcase" => {
                    let name = attribute(&e, "name")?;
                    let suite = suite.as_deref().ok_or_else(|| format!("test case {name} outside a testsuite"))?;
                    let id = format!("{suite}::{name}");
                    if empty {
                        out.entry(id).or_default();
                    } else {
                        case = Some((id, Outcome::default(), false));
                    }
                }
                b"failure" | b"error" => {
                    if let Some((_, outcome, _)) = case.as_mut() {
                        outcome.failed = true;
                    }
                    in_failure = !empty;
                }
                b"skipped" => {
                    if let Some((_, _, skipped)) = case.as_mut() {
                        *skipped = true;
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_failure && let Some((_, outcome, _)) = case.as_mut() {
                    let decoded = text.decode().map_err(|e| format!("junit text: {e}"))?;
                    let decoded = unescape(&decoded).map_err(|e| format!("junit text: {e}"))?;
                    outcome.text.push_str(&decoded);
                    outcome.text.push('\n');
                }
            }
            Event::CData(text) => {
                if in_failure && let Some((_, outcome, _)) = case.as_mut() {
                    let decoded = text.decode().map_err(|e| format!("junit text: {e}"))?;
                    outcome.text.push_str(&decoded);
                    outcome.text.push('\n');
                }
            }
            Event::End(e) => match e.name().as_ref() {
                b"failure" | b"error" => in_failure = false,
                b"testcase" => {
                    if let Some((id, outcome, skipped)) = case.take()
                        && !skipped
                    {
                        let entry = out.entry(id).or_default();
                        entry.failed |= outcome.failed;
                        entry.text.push_str(&outcome.text);
                    }
                }
                b"testsuite" => suite = None,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn attribute(e: &BytesStart, name: &str) -> Result<String, String> {
    let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let attribute = e.try_get_attribute(name).map_err(|err| format!("<{tag}>: {err}"))?.ok_or_else(|| format!("<{tag}> has no {name}"))?;
    attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|value| value.into_owned()).map_err(|err| format!("<{tag} {name}>: {err}"))
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_raw_exit(raw_exit: Option<i32>, outcomes: &BTreeMap<String, Outcome>) -> Result<(), String> {
    let Some(code) = raw_exit else {
        return Ok(());
    };
    if code != 0 && code != 100 {
        return Err(format!(
            "RAW EXIT: nextest exited {code}, not 0 (pass) or 100 (test failure): this is an infrastructure result, not an excused test failure"
        ));
    }
    let failed = outcomes.values().any(|outcome| outcome.failed);
    match (code, failed) {
        (0, true) => Err("RAW EXIT: nextest exited 0 but the junit report has a failed test".into()),
        (100, false) => Err("RAW EXIT: nextest exited 100 but the junit report has no failed test".into()),
        _ => Ok(()),
    }
}

fn grade(exceptions: &[Exception], outcomes: &BTreeMap<String, Outcome>, today: Date) -> Result<String, String> {
    let mut problems = Vec::new();
    let mut excused: Vec<&Exception> = Vec::new();
    for exception in exceptions {
        if today >= exception.expires {
            problems.push(format!(
                "EXPIRED: {} was excused by {} until {} (UTC) and is not renewable; remove the line or fix the test",
                exception.test, exception.issue, exception.expires
            ));
            continue;
        }
        match outcomes.get(&exception.test) {
            None => problems.push(format!(
                "MISSING: {} is excused by {} but did not run; a missing or skipped test is not an excused failure",
                exception.test, exception.issue
            )),
            Some(outcome) if !outcome.failed => problems.push(format!(
                "FIXED: {} is excused by {} but passed; remove its line from {LIST} in this pull request",
                exception.test, exception.issue
            )),
            Some(outcome) => {
                let text = normalize(&outcome.text);
                let missing: Vec<&str> = exception.signature.iter().map(String::as_str).filter(|marker| !text.contains(&normalize(marker))).collect();
                if !missing.is_empty() {
                    problems.push(format!(
                        "UNMATCHED: {} failed but its text does not contain the recorded signature marker(s) for {}: {}",
                        exception.test,
                        exception.issue,
                        missing.join("; ")
                    ));
                    continue;
                }
                let missing: Vec<&str> = exception.input.iter().map(String::as_str).filter(|marker| !text.contains(&normalize(marker))).collect();
                if !missing.is_empty() {
                    problems.push(format!(
                        "UNMATCHED: {} failed but its text does not contain the recorded input marker(s) for {}: {}",
                        exception.test,
                        exception.issue,
                        missing.join("; ")
                    ));
                    continue;
                }
                excused.push(exception);
            }
        }
    }
    for (test, outcome) in outcomes {
        if outcome.failed && !exceptions.iter().any(|exception| exception.test == *test) {
            problems.push(format!("REGRESSION, failed and not excused by {LIST}: {test}"));
        }
    }
    if !problems.is_empty() {
        let ran = outcomes.len();
        let passed = outcomes.values().filter(|outcome| !outcome.failed).count();
        return Err(format!(
            "known-failure-exception: {ran} ran, {passed} pass, {} failed; {} problem(s):\n  {}",
            ran - passed,
            problems.len(),
            problems.join("\n  ")
        ));
    }
    Ok(report(exceptions, outcomes, today, &excused))
}

fn report(exceptions: &[Exception], outcomes: &BTreeMap<String, Outcome>, today: Date, excused: &[&Exception]) -> String {
    let ran = outcomes.len();
    let passed = outcomes.values().filter(|outcome| !outcome.failed).count();
    let mut text = String::new();
    text.push_str(&format!("KNOWN FAILURE EXCEPTIONS: {} active, {} excused on {today} (UTC)\n", exceptions.len(), excused.len()));
    for exception in excused {
        text.push_str(&format!("KNOWN FAILURE: {} ({})\n", exception.issue, exception.test));
        text.push_str(&format!("  owner: {}\n", exception.owner));
        text.push_str(&format!("  expires: {} (UTC; first invalid day, no renewal)\n", exception.expires));
        text.push_str(&format!("  seed: {}\n", exception.seed));
        text.push_str(&format!("  signature: {}\n", exception.signature.join(" ; ")));
        text.push_str(&format!("  input: {}\n", exception.input.join(" ; ")));
        text.push_str(&format!("  why: {}\n", exception.why));
    }
    text.push_str(&format!(
        "known-failure-exception: {ran} ran, {passed} pass, {} known failure(s) excused, 0 unexcused; the raw job is still red for a known failure\n",
        ran - passed
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::{Date, Exception, Outcome, check_raw_exit, grade, junit_outcomes, parse_date, parse_exceptions};
    use std::collections::BTreeMap;

    const LIST: &str = "solidcraft-kernel::boolean_props::translation_invariant | issue=#56 | owner=repository owner (CTO) | expires=2026-12-08 | seed=cc 0f8ee4d4597db6bdec2ad2cc7e5cfe7d4ebba351e24642118135da34bf489d0c | signature=SmBSplineSurface::Test #5 Bad NonMonotonic CPts;[owner SmSphere] | input=minimal failing input: a = Sph { at: [ 0.0, 0.0, 5.0, ], r: 2.5, };b = Box { at: [ 0.0, 0.0, 0.0, ], size: [ 5.0, 5.0, 5.0, ], };off = 100000.0 | why=issue #56\n";

    const REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="3" failures="1">
    <testsuite name="solidcraft-kernel::boolean_props" tests="3" failures="1">
        <testcase name="box_box_exact" classname="solidcraft-kernel::boolean_props"/>
        <testcase name="translation_invariant" classname="solidcraft-kernel::boolean_props">
            <failure message="thread &apos;translation_invariant&apos; panicked at crates/kernel/tests/boolean_props.rs:155:1" type="test failure with exit code 101">Test failed: Union returned an error: the operation failed: SMLib boolean: SMLib&apos;s check refuses the result (the first operand already fails it): 1 failed tests:
SmBSplineSurface::Test #5 Bad NonMonotonic CPts: SmBSplineSurface Control Net doubles back on itself probably making a self intersecting surface [owner SmSphere]
.
minimal failing input: a = Sph {
    at: [
        0.0,
        0.0,
        5.0,
    ],
    r: 2.5,
}, b = Box {
    at: [
        0.0,
        0.0,
        0.0,
    ],
    size: [
        5.0,
        5.0,
        5.0,
    ],
}, off = 100000.0
successes: 0
local rejects: 0
global rejects: 0</failure>
        </testcase>
        <testcase name="commutative" classname="solidcraft-kernel::boolean_props"/>
    </testsuite>
</testsuites>
"#;

    fn exceptions() -> Vec<Exception> {
        parse_exceptions(LIST).unwrap()
    }

    fn outcomes(xml: &str) -> BTreeMap<String, Outcome> {
        junit_outcomes(xml).unwrap()
    }

    fn before_expiry() -> Date {
        Date { year: 2026, month: 11, day: 8 }
    }

    /// The report with the one failure element removed, so the excused test passed.
    fn without_failure() -> String {
        let start = REPORT.find("<failure").unwrap();
        let end = REPORT.find("</failure>").unwrap() + "</failure>".len();
        format!("{}{}", &REPORT[..start], &REPORT[end..])
    }

    /// The report with the one failure element replaced by a skip.
    fn skipped() -> String {
        let start = REPORT.find("<failure").unwrap();
        let end = REPORT.find("</failure>").unwrap() + "</failure>".len();
        format!("{}<skipped/>{}", &REPORT[..start], &REPORT[end..])
    }

    #[test]
    fn only_issue_56_is_excused_and_the_report_names_the_binding() {
        let report = grade(&exceptions(), &outcomes(REPORT), before_expiry()).unwrap();
        assert!(report.contains("KNOWN FAILURE"), "{report}");
        assert!(report.contains("#56"), "{report}");
        assert!(report.contains("translation_invariant"), "{report}");
        assert!(report.contains("0f8ee4d4597db6bdec2ad2cc7e5cfe7d4ebba351e24642118135da34bf489d0c"), "{report}");
        assert!(report.contains("repository owner (CTO)"), "{report}");
        assert!(report.contains("2026-12-08"), "{report}");
    }

    #[test]
    fn another_failure_is_not_excused() {
        let xml = REPORT.replace(
            r#"<testcase name="commutative" classname="solidcraft-kernel::boolean_props"/>"#,
            r#"<testcase name="commutative" classname="solidcraft-kernel::boolean_props"><failure message="boom">other failure</failure></testcase>"#,
        );
        let error = grade(&exceptions(), &outcomes(&xml), before_expiry()).unwrap_err();
        assert!(error.contains("REGRESSION, failed and not excused"), "{error}");
        assert!(error.contains("commutative"), "{error}");
    }

    #[test]
    fn an_expired_exception_fails_on_its_expiry_day_and_after() {
        for today in [Date { year: 2026, month: 12, day: 8 }, Date { year: 2026, month: 12, day: 9 }] {
            let error = grade(&exceptions(), &outcomes(REPORT), today).unwrap_err();
            assert!(error.contains("EXPIRED"), "{error}");
            assert!(error.contains("#56"), "{error}");
            assert!(error.contains("2026-12-08"), "{error}");
        }
        assert!(grade(&exceptions(), &outcomes(REPORT), Date { year: 2026, month: 12, day: 7 }).is_ok());
    }

    #[test]
    fn a_missing_or_skipped_excused_test_fails() {
        for xml in [
            REPORT.replace(
                r#"<testcase name="translation_invariant" classname="solidcraft-kernel::boolean_props">"#,
                r#"<testcase name="not_translation_invariant" classname="solidcraft-kernel::boolean_props">"#,
            ),
            skipped(),
        ] {
            let error = grade(&exceptions(), &outcomes(&xml), before_expiry()).unwrap_err();
            assert!(error.contains("MISSING"), "{error}");
            assert!(error.contains("translation_invariant"), "{error}");
        }
    }

    #[test]
    fn a_different_signature_or_input_does_not_match() {
        let wrong_signature = REPORT.replace("Bad NonMonotonic CPts", "Some Other Failure");
        let error = grade(&exceptions(), &outcomes(&wrong_signature), before_expiry()).unwrap_err();
        assert!(error.contains("UNMATCHED"), "{error}");
        assert!(error.contains("signature"), "{error}");

        let wrong_input = REPORT.replace("off = 100000.0", "off = 1000.0");
        let error = grade(&exceptions(), &outcomes(&wrong_input), before_expiry()).unwrap_err();
        assert!(error.contains("UNMATCHED"), "{error}");
        assert!(error.contains("input marker"), "{error}");
    }

    #[test]
    fn an_excused_test_that_passed_must_leave_the_list() {
        let error = grade(&exceptions(), &outcomes(&without_failure()), before_expiry()).unwrap_err();
        assert!(error.contains("FIXED"), "{error}");
        assert!(error.contains("remove its line"), "{error}");
    }

    #[test]
    fn raw_nextest_exit_must_agree_with_the_junit_report() {
        let failed = outcomes(REPORT);
        assert!(check_raw_exit(Some(100), &failed).is_ok());
        assert!(check_raw_exit(None, &failed).is_ok());
        assert!(check_raw_exit(Some(0), &failed).is_err());
        assert!(check_raw_exit(Some(102), &failed).unwrap_err().contains("infrastructure"));

        let passed = outcomes(&without_failure());
        assert!(check_raw_exit(Some(0), &passed).is_ok());
        assert!(check_raw_exit(Some(100), &passed).is_err());
    }

    #[test]
    fn malformed_lists_fail() {
        for (list, expected) in [
            (
                "b::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\na::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\n",
                "out of order",
            ),
            (
                "a::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\na::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\n",
                "a repeat of",
            ),
            ("a::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i\n", "missing `why`"),
            ("a::x | issue=#1 | owner= | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\n", "`owner` is empty"),
            ("a::x | issue=1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\n", "not `#N`"),
            ("a::x | issue=#1 | owner=o | expires=2026-02-30 | seed=s | signature=s | input=i | why=w\n", "day is out of range"),
            ("a::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i; | why=w\n", "empty marker"),
            ("a::x | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w | extra=x\n", "unknown field `extra`"),
            ("not a nextest id | issue=#1 | owner=o | expires=2026-12-08 | seed=s | signature=s | input=i | why=w\n", "not a nextest id"),
        ] {
            let error = parse_exceptions(list).unwrap_err();
            assert!(error.contains(expected), "expected `{expected}` in `{error}`");
        }
    }

    #[test]
    fn dates_are_validated_and_ordered() {
        assert!(parse_date("2026-12-08").unwrap() < parse_date("2026-12-09").unwrap());
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("2026-02-29").is_err());
        assert!(parse_date("2026-13-01").is_err());
        assert!(parse_date("2026-1-1").is_err());
        assert!(parse_date("2026-01-01-1").is_err());
    }

    #[test]
    fn a_test_failing_in_any_report_failed() {
        let passed = without_failure();
        let mut all = BTreeMap::new();
        super::merge(&mut all, &passed).unwrap();
        assert_eq!(all.get("solidcraft-kernel::boolean_props::translation_invariant").map(|outcome| outcome.failed), Some(false));
        super::merge(&mut all, REPORT).unwrap();
        assert_eq!(all.get("solidcraft-kernel::boolean_props::translation_invariant").map(|outcome| outcome.failed), Some(true));
    }
}
