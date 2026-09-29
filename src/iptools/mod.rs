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
pub use structs::{Cidr, IpFam, IpIterator, IpRange};

pub(crate) const IPV4_BITS: u8 = 32;
pub(crate) const IPV6_BITS: u8 = 128;
/// Max number of addresses [parse_ip_or_range] and [generate_ip_range] will produce.
pub const MAX_RANGE_SIZE: usize = 65536;

/// Errors from parsing IPs, ranges and CIDRs. May gain variants in minor versions.
#[rustfmt::skip]
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
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
    /// invalid plain IP address (no prefix) given as a CIDR
    InvalidAddr(String),
    /// CIDR has more than one slash
    InvalidCidrFmt(String),
    InvalidCidrAddr(String),
    InvalidCidrPrefix(String),
    /// prefix is > 32
    InvalidV4Prefix(u8),
    /// prefix is > 128
    InvalidV6Prefix(u8),
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
                write!(f, "{ERR_V6_HEXTET} {val:x}")
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
            AddressError::InvalidAddr(addr) => {
                write!(f, "{ERR_INV_ADDR}: '{addr}'")
            }
            AddressError::InvalidCidrFmt(cidr) => {
                write!(f, "{ERR_CIDR_FMT}: '{cidr}'")
            }
            AddressError::InvalidCidrAddr(addr) => {
                write!(f, "{ERR_CIDR_INV_ADDR}: '{addr}'")
            }
            AddressError::InvalidCidrPrefix(prefix) => {
                write!(f, "{ERR_CIDR_INV_PRE}: '{prefix}'")
            }
            AddressError::InvalidV4Prefix(prefix) => {
                write!(f, "{ERR_CIDR_INV_V4}: '{prefix}'")
            }
            AddressError::InvalidV6Prefix(prefix) => {
                write!(f, "{ERR_CIDR_INV_V6}: '{prefix}'")
            }
        }
    }
}

impl error::Error for AddressError {}
