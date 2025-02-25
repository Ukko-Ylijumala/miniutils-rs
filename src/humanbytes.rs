// Copyright (c) 2023 Mikko Tanner. All rights reserved.

/**
The `HumanBytes` struct is a utility for converting a floating-point number
to a human-readable string representation in either binary or metric units.

Logic converted from Python to Rust, original here:
https://stackoverflow.com/questions/12523586/python-format-size-application-converting-b-to-kb-mb-gb-tb

The `to_human` function can represent the number in metric units (kB, MB, GB, TB, PB, EB, ZB, YB)
or in binary units (KiB, MiB, GiB, TiB, PiB, EiB, ZiB, YiB). The number of digits after the decimal
point can be 0, 1, 2, or 3, depending on the `precision` argument.

Negative numbers are represented with a minus sign in front of the number.
*/
pub struct HumanBytes;

impl HumanBytes {
    const METRIC_LABELS: [&'static str; 9] = ["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
    const BINARY_LABELS: [&'static str; 9] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"];
    const PRECISION_OFFSETS: [f64; 4] = [0.5, 0.05, 0.005, 0.0005];

    /**
    Converts a floating-point number to a human-readable string in either binary or metric units.

    # Arguments
    * `num`: The number to be converted.
    * `metric`: If true, the function uses metric units; otherwise, it uses binary units.
    * `precision`: The number of digits after the decimal point. It must be in the range 0-3.

    # Returns
    * `Ok(String)`: A string that represents the number in the requested units with the requested precision.
    * `Err(&'static str)`: An error if `num` is not a normal number or if `precision` is not in the range 0-3.
    */
    pub fn to_human(num: f64, metric: bool, precision: usize) -> Result<String, &'static str> {
        if !(num.is_normal() || num == 0.0) {
            return Err("num must be a regular number");
        }
        if precision > 3 {
            return Err("precision must be in range 0-3");
        }

        let unit_labels: [&str; 9] = if metric {
            Self::METRIC_LABELS
        } else {
            Self::BINARY_LABELS
        };
        let last_label: &&str = unit_labels.last().unwrap();
        let unit_step: f64 = if metric { 1000.0 } else { 1024.0 };
        let unit_step_thresh: f64 = unit_step - Self::PRECISION_OFFSETS[precision];

        let sign: &str = if num.is_sign_negative() { "-" } else { "" };
        let mut num: f64 = num.abs();
        let mut unit: &str = "";

        for label in &unit_labels {
            unit = label;
            if num < unit_step_thresh {
                /*
                VERY IMPORTANT:
                Only accepts the CURRENT unit if we're BELOW the threshold where
                float rounding behavior would place us into the NEXT unit: f.ex.
                when rounding a float to 1 decimal, any number ">= 1023.95" will
                be rounded to "1024.0". Obviously we don't want ugly output such
                as "1024.0 KiB", since the proper term for that is "1.0 MiB".
                */
                break;
            }
            if label != last_label {
                /*
                We only shrink the number if we HAVEN'T reached the last unit.
                NOTE: These looped divisions accumulate floating point rounding
                errors, but each new division pushes the rounding errors further
                and further down in the decimals, so it doesn't matter at all.
                */
                num /= unit_step;
            }
        }

        Ok(match precision {
            0 => format!("{}{:.0} {}", sign, num, unit),
            1 => format!("{}{:.1} {}", sign, num, unit),
            2 => format!("{}{:.2} {}", sign, num, unit),
            3 => format!("{}{:.3} {}", sign, num, unit),
            // Since we've checked that precision is <= 3 earlier,
            // this branch should be unreachable
            _ => unreachable!(),
        })
    }
}
