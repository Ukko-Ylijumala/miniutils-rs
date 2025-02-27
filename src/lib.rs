// Copyright (c) 2024-2025 Mikko Tanner. All rights reserved.

mod humanbytes;
mod procinfo;
mod strtobytes;

pub use humanbytes::HumanBytes;
pub use procinfo::ProcessInfo;
use std::{
    fmt::{Debug, Display},
    path::{Component, Path, PathBuf},
    thread::available_parallelism,
};
pub use strtobytes::{str_to_bytes, str_to_bytes_64};

/* ######################################################################### */

/// This trait makes available a method `.to_debug()` for converting a value
/// to its debug string. A type must implement the [Debug] trait to use this.
pub trait ToDebug {
    fn to_debug(&self) -> String;
}

/// This trait makes available a method `.to_display()` for converting a value
/// to its display string. A type must implement the [Display] trait to use this.
pub trait ToDisplay {
    fn to_display(&self) -> String;
}

impl<T: Debug> ToDebug for T {
    /// Convert a value to its debug string. Convenience method for `format!("{self:?}")`.
    #[inline]
    fn to_debug(&self) -> String {
        format!("{self:?}")
    }
}

impl<T: Display> ToDisplay for T {
    /// Convert a value to its display string. Convenience method for `format!("{self}")`.
    #[inline]
    fn to_display(&self) -> String {
        format!("{self}")
    }
}

/* ######################################################################### */

/// Get the number of available CPUs, but at least 1.
pub fn num_cpus() -> usize {
    match available_parallelism() {
        Ok(available) => available.get(),
        Err(_) => 1,
    }
}

/* ######################################################################### */

/**
Whether a character definitely can be considered "suspicious"
in the context of a filesystem path.

This function flags the following:
- Null character, newline and carriage return
- Backslash used as an escape character
- Control characters (`\x01`..=`\x1F` and `\x7F`)
*/
#[inline]
pub fn is_suspicious_char(c: char) -> bool {
    matches!(c, '\0' | '\n' | '\r' | '\\' | '\x01'..='\x1F' | '\x7F')
}

/**
More strict version of `is_suspicious_char()`.

This function flags the following characters:
```ignore
    '*' | '?' |     // Wildcards
    '"' | '\'' |    // Quote characters
    '<' | '>' |     // Command output redirection
    '|' |           // Command piping
    ';' |           // Command separator in some shells
    '&' |           // Background job operator in some shells
    '!' |           // History expansion in some shells
    '$' |           // Environment variable expansion
    '`' |           // Command substitution in some shells
    '(' | ')' |     // Special characters in some shells
    '[' | ']' |
    '{' | '}' |
*/
#[rustfmt::skip]
#[inline]
pub fn is_suspicious_strict(c: char) -> bool {
    matches!(
        c,
        '*' | '?' | '"' | '\'' |
        '<' | '>' | '|' |
        ';' | '&' | '!' | '$' | '`' |
        '(' | ')' | '[' | ']' | '{' | '}'
    )
}

#[rustfmt::skip]
/**
Normalize a string path by removing suspicious characters and resolving
relative path components (e.g. `.` and `..`).

In contrast to the standard library's [std::path::Path::canonicalize], this
function does not require the path to exist on the filesystem, but it cannot
resolve symlinks either.

"Suspicous characters" in non-strict context are considered to be:
- Null character, newline and carriage return
- Backslash used as an escape character
- Control characters (`\x01`..=`\x1F` and `\x7F`)

... and in strict context, in addition to the above:
- Wildcards: `*` and `?`
- Quote characters: `"` and `'`
- Command output redirection: `<` and `>`
- Command piping: `|`
- Command separator in some shells: `;`
- Background job operator in some shells: `&`
- History expansion in some shells: `!`
- Environment variable expansion: `$`
- Command substitution in some shells: `` ` ``
- Special characters in some shells: `(`, `)`, `[`, `]`, `{`, `}`

NOTE: non-unicode sequences will be replaced with the replacement
character [`U+FFFD REPLACEMENT CHARACTER`][U+FFFD].

[U+FFFD]: core::char::REPLACEMENT_CHARACTER
*/
pub fn normalize_path<P>(path: P, strict: bool) -> PathBuf
where P: AsRef<Path>,
{
    let path: &Path = path.as_ref();
    let mut parts: Vec<Component> = Vec::new();
    let mut is_absolute: bool = false;

    // Sanitize the string by removing certain suspicious characters,
    // which for sure don't belong in a filesystem path.
    let sanitized: String = path
        .to_string_lossy()
        .chars()
        .filter(|&c| !{
            match strict {
                true => is_suspicious_strict(c),
                false => is_suspicious_char(c)}
            })
        .collect();

    for c in Path::new(&sanitized).components() {
        match c {
            Component::CurDir => {}
            Component::Prefix(_) => {
                // this is a Windows thingy, so we skip it
                //parts.push(Component::Prefix(prefix));
            }

            Component::RootDir => {
                if !parts.is_empty() {
                    parts.clear();
                }
                parts.push(c);
                is_absolute = true;
            }

            Component::Normal(s) => {
                if s.to_string_lossy().chars().all(|c| c == '.') {
                    // skip components that consist only of dots
                    continue;
                }
                parts.push(c);
            }

            Component::ParentDir => {
                if !is_absolute || !parts.is_empty() {
                    match parts.last() {
                        Some(Component::Normal(_)) => { parts.pop(); }
                        Some(Component::ParentDir | Component::CurDir) => { parts.pop(); }
                        //None => { parts.push(c); }
                        _ => {}
                    }
                }
            }
        }
    }
    PathBuf::from_iter(parts)
}

/* ######################################################################### */

#[cfg(test)]
mod tests {
    use super::*;

    #[rustfmt::skip]
    #[test]
    fn test_normalize_path() {
        let tests: Vec<&str> = vec![
            // basics
            "/a/b/../c/./d",            "/a/c/d",
            "a/../../b/c/d",            "b/c/d",
            "/a/b/../../../../c",       "/c",
            "a/../.././/c",             "c",
            "../a/b/c",                 "a/b/c",
            "./a/./b/./c",              "a/b/c",
            "./a/./b/./...../c",        "a/b/c",
            "",                         "",
            "./",                       "",
            "/",                        "/",
            "/.",                       "/",
            "/./",                      "/",
            "/./.",                     "/",
            "/..",                      "/",
            "../..",                    "",
            "./foo",                    "foo",

            // Suspicious character tests
            "/a/b\0/c",                 "/a/b/c",
            "/a/b\n/c",                 "/a/b/c",
            "/a/b\r/c",                 "/a/b/c",
            "/a/b\\/c",                 "/a/b/c",
            "a/b/../../.\\/\\//c",      "c",

            // Combined tests
            "/a/b\0/../\\Xc/./d\r\n",   "/a/Xc/d",
            "a/b\x1a/\\Xc/\x1F\\./d..", "a/b/Xc/d..",
        ];

        for i in (0..tests.len()).step_by(2) {
            let input: &str = tests[i];
            let expected: &str = tests[i + 1];
            assert_eq!(normalize_path(input, false), PathBuf::from(expected), "Failed: '{input}'");
        }
    }
}
