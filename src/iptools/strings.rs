// Copyright (c) 2026 Mikko Tanner. All rights reserved.
// Licensed under the MIT License or the Apache License, Version 2.0.
// SPDX-License-Identifier: MIT OR Apache-2.0

pub(crate) static DASH: &str = "-";
pub(crate) static SLASH: &str = "/";

// addresses.rs
pub(crate) static ERR_INVALID_IP: &str = "invalid IP address, CIDR, or range";
pub(crate) static ERR_RNG_FMT: &str = "invalid range format";
pub(crate) static ERR_START: &str = "invalid start IP in range";
pub(crate) static ERR_END: &str = "invalid end IP in range";
pub(crate) static ERR_RNG_END: &str = "invalid range end value";
pub(crate) static ERR_V4_OCTET: &str = "IPv4 octet must be <= 255, got";
pub(crate) static ERR_V6_HEXTET: &str = "IPv6 hextet must be <= 65535, got";
pub(crate) static ERR_RNG_ORDER: &str = "start IP is greater than end IP";
pub(crate) static ERR_RNG_TOOLARGE: &str = "range too large - addresses";
pub(crate) static ERR_MISMATCH: &str = "cannot mix IPv4 and IPv6 in range";
pub(crate) static PANIC_NAUGHTY: &str = "Naughty programmer! Beginning cannot be larger than end!";

// structs.rs
pub(crate) static ERR_INV_ADDR: &str = "invalid IP address";
pub(crate) static ERR_CIDR_FMT: &str = "invalid CIDR format (too many slashes)";
pub(crate) static ERR_CIDR_INV_ADDR: &str = "invalid IP address in CIDR";
pub(crate) static ERR_CIDR_INV_PRE: &str = "invalid prefix in CIDR";
pub(crate) static ERR_CIDR_INV_V4: &str = "invalid IPv4 prefix in CIDR";
pub(crate) static ERR_CIDR_INV_V6: &str = "invalid IPv6 prefix in CIDR";
