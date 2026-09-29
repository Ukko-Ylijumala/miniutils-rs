// Copyright (c) 2025-2026 Mikko Tanner. All rights reserved.
// Licensed under the MIT License or the Apache License, Version 2.0.
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{borrow::Cow, fmt::Display, iter::repeat_n};
use unicode_width::UnicodeWidthStr;

const ESC: char = '\x1b';
/// separator between cells of a row
const COL_SEP: &str = " | ";
/// separator between columns of the line under the header row
const HDR_SEP: &str = "-+-";

/**
Remove ANSI SGR escape codes (`ESC [ <digits and ;> m`), f.ex. colors.
Borrows the input when there are none, which is the common case.
*/
fn strip_sgr(s: &str) -> Cow<'_, str> {
    if !s.contains(ESC) {
        return Cow::Borrowed(s);
    }
    let mut out: String = String::with_capacity(s.len());
    let mut rest: &str = s;
    while let Some(pos) = rest.find(ESC) {
        out.push_str(&rest[..pos]);
        let tail: &str = &rest[pos..];
        match sgr_len(tail) {
            0 => {
                // lone ESC, not an SGR sequence: keep it as text
                out.push(ESC);
                rest = &tail[ESC.len_utf8()..];
            }
            n => rest = &tail[n..],
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Byte length of the SGR sequence at the start of `s`, or 0 if there is none.
fn sgr_len(s: &str) -> usize {
    let b: &[u8] = s.as_bytes();
    if b.len() < 3 || b[0] != ESC as u8 || b[1] != b'[' {
        return 0;
    }
    let params: usize = b[2..].iter().take_while(|c| c.is_ascii_digit() || **c == b';').count();
    match b.get(2 + params) {
        Some(b'm') => 3 + params,
        _ => 0,
    }
}

/**
Return the visible width of a string in terminal columns, ignoring ANSI
escape codes. Wide characters (CJK, most emoji) count as 2 columns and
combining / zero-width characters as 0.
*/
#[inline]
fn visible_len(s: &str) -> usize {
    strip_sgr(s).width()
}

/// Byte capacity estimate for one output line (exact for ASCII-only content).
#[inline]
fn line_capacity(widths: &[usize], sep: &str) -> usize {
    widths.iter().sum::<usize>() + sep.len() * widths.len().saturating_sub(1)
}

/// Append one padded cell, preceded by [COL_SEP] unless it is the first column.
#[inline]
fn push_cell(line: &mut String, col: usize, item: &str, vis_len: usize, width: usize) {
    if col > 0 {
        line.push_str(COL_SEP);
    }
    line.push_str(item);
    line.extend(repeat_n(' ', width.saturating_sub(vis_len)));
}

/**
Format a single row with padding. `row_lens` holds the visible length of
each item, `missing` the fill value for absent columns and its visible length.
*/
fn format_row(
    row: &[String],
    row_lens: &[usize],
    widths: &[usize],
    missing: Option<(&str, usize)>,
) -> String {
    let mut line: String = String::with_capacity(line_capacity(widths, COL_SEP));

    for (col, (item, &vis_len)) in row.iter().zip(row_lens).enumerate() {
        push_cell(&mut line, col, item, vis_len, widths[col]);
    }

    // Pad with `missing` value(s) if the row has too few items: one per absent column
    if let Some((missing, missing_len)) = missing {
        for (col, &width) in widths.iter().enumerate().skip(row.len()) {
            push_cell(&mut line, col, missing, missing_len, width);
        }
    }
    line
}

/// Core tabulation function
fn tabulate(data: &[Vec<String>], hdr: bool, out: &mut Vec<String>, missing: Option<&str>) {
    // Visible length of every item, computed once and reused for the padding
    let lens: Vec<Vec<usize>> = data
        .iter()
        .map(|row: &Vec<String>| row.iter().map(|item: &String| visible_len(item)).collect())
        .collect();

    // Find the maximum width needed for each column (based on visible lengths)
    let columns: usize = lens.iter().map(Vec::len).max().unwrap_or(1);
    let mut widths: Vec<usize> = vec![0; columns];
    for row_lens in &lens {
        for (width, &len) in widths.iter_mut().zip(row_lens) {
            *width = (*width).max(len);
        }
    }

    let missing: Option<(&str, usize)> = missing.map(|m: &str| (m, visible_len(m)));
    out.reserve(data.len() + usize::from(hdr));

    // Format each row with appropriate padding
    let start_index: usize = if hdr {
        // Format headers with a separator line
        out.push(format_row(&data[0], &lens[0], &widths, missing));
        let mut separator: String = String::with_capacity(line_capacity(&widths, HDR_SEP));
        for (col, &width) in widths.iter().enumerate() {
            if col > 0 {
                separator.push_str(HDR_SEP);
            }
            separator.extend(repeat_n('-', width));
        }
        out.push(separator);
        1
    } else {
        0
    };

    for (row, row_lens) in data.iter().zip(&lens).skip(start_index) {
        out.push(format_row(row, row_lens, &widths, missing));
    }
}

/**
Format a collection of rows as a table for printing.

## Arguments
* `data` - Iterator of rows (each row is an iterator of items)
* `headers` - Optional slice of column headers

## Returns
  * Vec of Strings containing the formatted table
*/
pub fn simple_tabulate<I, R, T>(data: I, headers: Option<&[&str]>) -> Vec<String>
where
    I: IntoIterator<Item = R>,
    R: IntoIterator<Item = T>,
    T: Display,
{
    let mut data_rows: Vec<Vec<String>> = Vec::new();
    let mut formatted: Vec<String> = Vec::new();

    // Add headers if provided
    if let Some(hdrs) = headers {
        data_rows.push(hdrs.iter().map(|h: &&str| h.to_string()).collect());
    }

    // Stringify all data row items
    for row in data {
        let stringified_items: Vec<String> = row.into_iter().map(|item| item.to_string()).collect();
        data_rows.push(stringified_items);
    }

    if data_rows.is_empty() {
        return formatted;
    }

    tabulate(&data_rows, headers.is_some(), &mut formatted, None);
    formatted
}

/**
Format a collection of rows as a table for printing. Handles `Option<T>` values,
replacing None with the provided `missing` string.

## Arguments
* `data` - Iterator of rows (each row is an iterator of items)
* `headers` - Optional slice of column headers
* `missing` - String to replace None values with

## Returns
  * Vec of Strings containing the formatted table
*/
pub fn tabulate_with_missing<I, R, T>(
    data: I,
    headers: Option<&[&str]>,
    missing: &str,
) -> Vec<String>
where
    I: IntoIterator<Item = R>,
    R: IntoIterator<Item = Option<T>>,
    T: Display,
{
    let mut data_rows: Vec<Vec<String>> = Vec::new();
    let mut formatted: Vec<String> = Vec::new();

    // Add headers if provided
    if let Some(hdrs) = headers {
        data_rows.push(hdrs.iter().map(|h: &&str| h.to_string()).collect());
    }

    // Handle Option<T> values and stringify all data row items
    for row in data {
        let stringified_items: Vec<String> = row
            .into_iter()
            .map(|item| match item {
                Some(val) => val.to_string(),
                None => missing.to_string(),
            })
            .collect();
        data_rows.push(stringified_items);
    }

    if data_rows.is_empty() {
        return formatted;
    }

    tabulate(&data_rows, headers.is_some(), &mut formatted, Some(missing));
    formatted
}

/* ######################################################################### */

#[cfg(test)]
mod tests {
    use super::*;

    const HDRS_2: [&str; 2] = ["a", "b"];
    const HDRS_3: [&str; 3] = ["a", "b", "c"];
    const RED: &str = "\x1b[31mred\x1b[0m";
    // (input, visible width): same results as the old `\x1b\[[0-9;]*m` regex strip for ASCII
    #[rustfmt::skip]
    const WIDTHS: [(&str, usize); 9] = [
        ("",                            0),
        ("abc",                         3),
        (RED,                           3),
        ("\x1b[1;4;38;5;208mX\x1b[m",    1),
        ("\x1b",                        1),    // lone ESC is kept as text
        ("\x1b[31",                     4),    // unterminated: not an SGR sequence
        ("\x1b[31\x1b[0m",              4),    // ...but the following one still is
        ("名前",                        4),    // wide (CJK)
        ("e\u{301}",                    1),    // e + combining acute accent
    ];

    #[test]
    fn test_simple_tabulate() {
        let out: Vec<String> = simple_tabulate(vec![vec![1, 22], vec![333, 4]], Some(&HDRS_2));
        assert_eq!(out, ["a   | b ", "----+---", "1   | 22", "333 | 4 "]);
    }

    #[test]
    fn test_ansi_codes_are_invisible() {
        let out: Vec<String> = simple_tabulate(vec![vec![RED, "x"], vec!["ab", "y"]], None);
        assert_eq!(out, [format!("{RED} | x"), "ab  | y".to_string()]);
    }

    #[test]
    fn test_visible_len() {
        for (input, expected) in WIDTHS {
            assert_eq!(visible_len(input), expected, "Failed: {input:?}");
        }
    }

    #[test]
    fn test_wide_chars_align() {
        let out: Vec<String> = simple_tabulate(vec![vec!["名前", "x"], vec!["ab", "y"]], None);
        assert_eq!(out, ["名前 | x", "ab   | y"]);
    }

    #[test]
    fn test_tabulate_with_missing() {
        // row short by 2 columns used to panic (index out of bounds)
        let rows: Vec<Vec<Option<i32>>> = vec![vec![Some(1), None, Some(3)], vec![Some(4)]];
        let out: Vec<String> = tabulate_with_missing(rows, Some(&HDRS_3), "-");
        assert_eq!(out, ["a | b | c", "--+---+--", "1 | - | 3", "4 | - | -"]);
    }
}
