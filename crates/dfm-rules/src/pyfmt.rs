//! CPython's `%g` number spelling, for rule messages copied from INTdfm.
//!
//! Rust's `Display` and `Debug` for `f64` do not match Python: `Display` never uses an exponent,
//! and `{:e}` writes `1e6` where Python writes `1e+06`. The rule messages are part of the report
//! contract, so a number in one has to be spelled the way CPython spells it.
//!
//! `%g` keeps `precision` significant digits, drops trailing zeros, and switches to exponent form
//! when the decimal exponent is below -4 or at least the precision. `g` is the default six digits;
//! `g3` and `g4` are the `.3g` and `.4g` INTdfm uses for a moved distance and a bend's angle.

/// `format(value, "g")`: up to six significant digits, trailing zeros dropped.
pub fn g(value: f64) -> String {
    g_prec(value, 6)
}

/// `format(value, ".3g")`.
pub fn g3(value: f64) -> String {
    g_prec(value, 3)
}

/// `format(value, ".4g")`.
pub fn g4(value: f64) -> String {
    g_prec(value, 4)
}

/// CPython's `round(value, places) + 0.0`, for a coordinate a message carries.
pub fn round(value: f64, places: usize) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let text = format!("{value:.places$}");
    match text.parse::<f64>() {
        Ok(rounded) => rounded + 0.0,
        Err(_) => value,
    }
}

/// `format(value, ".<precision>g")`.
pub fn g_prec(value: f64, precision: usize) -> String {
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
    // Rust rounds the exact binary value to the requested significant digits, as CPython does.
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

    /// Pinned from CPython: `format(value, ".6g")` and `format(value, ".3g")`.
    #[test]
    fn g_matches_python() {
        let cases: &[(f64, &str, &str)] = &[
            (0.0, "0", "0"),
            (-0.0, "-0", "-0"),
            (1.5, "1.5", "1.5"),
            (90.0, "90", "90"),
            (92.5, "92.5", "92.5"),
            (1500.0, "1500", "1.5e+03"),
            (1_234_567.0, "1.23457e+06", "1.23e+06"),
            (2_835.290_358_249_673, "2835.29", "2.84e+03"),
            (0.0001, "0.0001", "0.0001"),
            (0.00001, "1e-05", "1e-05"),
            (0.000001, "1e-06", "1e-06"),
            (0.000_046, "4.6e-05", "4.6e-05"),
            (0.05, "0.05", "0.05"),
            (1.0499999999999998, "1.05", "1.05"),
            (0.19999999999999996, "0.2", "0.2"),
            (123.456, "123.456", "123"),
            (1234.0, "1234", "1.23e+03"),
            (1_000_000.0, "1e+06", "1e+06"),
            (999_999.5, "1e+06", "1e+06"),
            (0.15625, "0.15625", "0.156"),
        ];
        for (value, six, three) in cases {
            assert_eq!(g(*value), *six, "{value} as %g");
            assert_eq!(g3(*value), *three, "{value} as .3g");
        }
        assert_eq!(g4(92.12345), "92.12");
        assert_eq!(round(2.000_04, 3), 2.0);
        assert_eq!(round(-0.000_4, 3), 0.0);
        assert_eq!(g(round(1.318_019_484_660_535_8, 3)), "1.318");
        assert_eq!(g4(90.0), "90");
        assert_eq!(g(f64::NAN), "nan");
        assert_eq!(g(f64::INFINITY), "inf");
    }
}
