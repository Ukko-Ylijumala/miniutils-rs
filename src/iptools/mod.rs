// Copyright (c) 2026 Mikko Tanner. All rights reserved.
// Licensed under the MIT License or the Apache License, Version 2.0.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! IP address and/or CIDR parsing/collapsing into minimal representations.

mod addresses;
mod collapsing;
mod strings;
mod structs;

use std::{
    error, fmt,
    net::{AddrParseError, IpAddr},
    num::ParseIntError,
};
use strings::*;

pub use addresses::*;
pub use collapsing::*;
pub use structs::{Cidr, IpFam, IpRange};

pub(crate) const IPV4_BITS: u8 = 32;
pub(crate) const IPV6_BITS: u8 = 128;
pub(crate) const MAX_RANGE_SIZE: usize = 65536; // max number of addresses in a range allowed

#[rustfmt::skip]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AddressError {
    /// invalid IP/range/CIDR
    Invalid(String),
    /// range format is invalid
    InvalidRangeFmt(String),
    InvalidRangeBegIp  { beg: String, source: AddrParseError },
    InvalidRangeEndIp  { end: String, source: AddrParseError },
    InvalidRangeEndVal { val: String, source: ParseIntError },
    InvalidV4Octet(u32),
    InvalidV6Hextet(u32),
    RangeTooLarge(u128),
    RangeOrder(IpAddr, IpAddr),
    /// start and end are not the same IP family (v4 vs v6).
    Mismatch(IpAddr, IpAddr),
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressError::Invalid(ip) => {
                write!(f, "{ERR_INVALID_IP}: '{ip}'")
            }
            AddressError::InvalidRangeFmt(rng) => {
                write!(f, "{ERR_RNG_FMT}: '{rng}'")
            }
            AddressError::InvalidV4Octet(val) => {
                write!(f, "{ERR_V4_OCTET} {val}")
            }
            AddressError::InvalidV6Hextet(val) => {
                write!(f, "{ERR_V6_HEXTET} {val}")
            }
            AddressError::RangeTooLarge(size) => {
                write!(f, "{ERR_RNG_TOOLARGE}: {size} (max {MAX_RANGE_SIZE})")
            }
            AddressError::RangeOrder(beg, end) => {
                write!(f, "{ERR_RNG_ORDER} ({beg} > {end})")
            }
            AddressError::Mismatch(a, b) => {
                write!(f, "{ERR_MISMATCH}: {a} - {b}")
            }
            AddressError::InvalidRangeBegIp { beg, source } => {
                write!(f, "{ERR_START}: '{beg}': {source}")
            }
            AddressError::InvalidRangeEndIp { end, source } => {
                write!(f, "{ERR_END}: '{end}': {source}")
            }
            AddressError::InvalidRangeEndVal { val, source } => {
                write!(f, "{ERR_RNG_END}: '{val}': {source}")
            }
        }
    }
}

impl error::Error for AddressError {}
