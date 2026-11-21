//! How an answer about a stress estimate is worded (PIL-294, decision 25): the words that read as
//! a verdict on the part, and whether an answer gives the stress as a share of the yield strength.
//! One list, for the engine's own wording tests and for the chat, which reads every agent's
//! answer after an estimate and warns when it misses (#201).
//!
//! False alarms cost more than misses: a warning under a good answer teaches the user to ignore
//! it. So English words match whole words only, a few harmless uses are let through
//! (`HARMLESS_AFTER`, `HARMLESS_BEFORE`, an opening "OK,"), and 通过 counts only where it means
//! "passes" (`chinese_pass`).

use std::cmp::Reverse;

/// The longest text read; the rest of an answer is not checked.
const MAX_TEXT: usize = 64 * 1024;

/// Words and phrases that read as a verdict on the part; `how_to_report` forbids them. English
/// ones match whole words in any case, a phrase's words split by spaces or hyphens. Chinese ones
/// match anywhere, the longest first (不安全 is not also 安全), except 通过 (`chinese_pass`).
pub const VERDICT_WORDS: &[&str] = &[
    "safe",
    "unsafe",
    "pass",
    "passes",
    "passed",
    "passing",
    "fail",
    "fails",
    "failed",
    "failing",
    "OK",
    "okay",
    "margin",
    "margins",
    "safety factor",
    "safety factors",
    "factor of safety",
    "factors of safety",
    "FoS",
    "verdict",
    "verdicts",
    "approved",
    "acceptable",
    "unacceptable",
    "certify",
    "certified",
    "compliant with",
    "complies with",
    "strong enough",
    "too weak",
    "安全系数",
    "安全裕度",
    "不安全",
    "安全",
    "裕度",
    "不合格",
    "合格",
    "达标",
    "不通过",
    "未通过",
    "没通过",
    "没有通过",
    "通过",
];

/// Uses of an English verdict word that judge nothing, by the words right after it: a load
/// passes through a face, a stress passes (goes past) yield, a program failed to mesh.
const HARMLESS_AFTER: &[(&[&str], &[&str])] = &[
    (&["pass", "passes", "passed", "passing"], &["through", "into", "along", "across", "around", "via", "yield", "the yield"]),
    (&["fail", "fails", "failed"], &["to"]),
    (&["safe"], &["to say", "to assume"]),
    (&["OK", "okay"], &["with you"]),
];

/// The same by the words right before it: a question to the user ("is that OK?").
const HARMLESS_BEFORE: &[(&[&str], &[&str])] = &[(&["OK", "okay"], &["is that", "is this", "would that be", "if that is", "if that s"])];

/// Chinese uses that judge nothing: 为安全起见 ("to be on the safe side").
const HARMLESS_CHINESE: &[&str] = &["安全起见"];

/// The verdict words in `text`, each once, in the order they first appear.
pub fn verdict_words(text: &str) -> Vec<&'static str> {
    let text = capped(text);
    let mut found = english(text);
    found.extend(chinese(text));
    found.sort_by_key(|(at, _)| *at);
    let mut words: Vec<&'static str> = Vec::new();
    for (_, w) in found {
        if !words.contains(&w) {
            words.push(w);
        }
    }
    words
}

/// Whether `text` gives a stress as a share of the yield strength: a percentage in the same
/// sentence (or table row) as "yield" or 屈服 ("about 55 % of the yield strength", "屈服强度的
/// 55%"). A ratio given another way (0.55 of yield, half of yield) does not count.
pub fn gives_share_of_yield(text: &str) -> bool {
    sentences(capped(text)).into_iter().any(|s| mentions_yield(s) && has_percent(s))
}

/// `text`, cut to [`MAX_TEXT`] bytes on a character boundary.
fn capped(text: &str) -> &str {
    if text.len() <= MAX_TEXT {
        return text;
    }
    let mut cut = MAX_TEXT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.get(..cut).unwrap_or_default()
}

/// A run of ASCII letters, digits or underscores (so `yield_strength` is one word), by its bytes.
#[derive(Clone, Copy)]
struct Word {
    start: usize,
    end: usize,
}

fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let inside = c.is_ascii_alphanumeric() || c == '_';
        match (inside, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push(Word { start: s, end: i });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(Word { start: s, end: text.len() });
    }
    out
}

fn word(text: &str, w: Word) -> &str {
    text.get(w.start..w.end).unwrap_or_default()
}

/// Whether two words are one phrase's: only spaces, hyphens or an apostrophe between them.
fn joined(text: &str, a: Word, b: Word) -> bool {
    text.get(a.end..b.start).is_some_and(|gap| !gap.is_empty() && gap.chars().all(|c| c.is_whitespace() || matches!(c, '-' | '\'' | '’')))
}

/// Whether the words from `at` read `phrase` (its words split by spaces): the index past it.
fn reads(text: &str, words: &[Word], at: usize, phrase: &str) -> Option<usize> {
    let mut i = at;
    let mut prev: Option<Word> = None;
    for part in phrase.split(' ') {
        let w = *words.get(i)?;
        if !word(text, w).eq_ignore_ascii_case(part) {
            return None;
        }
        if let Some(p) = prev
            && !joined(text, p, w)
        {
            return None;
        }
        prev = Some(w);
        i += 1;
    }
    Some(i)
}

