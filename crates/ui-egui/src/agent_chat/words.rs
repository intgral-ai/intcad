//! Whether the words of a next-step suggestion give a hole the size its commands would make (the
//! list's rules 9 and 10, docs/design/autocomplete-tests.md). The words are what the user decides
//! by, and the undo step's name afterwards, so a suggestion whose words say another size than its
//! ghost is withdrawn before any of its commands runs.
//!
//! Only holes are checked: `solid.hole`, and a `solid.cylinder` that cuts. A length is a radius when
//! R or the app's word for radius stands next to it; any other is a diameter. A length said as neither
//! is taken as the hole's only when it is near a drawn hole's size, or twice or half of it ("a hole
//! 10 mm from each edge" is not about its size).

use serde_json::Value;
use solidcraft_engine::doc::expr::{Defaults, Kind, eval_with_defaults, is_literal, unit_info};
use solidcraft_engine::doc::{DocError, Document, Operation};

use super::ghost::{PLACES, SIZES, Step};
use crate::i18n::tr;

/// What the words say a length is of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Of {
    Radius,
    Diameter,
    Unsaid,
}

/// A length in the words: where it is (characters), in millimetres, and half of its last digit.
#[derive(Clone, Debug, PartialEq)]
struct Length {
    start: usize,
    end: usize,
    mm: f64,
    half: f64,
    of: Of,
}

impl Length {
    /// It is `size` (mm), or that rounded to the digits written and near it.
    fn is(&self, size: f64) -> bool {
        let twice = if self.of == Of::Radius { 2.0 } else { 1.0 };
        let (mm, half) = (twice * self.mm, twice * self.half);
        same(mm, size) || (mm - size).abs() <= half.min(ROUNDED * size) + 1e-9 * size
    }
}

/// The most a rounded size may be off, of the size.
const ROUNDED: f64 = 0.005;

/// How far, of it, a length said as neither radius nor diameter may be from a drawn hole's size, or
/// from twice or half of it, to be taken as said of the hole.
const NEAR: f64 = 0.05;

/// How the questions speak of a unit that is not the millimetre: its name, a small size, a size.
pub(super) fn spoken(unit: &str) -> (&str, &'static str, &'static str) {
    match unit {
        "in" | "inch" => ("inches", "0.08 in", "0.25 in"),
        "cm" => ("centimetres", "0.2 cm", "0.5 cm"),
        "m" => ("metres", "0.002 m", "0.005 m"),
        "ft" => ("feet", "0.007 ft", "0.02 ft"),
        _ => (unit, "2.0 mm", "5 mm"),
    }
}

/// Units as they are written, in the scripts of the app's languages, with their millimetres.
const UNITS: &[(&str, f64)] = &[
    ("millimeters", 1.0),
    ("millimetres", 1.0),
    ("millimeter", 1.0),
    ("millimetre", 1.0),
    ("mm", 1.0),
    ("centimeters", 10.0),
    ("centimetres", 10.0),
    ("centimeter", 10.0),
    ("centimetre", 10.0),
    ("cm", 10.0),
    ("meters", 1000.0),
    ("metres", 1000.0),
    ("m", 1000.0),
    ("feet", 304.8),
    ("foot", 304.8),
    ("ft", 304.8),
    ("inches", 25.4),
    ("inch", 25.4),
    ("in", 25.4),
    ("zoll", 25.4),
    ("″", 25.4),
    ("\"", 25.4),
    ("”", 25.4),
    ("英寸", 25.4),
    ("インチ", 25.4),
    ("毫米", 1.0),
    ("公釐", 1.0),
    ("厘米", 10.0),
    ("公分", 10.0),
    ("ミリ", 1.0),
    ("밀리", 1.0),
    ("мм", 1.0),
    ("см", 10.0),
    ("მმ", 1.0),
    ("مم", 1.0),
    ("ملم", 1.0),
    ("מ\"מ", 1.0),
    ("מ״מ", 1.0),
];

/// Units that are something else too ("Hole 5 in the corner", a name in quotes): taken after a whole
/// number in an inch design only.
const DOUBTFUL: [&str; 3] = ["in", "\"", "”"];

/// Signs that stand before a diameter.
const DIAMETER_SIGNS: [char; 5] = ['⌀', 'ø', '∅', 'φ', 'ϕ'];

/// Short words that may stand between a length and what it is of ("radius of 5 mm", "5 mm de rayon").
const LINKS: &[&str] = &["of", "in", "im", "de", "di", "da", "do", "von", "van", "af", "av", "på", "של", "の", "は", "が", "为", "為", "是", "的"];

