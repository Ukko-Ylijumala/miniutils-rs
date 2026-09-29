# Miniutils

A compilation of task-specific utilities for Rust which might not warrant a separate crate by themselves.

## Features

Always available:

- **ToDisplay / ToDebug**: Convenience traits
- **HumanBytes**: Convert bytes (number) to human readable format, f.ex. `1.5 KiB`
- **str_to_bytes / str_to_bytes_64**: Convert a size string (`64k`, `1.5 MB`, `2 gigabytes`) to bytes
- **inject / templater!**: Fill `{}` placeholders of a runtime template string
- **normalize_path**: Sanitize a path and resolve `.` / `..` without touching the filesystem
- **num_cpus**: Number of available CPUs

Behind cargo features (all enabled by default):

| Feature      | Contents                                                                  |
|--------------|---------------------------------------------------------------------------|
| `filesystem` | `check_readable_dir`                                                      |
| `iptools`    | `miniutils::iptools`: IP/CIDR/range parsing, iteration and CIDR collapsing |
| `sysinfo`    | `SysInfo` and `ProcessInfo` system/process stats, `sysinfo-printer` binary |
| `tabulator`  | `simple_tabulate` / `tabulate_with_missing` text tables (ANSI and wide-char aware) |

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies.miniutils]
git = "https://github.com/Ukko-Ylijumala/miniutils-rs"
version = "0.3"
```

For a lean build, pick only the features you need:

```toml
[dependencies.miniutils]
git = "https://github.com/Ukko-Ylijumala/miniutils-rs"
version = "0.3"
default-features = false
features = ["iptools"]
```

Releases are tagged `vX.Y.Z`; use `tag = "v0.3.2"` instead of `version` to pin one exactly.

Requires Rust 1.88 or newer.

## Basic Usage

```rust
use miniutils::{str_to_bytes_64, HumanBytes};
use miniutils::iptools::{collapse_strings, Cidr};

let bs: u64 = str_to_bytes_64("64k").unwrap();                          // 65536
let size: String = HumanBytes::to_human(1536.0, false, 1).unwrap();     // "1.5 KiB"
let nets: Vec<Cidr> = collapse_strings(&["10.0.0.0/25", "10.0.0.128/25"], 0); // [10.0.0.0/24]
```

## License

Copyright (c) 2024-2026 Mikko Tanner. All rights reserved.

License: MIT OR Apache-2.0

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Version History

- 0.3.2: Performance
    - tabulator ~3x faster: each cell measured once, rows built in one buffer
    - tabulator measures display width (CJK / emoji count as 2 columns) and no longer needs `regex` or `lazy_static`
    - IP collapsing merges in a single in-place pass (10-27% faster), one shared pipeline for all `collapse_*` functions
    - minimum supported Rust version pinned: 1.88 (`rust-version`, required by `sysinfo` 0.37)
- 0.3.1: Small fixes
    - `tabulate_with_missing` no longer panics on rows short by 2+ columns
    - path sanitizing also removes C1 control characters (`U+0080..=U+009F`)
    - `HumanBytes` prints `-0.0` as `0 B`, not `-0 B`
    - `MAX_RANGE_SIZE` is public; `Default` for `SysInfo` / `ProcessInfo`; `is_empty()` for `Cidr` / `IpRange`
    - `sysinfo` dependency built with its `system` feature only; clippy and rustdoc clean-ups
- 0.3.0: API clean-up (**breaking**)
    - `Cidr::from_str` returns `AddressError` (now `#[non_exhaustive]`) instead of `String`
    - `IpRange` fields are private: use `IpRange::new`, `beg()`, `end()`
    - one exported `IpIterator` for both `Cidr::iter` and `IpRange::iter`
    - `check_readable_dir` returns `PathBuf`; `str_to_bytes_64` returns `Result<u64, String>`
    - `str_to_bytes` rewritten: spaces between number and unit, exact integers, strict errors
    - IPv6 short-range ends are hexadecimal (`2001:db8::1-ff`)
- 0.2.8: Bug fixes
    - IPv6 ranges and CIDRs ending at `ffff:...:ffff` no longer hang, `::/0` no longer panics
    - `SysInfo` no longer panics when the wall clock steps backwards
    - strict `normalize_path` also removes the non-strict character set
    - `templater!` works without importing `inject` / `Display`
    - `check_readable_dir` reports the real error and actually checks readability
- 0.2.7: Feature-gated module families (`filesystem`, `iptools`, `sysinfo`, `tabulator`)
- 0.1.5: Initial library version
    - Gather utilities to a separate crate

This library started its life as a component of a larger application, but at some point it made more sense to separate the code into its own little project and here we are.
