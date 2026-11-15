//! RFC 8785 JSON Canonicalization Scheme for validation payloads and profiles.
//!
//! Candidate manifests are integers only, but a check profile carries expected values and
//! tolerances, so this encoder also canonicalizes finite floating-point numbers the way
//! ECMAScript writes them (the number format RFC 8785 requires). It is deliberately separate
//! from `solidcraft_io::candidate::canonical_json`, whose narrower contract refuses floats.

use serde_json::Value;

use super::ValidationError;

/// Canonical bytes of `value`, or `INVALID_REQUEST` when it holds a non-finite or malformed
/// number.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>, ValidationError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out.into_bytes())
}

fn invalid(message: impl std::fmt::Display) -> ValidationError {
    ValidationError::invalid_request(message.to_string())
}

fn write_value(out: &mut String, value: &Value) -> Result<(), ValidationError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number_text(number)?),
        Value::String(text) => {
            let escaped = serde_json::to_string(text).map_err(|e| invalid(format!("JCS string: {e}")))?;
            out.push_str(&escaped);
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            // RFC 8785 orders object keys by UTF-16 code units.
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, &Value::String(key.clone()))?;
                out.push(':');
                if let Some(item) = object.get(key) {
                    write_value(out, item)?;
                }
            }
            out.push('}');
        }
    }
    Ok(())
}

fn number_text(number: &serde_json::Number) -> Result<String, ValidationError> {
    if let Some(integer) = number.as_i64() {
        return Ok(integer.to_string());
    }
    if let Some(integer) = number.as_u64() {
        return Ok(integer.to_string());
    }
    let value = number.as_f64().ok_or_else(|| invalid("JCS: number is not representable"))?;
    canonical_f64(value)
}

/// ECMAScript `Number.prototype.toString` for a finite `f64`, which RFC 8785 uses.
fn canonical_f64(value: f64) -> Result<String, ValidationError> {
    if !value.is_finite() {
        return Err(invalid("JCS: numbers must be finite"));
    }
    if value == 0.0 {
        return Ok("0".into());
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let magnitude = value.abs();
    // Rust's LowerExp prints the shortest round-tripping significant digits, which is the
    // digit sequence ECMAScript starts from.
    let text = format!("{magnitude:e}");
    let (mantissa, exponent) = text.split_once('e').ok_or_else(|| invalid("JCS: malformed number"))?;
    let exponent: i32 = exponent.parse().map_err(|_| invalid("JCS: malformed exponent"))?;
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        return Ok("0".into());
    }
    let k = i32::try_from(digits.len()).map_err(|_| invalid("JCS: number is too long"))?;
    let n = exponent.saturating_add(1);
    let mut out = String::from(sign);
    if k <= n && n <= 21 {
        out.push_str(&digits);
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        let split = usize::try_from(n).map_err(|_| invalid("JCS: malformed number"))?;
        if split <= digits.len() {
            out.push_str(&digits[..split]);
            out.push('.');
            out.push_str(&digits[split..]);
        } else {
            out.push_str(&digits);
        }
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        let e = n - 1;
        if e >= 0 {
            out.push('+');
        }
        out.push_str(&e.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_follow_the_ecmascript_form_used_by_jcs() {
        let cases = [
            (json!(0), "0"),
            (json!(-0.0), "0"),
            (json!(1), "1"),
            (json!(1.5), "1.5"),
            (json!(20.0), "20"),
            (json!(0.000001), "0.000001"),
            (json!(0.0000001), "1e-7"),
            (json!(100000000000000000000.0), "100000000000000000000"),
            (json!(1e21), "1e+21"),
            (json!(-1.25), "-1.25"),
        ];
        for (value, expected) in cases {
            assert_eq!(String::from_utf8(canonical_json(&value).unwrap()).unwrap(), expected, "{value}");
        }
        assert!(super::canonical_f64(f64::NAN).is_err());
    }

    #[test]
    fn objects_sort_by_utf16_code_units() {
        let value = json!({"b": 2, "a": [true, null, "é"], "nested": {"z": 0, "y": 1}});
        assert_eq!(String::from_utf8(canonical_json(&value).unwrap()).unwrap(), "{\"a\":[true,null,\"é\"],\"b\":2,\"nested\":{\"y\":1,\"z\":0}}");
    }
}