/// The words `step` give a hole another size than `steps` do.
pub fn differ(doc: &Document, step: &str, steps: &[Step]) -> bool {
    let said = lengths(step, doc);
    if said.is_empty() {
        return false;
    }
    let Some(holes) = holes(doc, steps).filter(|h| !h.bores.is_empty()) else { return false };
    // A depth or a place the words give is not the hole's size, and nor is a length far from it.
    let others = other_numbers(doc, steps);
    let near = |l: &Length| holes.bores.iter().flat_map(|b| [*b, 2.0 * b, 0.5 * b]).any(|s| (l.mm - s).abs() <= NEAR * s);
    let is_size = |l: &&Length| match l.of {
        Of::Unsaid => !others.iter().any(|o| same(o.abs(), l.mm)) && near(l),
        Of::Diameter => !holes.wide.iter().any(|d| l.is(*d)),
        Of::Radius => true,
    };
    let mut sizes = said.iter().filter(is_size).peekable();
    let drawn = |l: &Length| holes.bores.iter().any(|b| l.is(*b));
    sizes.peek().is_some() && !sizes.any(drawn)
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(1.0)
}

/// The diameters (mm) of the holes some commands make, and of their counterbores and countersinks.
struct Holes {
    bores: Vec<f64>,
    wide: Vec<f64>,
}

/// The holes `steps` make. `None`: one gives its size in a way that is not read here (a thread, a
/// bolt's clearance, an expression that cannot be worked out).
fn holes(doc: &Document, steps: &[Step]) -> Option<Holes> {
    let mut out = Holes { bores: Vec::new(), wide: Vec::new() };
    for (command, params) in steps {
        // As the engine does, nothing given reads as no size.
        let given = |key: &str| params.get(key).filter(|v| v.is_number() || v.as_str().is_some_and(|s| !s.trim().is_empty()));
        if command == "solid.hole" {
            out.bores.push(length(doc, given("diameter")?)?);
            out.wide.extend(["cb_diameter", "cs_diameter"].into_iter().filter_map(|k| length(doc, given(k)?)));
        } else if cuts(command, params) {
            // The engine's order: a radius given is the one that counts.
            out.bores.push(match given("radius") {
                Some(r) => 2.0 * length(doc, r)?,
                None => length(doc, given("diameter")?)?,
            });
        }
    }
    Some(out)
}

/// A cylinder taken out of the bodies there.
fn cuts(command: &str, params: &Value) -> bool {
    command == "solid.cylinder" && params.get("operation").and_then(Value::as_str).and_then(Operation::parse) == Some(Operation::Cut)
}

