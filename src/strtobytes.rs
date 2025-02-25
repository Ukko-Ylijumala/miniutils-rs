// Copyright (c) 2023 Mikko Tanner. All rights reserved.

use std::collections::HashMap;
use std::num::TryFromIntError;
use lazy_static::lazy_static;

// Lazily evaluated table of size constants.
// Will be generated only once per program execution.
lazy_static! {
    static ref MULTIPLIERS: HashMap<&'static str, u128> = {
        let mut m = HashMap::new();
        m.insert("kilobyte", 1024);
        m.insert("megabyte", 1024u128.pow(2));
        m.insert("gigabyte", 1024u128.pow(3));
        m.insert("terabyte", 1024u128.pow(4));
        m.insert("petabyte", 1024u128.pow(5));
        m.insert("exabyte", 1024u128.pow(6));
        m.insert("zetabyte", 1024u128.pow(7));
        m.insert("yottabyte", 1024u128.pow(8));
        m.insert("kb", 1024);
        m.insert("mb", 1024u128.pow(2));
        m.insert("gb", 1024u128.pow(3));
        m.insert("tb", 1024u128.pow(4));
        m.insert("pb", 1024u128.pow(5));
        m.insert("eb", 1024u128.pow(6));
        m.insert("zb", 1024u128.pow(7));
        m.insert("yb", 1024u128.pow(8));
        m.insert("k", 1024);
        m.insert("m", 1024u128.pow(2));
        m.insert("g", 1024u128.pow(3));
        m.insert("t", 1024u128.pow(4));
        m.insert("p", 1024u128.pow(5));
        m.insert("e", 1024u128.pow(6));
        m.insert("z", 1024u128.pow(7));
        m.insert("y", 1024u128.pow(8));
        m
    };
}

/**
Converts a size specification string to the equivalent number of bytes.

Logic converted from Python to Rust, original here:
https://stackoverflow.com/questions/44307480/convert-size-notation-with-units-100kb-32mb-to-number-of-bytes-in-python

The function recognizes suffixes for kilobytes (k, kb), megabytes (m, mb),
gigabytes (g, gb), terabytes (t, tb), petabytes (p, pb), exabytes (e, eb),
zetabytes (z, zb), and yottabytes (y, yb). It also recognizes the long form
of these suffixes (kilobyte, megabyte, etc.). The suffixes are case-insensitive.

The function interprets the number part of the string as a floating-point number.
It calculates the number of bytes as this number times the multiplier corresponding
to the suffix.

If the string ends with 'b' or 'byte', the function interprets the string as a
byte count. It attempts to parse the string as a u128 integer.

If the string does not have a recognized suffix and does not end with 'b' or 'byte',
the function treats it as an error.

Special cases:
- singular units, e.g., "1 byte"
- byte vs b
- yottabytes, zetabytes, etc.
- with & without spaces between & around units.
- floats ("5.2 mb")

# Arguments

* `size_str`: A string specifying the size. It consists of a number part and an optional suffix.

# Returns

* `u128`: The number of bytes corresponding to the size specification.

# Errors
 * if the string cannot be parsed as a floating-point number
 * if it does not have a recognized suffix, or does not end with 'b' or 'byte'
 * if the number of bytes is too large to fit in a u128
*/
pub fn str_to_bytes(size_str: &str) -> Result<u128, String> {
    let binding: String = size_str.to_lowercase();
    let size_str: &str = binding.trim().trim_end_matches('s');

    for (suffix, multiplier) in &*MULTIPLIERS {
        if size_str.ends_with(suffix) {
            let num_str: &str = size_str.trim_end_matches(suffix);
            if let Ok(num) = num_str.parse::<f64>() {
                return Ok((num * (*multiplier as f64)) as u128);
            }
        }
    }

    // Case when the string ends with 'b' or 'byte'
    let size_str: &str = if size_str.ends_with("b") {
        size_str.trim_end_matches('b')
    } else if size_str.ends_with("byte") {
        size_str.trim_end_matches("byte")
    } else {
        &size_str
    };

    match size_str.parse::<u128>().ok() {
        Some(val) => {
            if val > u128::MAX as u128 {
                Err("number too large to fit in a u128".to_string())
            } else {
                Ok(val)
            }
        }
        None => Err("string conversion to a number failed".to_string())
    }
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
```ignore
let bytes = str_to_bytes_64("64k").unwrap();
assert_eq!(bytes, 65536);
```
*/
pub fn str_to_bytes_64(s: &str) -> Result<u64, TryFromIntError> {
    let val: u128 = str_to_bytes(s).unwrap_or(u128::MAX);
    u64::try_from(val)
}