/// Whether the verdict word `entry`, read from word `at` to `end`, is one of its harmless uses.
fn harmless(text: &str, words: &[Word], at: usize, end: usize, entry: &str) -> bool {
    let next_joined = |phrase: &str| {
        let (Some(last), Some(next)) = (end.checked_sub(1).and_then(|i| words.get(i)), words.get(end)) else { return false };
        joined(text, *last, *next) && reads(text, words, end, phrase).is_some()
    };
    if HARMLESS_AFTER.iter().filter(|(w, _)| w.contains(&entry)).flat_map(|(_, next)| next.iter().copied()).any(next_joined) {
        return true;
    }
    let prev_joined = |phrase: &str| {
        let n = phrase.split(' ').count();
        let (Some(from), Some(last), Some(first)) = (at.checked_sub(n), at.checked_sub(1).and_then(|i| words.get(i)), words.get(at)) else {
            return false;
        };
        reads(text, words, from, phrase) == Some(at) && joined(text, *last, *first)
    };
    if HARMLESS_BEFORE.iter().filter(|(w, _)| w.contains(&entry)).flat_map(|(_, prev)| prev.iter().copied()).any(prev_joined) {
        return true;
    }
    // "OK, here is the estimate": an opening, not a judgement.
    matches!(entry, "OK" | "okay")
        && words.get(at).is_some_and(|w| {
            let before = text.get(..w.start).unwrap_or_default().trim_end_matches([' ', '\t']);
            let starts = before.is_empty() || before.ends_with(['.', '!', '?', ':', '\n', '。', '！', '？']);
            starts && text.get(w.end..).is_some_and(|a| a.starts_with(','))
        })
}

/// The English verdict words in `text`, with where each sits.
fn english(text: &str) -> Vec<(usize, &'static str)> {
    let words = words(text);
    let mut used = vec![false; words.len()];
    let mut entries: Vec<&'static str> = VERDICT_WORDS.iter().copied().filter(|e| e.is_ascii()).collect();
    entries.sort_by_key(|e| Reverse(e.split(' ').count()));
    let mut found = Vec::new();
    for entry in entries {
        let mut i = 0;
        while i < words.len() {
            match reads(text, &words, i, entry) {
                Some(end) if used.get(i..end).is_some_and(|u| !u.contains(&true)) && !harmless(text, &words, i, end, entry) => {
                    for u in used.iter_mut().take(end).skip(i) {
                        *u = true;
                    }
                    found.push((words.get(i).map_or(0, |w| w.start), entry));
                    i = end;
                }
                _ => i += 1,
            }
        }
    }
    found
}

/// The Chinese verdict words in `text`, with where each sits.
fn chinese(text: &str) -> Vec<(usize, &'static str)> {
    let mut taken = vec![false; text.len()];
    let mut take = |from: usize, to: usize| {
        let Some(span) = taken.get_mut(from..to) else { return false };
        if span.contains(&true) {
            return false;
        }
        span.fill(true);
        true
    };
    for h in HARMLESS_CHINESE {
        for (i, m) in text.match_indices(h) {
            take(i, i + m.len());
        }
    }
    let mut entries: Vec<&'static str> = VERDICT_WORDS.iter().copied().filter(|e| !e.is_ascii()).collect();
    entries.sort_by_key(|e| Reverse(e.chars().count()));
    let mut found = Vec::new();
    for entry in entries {
        for (i, m) in text.match_indices(entry) {
            let end = i + m.len();
            if entry == "通过" && !chinese_pass(text, end) {
                continue;
            }
            if take(i, end) {
                found.push((i, entry));
            }
        }
    }
    found
}

/// Whether the 通过 ending at byte `end` means "passes": nothing it acts on follows (校核通过。,
/// 已通过, 通过了). Followed by a word, it means "through" or "by way of" (通过网格检查, 通过加厚
/// 底板, 通过 3 次加密), and judges nothing. So 通过强度校核 ("passes the strength check") is
/// missed: the same words as "by way of the strength check".
fn chinese_pass(text: &str, end: usize) -> bool {
    match text.get(end..).unwrap_or_default().trim_start().chars().next() {
        None => true,
        Some(c) if "了的吗呢啦啊".contains(c) => true,
        Some(c) => !c.is_alphanumeric(),
    }
}

/// `text` cut into sentences and lines; a full stop counts only before a space or the end, so
/// "1.4" stays whole.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|(_, n)| *n);
        let ends = matches!(c, '。' | '！' | '？' | '；' | '!' | '?' | ';' | '\n') || (c == '.' && next.is_none_or(char::is_whitespace));
        if ends {
            let to = i + c.len_utf8();
            out.push(text.get(from..to).unwrap_or_default());
            from = to;
        }
    }
    out.push(text.get(from..).unwrap_or_default());
    out
}

fn mentions_yield(s: &str) -> bool {
    s.contains("屈服") || words(s).into_iter().any(|w| word(s, w).get(..5).is_some_and(|p| p.eq_ignore_ascii_case("yield")))
}

/// A number followed by %, ％, "percent" or "per cent"; or 百分之.
fn has_percent(s: &str) -> bool {
    if s.contains("百分之") {
        return true;
    }
    let sign = s.char_indices().filter(|(_, c)| matches!(c, '%' | '％')).any(|(i, _)| {
        s.get(..i).unwrap_or_default().trim_end().chars().next_back().is_some_and(|d| d.is_ascii_digit() || ('０'..='９').contains(&d))
    });
    if sign {
        return true;
    }
    let ws = words(s);
    ws.iter().enumerate().any(|(i, w)| {
        word(s, *w).bytes().all(|b| b.is_ascii_digit()) && (reads(s, &ws, i + 1, "percent").is_some() || reads(s, &ws, i + 1, "per cent").is_some())
    })
}

#[cfg(test)]
#[path = "report_words_tests.rs"]
mod tests;