/// The lengths (mm) among the sizes and places of `steps` that are not a hole's size.
fn other_numbers(doc: &Document, steps: &[Step]) -> Vec<f64> {
    fn point(doc: &Document, v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Array(a) => a.iter().for_each(|v| point(doc, v, out)),
            Value::Object(o) => o.values().for_each(|v| point(doc, v, out)),
            // A point's bare numbers are millimetres.
            Value::Number(n) => out.extend(n.as_f64()),
            Value::String(s) => out.extend(doc.eval(s, Kind::Length).ok()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (command, params) in steps {
        let hole = command == "solid.hole" || cuts(command, params);
        for (key, v) in params.as_object().into_iter().flatten() {
            let key = key.as_str();
            if PLACES.contains(&key) {
                point(doc, v, &mut out);
            } else if SIZES.contains(&key) && !key.contains("angle") && !(hole && matches!(key, "radius" | "diameter")) {
                out.extend(length(doc, v));
            }
        }
    }
    out
}

/// A length as a command reads it: a bare number is in the design's units.
fn length(doc: &Document, v: &Value) -> Option<f64> {
    let text = match v {
        Value::Number(n) => n.as_f64()?.to_string(),
        Value::String(s) => s.trim().to_string(),
        _ => return None,
    };
    millimetres(doc, &doc.with_design_unit(&text, Kind::Length))
}

/// A length of the design as the geometry reads it: one worked out from unit-less parameters is in
/// the design's units.
pub(super) fn millimetres(doc: &Document, e: &str) -> Option<f64> {
    if is_literal(e) { doc.eval(e, Kind::Length).ok() } else { read_in(doc, e, design_unit(doc)) }
}

/// A length whose bare numbers are in a unit of `len` millimetres.
fn read_in(doc: &Document, e: &str, len: f64) -> Option<f64> {
    let (values, _) = doc.param_values();
    let defaults = Defaults { len, ..Defaults::default() };
    let look = |n: &str| values.get(n).copied().ok_or_else(|| DocError::Expr(format!("unknown parameter `{n}`")));
    eval_with_defaults(e, &look, defaults).and_then(|v| v.to_kind_in(Kind::Length, defaults)).ok()
}

/// In a design that is not in mm: something in this length has no unit, so what it comes to depends
/// on the unit it is read in ("5", "2*3", "101.6 - 0.5 in", a parameter that has none).
pub(super) fn lacks_unit(doc: &Document, e: &str) -> bool {
    let (mm, design) = (read_in(doc, e, 1.0), read_in(doc, e, design_unit(doc)));
    mm.zip(design).is_some_and(|(a, b)| !same(a, b))
}

/// The millimetres of the design's unit.
pub(super) fn design_unit(doc: &Document) -> f64 {
    other_unit(doc).and_then(unit_info).map_or(1.0, |(_, mm)| mm)
}

/// The design's unit, when it is not the millimetre.
pub(super) fn other_unit(doc: &Document) -> Option<&str> {
    let unit = doc.units.trim();
    (unit != "mm" && unit_info(unit).is_some_and(|(k, _)| k == Kind::Length)).then_some(unit)
}

/// The lengths (mm) a text gives, for the numbers of a suggestion to be found in: one with a unit, and
/// one without in each of the units of `bare` (mm).
pub(super) fn known(text: &str, bare: &[f64]) -> Vec<f64> {
    let t = lower(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if !t[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let n = number(&t, i);
        let with = unit(&t, n.end, false);
        // "5 in" may be no length ("5 in a row"): then both readings are known.
        let sure = unit(&t, n.end, n.decimals == Some(0)).is_some();
        out.extend(with.map(|(mm, _)| n.value * mm));
        if !sure {
            let point = i > 0 && t[i - 1] == '.' && n.value < 1.0;
            let parts = t[i..n.end].split(|c| !c.is_ascii_digit() && *c != '.').filter(|_| !point);
            let parts = parts.filter_map(|s| s.iter().collect::<String>().parse::<f64>().ok());
            out.extend(parts.chain([n.value]).flat_map(|v| bare.iter().map(move |mm| v * mm)));
        }
        i = with.filter(|_| sure).map_or(n.end, |(_, end)| end);
    }
    out.retain(|v| v.is_finite());
    out
}

/// One character for one, so that places in the text stay where they are.
fn lower(text: &str) -> Vec<char> {
    text.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// A character that makes the one beside it part of a word or a name. Not the scripts that are
/// written without spaces.
fn in_word(c: char) -> bool {
    c == '_' || (c.is_alphanumeric() && c < '\u{1100}')
}

fn space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{a0}' | '\u{202f}')
}

/// The lengths in `text`, a suggestion's words for `doc`, in the app's language.
fn lengths(text: &str, doc: &Document) -> Vec<Length> {
    let t = lower(text);
    let mut found = numbers(&t, design_unit(doc));
    // "Yarıçap" (radius) ends in "çap" (diameter): a radius is read first.
    let radii = places(&t, &lower(tr("ui.radius")));
    let diameters = places(&t, &lower(tr("ui.diameter"))).into_iter().filter(|(s, e)| !radii.iter().any(|(a, b)| s < b && a < e));
    let mut words: Vec<(usize, usize, Of)> = diameters.map(|(s, e)| (s, e, Of::Diameter)).collect();
    words.extend(radii.iter().map(|(s, e)| (*s, *e, Of::Radius)));
    words.sort_by_key(|w| w.0);
    // Each is said of one length: the one before it, else the one after.
    for (start, end, of) in words {
        let free = |l: &&mut Length| l.of == Of::Unsaid;
        let before = found.iter_mut().filter(free).find(|l| l.end <= start && joined(&t[l.end..start]));
        let next = match before {
            Some(l) => Some(l),
            None => found.iter_mut().filter(free).find(|l| l.start >= end && joined(&t[end..l.start])),
        };
        if let Some(l) = next {
            l.of = of;
        }
    }
    found
}

/// Nothing stands between a length and a word but spaces, a hyphen, a sign, or a short link word.
fn joined(gap: &[char]) -> bool {
    let between: String = gap.iter().map(|c| if space(*c) || matches!(c, '-' | ':' | '=' | '(') { ' ' } else { *c }).collect();
    let mut words = between.split_whitespace();
    words.next().is_none_or(|w| LINKS.contains(&w)) && words.next().is_none()
}

/// Where `word` stands in `t`, with the ending it has there ("радиусом", "yarıçapı").
fn places(t: &[char], word: &[char]) -> Vec<(usize, usize)> {
    if word.is_empty() || t.len() < word.len() {
        return Vec::new();
    }
    let ending = |c: &&char| c.is_alphabetic() && (in_word(**c) || ('\u{ac00}'..='\u{d7a3}').contains(*c));
    (0..=t.len() - word.len())
        .filter(|i| t[*i..].starts_with(word))
        .map(|i| (i, i + word.len() + t[i + word.len()..].iter().take_while(ending).count()))
        .collect()
}

/// The numbers of `t` that are lengths: with a unit, or after R or a diameter sign (`design`: the
/// millimetres of the design's unit, which a bare one is in).
fn numbers(t: &[char], design: f64) -> Vec<Length> {
    let digit = |i: usize| t.get(i).is_some_and(char::is_ascii_digit);
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if !digit(i) {
            i += 1;
            continue;
        }
        let start = i;
        let n = number(t, start);
        let end = n.end;
        i = end;
        let mut p = start;
        while p > 0 && space(t[p - 1]) {
            p -= 1;
        }
        let sign = p.checked_sub(1).map(|p| t[p]);
        let alone = p < 2 || !in_word(t[p - 2]);
        let of = match sign {
            Some('r') if alone => Of::Radius,
            Some(c) if DIAMETER_SIGNS.contains(&c) && (c != 'ø' || alone) => Of::Diameter,
            // The digits of a name ("Hole1"), but for the 5 of "4x5 mm".
            Some(c) if p == start && in_word(c) && !(c == 'x' && p >= 2 && (t[p - 2].is_ascii_digit() || space(t[p - 2]))) => continue,
            _ => Of::Unsaid,
        };
        let half = n.decimals.map_or(0.0, |d| 0.5 * 0.1f64.powi(d));
        match unit(t, end, n.decimals == Some(0) && !same(design, 25.4)) {
            Some((mm, unit_end)) => out.push(Length { start, end: unit_end, mm: n.value * mm, half: half * mm, of }),
            None if of != Of::Unsaid => out.push(Length { start, end, mm: n.value * design, half: half * design, of }),
            None => {}
        }
        i = out.last().filter(|l| l.start == start).map_or(end, |l| l.end);
    }
    out
}

/// A number as it is written.
struct Number {
    value: f64,
    /// The digits after its point (`None`: it is a fraction).
    decimals: Option<i32>,
    end: usize,
}

/// The number that starts at `i`: with a decimal comma or point, or a fraction as "1/4" or "1 1/4".
fn number(t: &[char], i: usize) -> Number {
    let digit = |i: usize| t.get(i).is_some_and(char::is_ascii_digit);
    let digits = |mut i: usize| {
        while digit(i) {
            i += 1;
        }
        i
    };
    let read = |a: usize, b: usize| t[a..b].iter().collect::<String>().parse::<f64>().unwrap_or(0.0);
    let places = |from: usize, to: usize| Some(i32::try_from(to - from).unwrap_or(0));
    let end = digits(i);
    // ".25"
    if i > 0 && t[i - 1] == '.' && (i < 2 || !in_word(t[i - 2])) {
        let text: String = t[i..end].iter().collect();
        return Number { value: format!("0.{text}").parse().unwrap_or(0.0), decimals: places(i, end), end };
    }
    // A fraction `a/b` whose first digit is at `i`, and where it ends.
    let fraction = |i: usize| {
        let a = digits(i);
        let b = if a > i && t.get(a) == Some(&'/') { digits(a + 1) } else { a };
        (b > a + 1 && read(a + 1, b) > 0.0).then(|| (read(i, a) / read(a + 1, b), b))
    };
    if let Some((value, end)) = fraction(i) {
        return Number { value, decimals: None, end };
    }
    if t.get(end).is_some_and(|c| space(*c) || *c == '-')
        && let Some((v, end_of)) = fraction(end + 1)
    {
        return Number { value: read(i, end) + v, decimals: None, end: end_of };
    }
    if matches!(t.get(end), Some('.' | ',')) && digit(end + 1) {
        let after = digits(end + 1);
        let text: String = t[i..after].iter().map(|c| if *c == ',' { '.' } else { *c }).collect();
        return Number { value: text.parse().unwrap_or(0.0), decimals: places(end + 1, after), end: after };
    }
    Number { value: read(i, end), decimals: Some(0), end }
}

/// The unit at `i`, after a space or a hyphen, and where it ends; with `sure`, not a doubtful one.
fn unit(t: &[char], i: usize, sure: bool) -> Option<(f64, usize)> {
    let mut at = i;
    while t.get(at).is_some_and(|c| space(*c)) {
        at += 1;
    }
    if t.get(at) == Some(&'-') {
        at += 1;
    }
    UNITS.iter().filter(|(name, _)| !(sure && DOUBTFUL.contains(name))).find_map(|(name, mm)| {
        let name: Vec<char> = name.chars().collect();
        let end = at + name.len();
        (t.get(at..end)? == name.as_slice() && !t.get(end).is_some_and(|c| in_word(*c))).then_some((*mm, end))
    })
}
