// Copyright (c) 2023-2026 Mikko Tanner. All rights reserved.

static ERR_TOO_LARGE: &str = "number too large to fit in a u128";
static ERR_TOO_LARGE_64: &str = "number too large to fit in a u64";
static ERR_NEGATIVE: &str = "number must be finite and non-negative";

/// 2^128 (u128::MAX rounds up to exactly this as f64), the first value that no longer fits.
const U128_LIMIT: f64 = u128::MAX as f64;

/**
Converts a size specification string to the equivalent number of bytes.

Logic originally converted from Python to Rust, original here:
https://stackoverflow.com/questions/44307480/convert-size-notation-with-units-100kb-32mb-to-number-of-bytes-in-python

The function recognizes suffixes for kilobytes (k, kb), megabytes (m, mb),
gigabytes (g, gb), terabytes (t, tb), petabytes (p, pb), exabytes (e, eb),
zettabytes (z, zb), and yottabytes (y, yb). It also recognizes the long form
of these suffixes (kilobyte, megabyte, etc.). Bytes are given with no suffix,
or with 'b' or 'byte'. The suffixes are case-insensitive, may be plural
("bytes", "megabytes") and multipliers are binary (1k = 1024).

The number part is parsed exactly as an integer when possible, otherwise as a
floating-point number (f.ex. "1.5m", "1e3k"). Fractional bytes are truncated.

Special cases:
- singular units, e.g., "1 byte"
- byte vs b
- yottabytes, zettabytes, etc.
- with & without spaces between & around units.
- floats ("5.2 mb")

# Arguments

* `size_str`: A string specifying the size. It consists of a number part and an optional suffix.

# Returns

* `u128`: The number of bytes corresponding to the size specification.

# Errors
 * if the suffix is not a recognized unit
 * if the number part cannot be parsed as an integer or a floating-point number
 * if the number is negative, NaN or infinite
 * if the number of bytes is too large to fit in a u128

# Example
```
use miniutils::str_to_bytes;
assert_eq!(str_to_bytes("1.5 MB").unwrap(), 1_572_864);
```
*/
pub fn str_to_bytes(size_str: &str) -> Result<u128, String> {
    let s: String = size_str.trim().to_ascii_lowercase();

    // unit = trailing ASCII letters, number = everything before it
    let num_len: usize = s.trim_end_matches(|c: char| c.is_ascii_alphabetic()).len();
    let (num_str, unit) = s.split_at(num_len);
    let num_str: &str = num_str.trim();

    // plural, f.ex. "bytes" or "kbs" (but a lone "s" is not a unit)
    let unit: &str = match unit.strip_suffix('s') {
        Some(u) if !u.is_empty() => u,
        _ => unit,
    };

    let shift: u32 = unit_shift(unit).ok_or_else(|| format!("unknown unit: '{unit}'"))?;
    let multiplier: u128 = 1u128 << shift;

    // integers stay exact, only fractions and exponents go through f64
    if let Ok(num) = num_str.parse::<u128>() {
        return num.checked_mul(multiplier).ok_or_else(|| ERR_TOO_LARGE.to_string());
    }

    let num: f64 = num_str
        .parse::<f64>()
        .map_err(|_| format!("invalid number: '{num_str}'"))?;
    if !num.is_finite() || num < 0.0 {
        return Err(ERR_NEGATIVE.to_string());
    }

    let bytes: f64 = num * multiplier as f64;
    if bytes >= U128_LIMIT {
        return Err(ERR_TOO_LARGE.to_string());
    }
    Ok(bytes as u128)
}

/**
Convert a string to bytes (u64 version)

Wrapper function for `str_to_bytes()` which returns a `u64` instead
of `u128`.

# Arguments

* `s` - The string to be converted to bytes

# Errors
* Returns an error if the parsing is not successful.
* Returns an error if the number of bytes is too large to fit in a u64.

# Example
```
use miniutils::str_to_bytes_64;
let bytes = str_to_bytes_64("64k").unwrap();
assert_eq!(bytes, 65536);
```
*/
pub fn str_to_bytes_64(s: &str) -> Result<u64, String> {
    let val: u128 = str_to_bytes(s)?;
    u64::try_from(val).map_err(|_| ERR_TOO_LARGE_64.to_string())
}

/// Binary multiplier of a (lowercase, singular) unit as a power of two.
#[rustfmt::skip]
fn unit_shift(unit: &str) -> Option<u32> {
    Some(match unit {
        "" | "b" | "byte"                       => 0,
        "k" | "kb" | "kilobyte"                 => 10,
        "m" | "mb" | "megabyte"                 => 20,
        "g" | "gb" | "gigabyte"                 => 30,
        "t" | "tb" | "terabyte"                 => 40,
        "p" | "pb" | "petabyte"                 => 50,
        "e" | "eb" | "exabyte"                  => 60,
        // "zetabyte": misspelling accepted by earlier versions
        "z" | "zb" | "zettabyte" | "zetabyte"   => 70,
        "y" | "yb" | "yottabyte"                => 80,
        _ => return None,
    })
}

/* ######################################################################### */

#[cfg(test)]
mod tests {
    use super::*;

    #[rustfmt::skip]
    const OK: [(&str, u128); 26] = [
        ("0",                   0),
        ("5",                   5),
        ("5b",                  5),
        ("5 b",                 5),
        ("1 byte",              1),
        ("5 bytes",             5),
        ("5k",                  5120),
        ("5kb",                 5120),
        ("5 KB",                5120),
        ("5 kbs",               5120),
        ("5 kilobytes",         5120),
        ("  5k  ",              5120),
        ("1.5m",                1_572_864),
        ("1.5 megabytes",       1_572_864),
        (".5k",                 512),
        ("1.1k",                1126), // 1126.4, truncated
        ("1e3",                 1000),
        ("1e3k",                1_024_000),
        ("2g",                  2_147_483_648),
        ("1e",                  1 << 60),
        ("1 zettabyte",         1 << 70),
        ("1 zetabyte",          1 << 70),
        ("1y",                  1 << 80),
        // exact, no f64 rounding
        ("123456789123456789k", 126_419_752_062_419_751_936),
        ("340282366920938463463374607431768211455", u128::MAX),
        ("281474976710655y",    281_474_976_710_655u128 << 80),
    ];

    #[rustfmt::skip]
    const ERR: [&str; 15] = [
        "", "k", "abc", "5s", "5 foo", "5 k b",
        "5kbkb", "5bbbb",                   // used to be accepted as 5k / 5
        "-5k", "nank", "infk", "inf",       // used to be Ok(0) / Ok(u128::MAX)
        "281474976710656y",                 // 2^48 * 2^80 = 2^128 (integer path)
        "3e14y",                            // > 2^128 (float path)
        "340282366920938463463374607431768211456",
    ];

    #[test]
    fn test_str_to_bytes_ok() {
        for (input, expected) in OK {
            assert_eq!(str_to_bytes(input), Ok(expected), "Failed: '{input}'");
        }
    }

    #[test]
    fn test_str_to_bytes_err() {
        for input in ERR {
            assert!(str_to_bytes(input).is_err(), "Should fail: '{input}' -> {:?}", str_to_bytes(input));
        }
    }

    #[test]
    fn test_str_to_bytes_64() {
        assert_eq!(str_to_bytes_64("15e"), Ok(15 << 60));
        assert_eq!(str_to_bytes_64("16e"), Err(ERR_TOO_LARGE_64.to_string()));
        // parse errors are reported as such, not as overflow
        assert_eq!(str_to_bytes_64("5 foo"), Err("unknown unit: 'foo'".to_string()));
    }
}
