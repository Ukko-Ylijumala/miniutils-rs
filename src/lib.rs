// Copyright (c) 2024-2025 Mikko Tanner. All rights reserved.

mod humanbytes;
mod procinfo;
mod strtobytes;

pub use humanbytes::HumanBytes;
pub use procinfo::ProcessInfo;
use std::{
    fmt::{Debug, Display},
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
