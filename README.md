# Miniutils

A compilation of task-specific utilities for Rust which might not warrant a separate crate by themselves.

## Features

- **Process info**: Process info structure
- **ToDisplay / ToDebug**: Convenience traits
- **HumanBytes**: Convert bytes (number) to human readable format
- **str_to_bytes**: Convert a number string to a raw number

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
miniutils = { git = "https://github.com/Ukko-Ylijumala/miniutils-rs" }
```

## Basic Usage

```rust
use miniutils::{ProcessInfo};
```

## License

Copyright (c) 2024-2025 Mikko Tanner. All rights reserved.

License: MIT OR Apache-2.0

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Version History

- 0.1.5: Initial library version
    - Gather utilities to a separate crate

This library started its life as a component of a larger application, but at some point it made more sense to separate the code into its own little project and here we are.
