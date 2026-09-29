// Copyright (c) 2026 Mikko Tanner. All rights reserved.
// Licensed under the MIT License or the Apache License, Version 2.0.
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::{
    collapsing::{cidr_to_range, int_to_ip},
    strings::*,
    AddressError, IPV4_BITS, IPV6_BITS,
};
use std::{
    fmt,
    iter::FusedIterator,
    net::IpAddr,
    str::FromStr,
};

/// IP address family
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpFam {
    V4,
    V6,
}

/// Inclusive range of IP addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Range {
    pub fam: IpFam,
    pub beg: u128,
    /// inclusive
    pub end: u128,
}

impl Range {
    pub fn cmp_key(&self) -> (u8, u128, u128) {
        let fam_key = match self.fam {
            IpFam::V4 => 0u8,
            IpFam::V6 => 1u8,
        };
        (fam_key, self.beg, self.end)
    }

    /// The length of the range. Cannot be an [usize] due to IPv6. Saturating.
    pub fn len(&self) -> u128 {
        let diff: u128 = self.end.saturating_sub(self.beg);
        if diff == u128::MAX {
            return u128::MAX;
        }
        diff.saturating_add(1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cidr {
    /// network address
    pub addr: IpAddr,
    /// **v4**: `0..=32`, **v6**: `0..=128`
    pub prefix: u8,
}

impl Cidr {
    /// Number of IP addresses contained by this [Cidr].
    /// Cannot be an [usize] due to IPv6. Saturating.
    pub fn len(&self) -> u128 {
        let bits: u8 = match self.addr {
            IpAddr::V4(_) => IPV4_BITS,
            IpAddr::V6(_) => IPV6_BITS,
        };
        let host_bits: u8 = bits.saturating_sub(self.prefix);

        // 2^128 does not fit in u128
        if bits == IPV6_BITS && host_bits == IPV6_BITS {
            return u128::MAX;
        }

        1u128 << host_bits
    }

    /// Always `false`: a [Cidr] holds at least one address (a /32 or /128 holds one).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Number of IP addresses contained by this [Cidr] if IPv4, else None.
    pub fn len_v4(&self) -> Option<usize> {
        if self.is_ipv4() {
            let host_bits: u8 = IPV4_BITS.saturating_sub(self.prefix);
            if host_bits == IPV4_BITS {
                return Some(u32::MAX as usize + 1);
            }
            Some(1usize << host_bits)
        } else {
            None
        }
    }

    /// Returns true if the CIDR represents a single host address.
    pub fn is_host(&self) -> bool {
        match self.addr {
            IpAddr::V4(_) => self.prefix == IPV4_BITS,
            IpAddr::V6(_) => self.prefix == IPV6_BITS,
        }
    }

    pub fn is_ipv4(&self) -> bool {
        matches!(self.addr, IpAddr::V4(_))
    }

    pub fn is_ipv6(&self) -> bool {
        matches!(self.addr, IpAddr::V6(_))
    }

    /**
    Returns an iterator over all [IpAddr]s in the CIDR range.

    NOTE: For large CIDRs (e.g., /0), this can produce a very large number of
    addresses, especially for IPv6. Use with caution. You have been warned.
    */
    pub fn iter(&self) -> IpIterator {
        let range: Range = cidr_to_range(*self);

        debug_assert_eq!(
            range.len(),
            self.len(),
            "Cidr::iter: length mismatch between 'Cidr' and 'Range' structs"
        );

        IpIterator::new(range)
    }
}

impl IntoIterator for Cidr {
    type Item = IpAddr;
    type IntoIter = IpIterator;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{SLASH}{}", self.addr, self.prefix)
    }
}

impl FromStr for Cidr {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.contains(SLASH) {
            let addr: IpAddr = s
                .trim()
                .parse::<IpAddr>()
                .map_err(|_| AddressError::InvalidAddr(s.into()))?;
            return Ok(Cidr {
                addr,
                prefix: match addr {
                    IpAddr::V4(_) => IPV4_BITS,
                    IpAddr::V6(_) => IPV6_BITS,
                },
            });
        }

        let parts: Vec<&str> = s.split(SLASH).collect();
        if parts.len() != 2 {
            return Err(AddressError::InvalidCidrFmt(s.into()));
        }

        let addr: &str = parts[0].trim();
        let prefix: &str = parts[1].trim();

        let addr: IpAddr = addr
            .parse::<IpAddr>()
            .map_err(|_| AddressError::InvalidCidrAddr(addr.into()))?;

        let prefix: u8 = prefix
            .parse::<u8>()
            .map_err(|_| AddressError::InvalidCidrPrefix(prefix.into()))?;

        match addr {
            IpAddr::V4(_) => {
                if prefix > IPV4_BITS {
                    return Err(AddressError::InvalidV4Prefix(prefix));
                }
            }
            IpAddr::V6(_) => {
                if prefix > IPV6_BITS {
                    return Err(AddressError::InvalidV6Prefix(prefix));
                }
            }
        }

        Ok(Cidr { addr, prefix })
    }
}

/* -------------------------------------------------------------------------- */

/**
Inclusive range of IP addresses (endpoints are included).

Construct with [IpRange::new], which guarantees that both ends are of the
same IP family and `beg <= end`. The fields are not public so that this
invariant cannot be bypassed.
*/
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct IpRange {
    pub(crate) beg: IpAddr,
    pub(crate) end: IpAddr,
}

impl IpRange {
    /// Create a new [IpRange]. Ensures that IP families match and order is correct.
    pub fn new(beg: IpAddr, end: IpAddr) -> Result<Self, AddressError> {
        // Validate same IP version
        match (beg, end) {
            (IpAddr::V4(_), IpAddr::V6(_)) | (IpAddr::V6(_), IpAddr::V4(_)) => {
                return Err(AddressError::Mismatch(beg, end));
            }
            _ => {}
        }

        // Validate order
        if beg > end {
            return Err(AddressError::RangeOrder(beg, end));
        }

        Ok(Self { beg, end })
    }

    /// First address of the range.
    pub fn beg(&self) -> IpAddr {
        self.beg
    }

    /// Last address of the range (inclusive).
    pub fn end(&self) -> IpAddr {
        self.end
    }

    pub fn len(&self) -> u128 {
        assert!(self.beg <= self.end, "{PANIC_NAUGHTY}");
        match (self.beg, self.end) {
            (IpAddr::V4(beg_v4), IpAddr::V4(end_v4)) => {
                (u32::from(end_v4) - u32::from(beg_v4)) as u128 + 1
            }
            (IpAddr::V6(beg_v6), IpAddr::V6(end_v6)) => {
                let beg = u128::from(beg_v6);
                let end = u128::from(end_v6);
                end.saturating_sub(beg).saturating_add(1)
            }
            _ => unreachable!("{ERR_MISMATCH}"),
        }
    }

    /// Always `false`: an [IpRange] holds at least one address (`beg == end`).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Return an iterator over all [IpAddr]s in the range.
    pub fn iter(&self) -> IpIterator {
        IpIterator::new(Range::from(*self))
    }
}

impl IntoIterator for IpRange {
    type Item = IpAddr;
    type IntoIter = IpIterator;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<IpRange> for Range {
    fn from(r: IpRange) -> Self {
        match (r.beg, r.end) {
            (IpAddr::V4(a), IpAddr::V4(b)) => Range {
                fam: IpFam::V4,
                beg: u32::from(a) as u128,
                end: u32::from(b) as u128,
            },
            (IpAddr::V6(a), IpAddr::V6(b)) => Range {
                fam: IpFam::V6,
                beg: u128::from(a),
                end: u128::from(b),
            },
            // IpRange::new() rejects mixed families
            _ => unreachable!("{ERR_MISMATCH}"),
        }
    }
}

/* ---------------------------------- */

/**
Iterator over all [IpAddr]s of a [Cidr] or an [IpRange], in ascending order.

Walks plain integers internally and stops *on* the last address instead of
stepping past it, so ranges ending at the top of the address space
(`255.255.255.255`, `ffff:...:ffff`) terminate too.
*/
#[derive(Clone, Debug)]
pub struct IpIterator {
    fam: IpFam,
    current: u128,
    end: u128,
    done: bool,
}

impl IpIterator {
    /// `r` must satisfy `r.beg <= r.end`.
    pub(crate) fn new(r: Range) -> Self {
        debug_assert!(r.beg <= r.end, "{PANIC_NAUGHTY}");
        IpIterator {
            fam: r.fam,
            current: r.beg,
            end: r.end,
            done: false,
        }
    }
}

impl Iterator for IpIterator {
    type Item = IpAddr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let ip: IpAddr = int_to_ip(self.fam, self.current);

        /*
        Stop on reaching 'end' rather than stepping past it: when 'end' is the
        top of the v6 space (ffff:...:ffff), 'current + 1' does not exist and
        a saturating step would yield the last address forever.
        */
        if self.current == self.end {
            self.done = true;
        } else {
            self.current += 1;
        }

        Some(ip)
    }

    /// Exact whenever the remaining count fits in a [usize] (so `collect()` preallocates).
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done {
            return (0, Some(0));
        }
        // end - current + 1 overflows only for the full v6 space
        match (self.end - self.current).checked_add(1).map(usize::try_from) {
            Some(Ok(n)) => (n, Some(n)),
            _ => (usize::MAX, None),
        }
    }
}

impl FusedIterator for IpIterator {}

/* -------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    const TEST_V4: &str = "192.168.1.0/30";
    const TEST_V6: &str = "::/126";
    const TEST_LEN: &str = "10.0.0.0/8";
    // CIDRs containing ffff:...:ffff, and how many addresses each holds
    const TEST_TOP_V6: [(&str, usize); 2] = [
        ("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128", 1),
        ("ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffc/126", 4),
    ];
    // ranges ending at the top of the address space: (beg, end, count)
    const TEST_TOP_RANGE: [(&str, &str, usize); 2] = [
        ("255.255.255.250", "255.255.255.255", 6),
        ("ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffd", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", 3),
    ];

    #[test]
    fn test_cidr_parse_v4() {
        let cidr = TEST_V4.parse::<Cidr>();
        assert!(cidr.is_ok());
        let cidr = cidr.unwrap();
        assert_eq!(cidr.addr, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0)));
        assert_eq!(cidr.prefix, 30);
        assert_eq!(cidr.to_string(), TEST_V4);
    }

    #[test]
    fn test_cidr_parse_v6() {
        let cidr = TEST_V6.parse::<Cidr>();
        assert!(cidr.is_ok());
        let cidr = cidr.unwrap();
        assert_eq!(cidr.addr, IpAddr::V6(Ipv6Addr::from(0u128)));
        assert_eq!(cidr.prefix, 126);
        assert_eq!(cidr.to_string(), TEST_V6);
    }

    #[test]
    fn test_lengths_agree() {
        let cidr: Cidr = TEST_LEN.parse().unwrap();
        let range: Range = cidr_to_range(cidr);
        assert_eq!(range.len(), cidr.len());
        assert_eq!(range.len(), 2u128.pow((IPV4_BITS - cidr.prefix) as u32));
    }

    #[test]
    fn test_cidr_iter_v4() {
        let cidr: Cidr = TEST_V4.parse().unwrap();
        let ips: Vec<IpAddr> = cidr.iter().collect();
        let expected: Vec<IpAddr> = vec![
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 3)),
        ];
        assert_eq!(ips, expected);
    }

    #[test]
    fn test_cidr_iter_v6() {
        let cidr: Cidr = TEST_V6.parse().unwrap();
        let ips: Vec<IpAddr> = cidr.iter().collect();
        let expected: Vec<IpAddr> = vec![
            IpAddr::V6(Ipv6Addr::from(0u128)),
            IpAddr::V6(Ipv6Addr::from(1u128)),
            IpAddr::V6(Ipv6Addr::from(2u128)),
            IpAddr::V6(Ipv6Addr::from(3u128)),
        ];
        assert_eq!(ips, expected);
    }

    #[test]
    fn test_cidr_iter_top_v6() {
        for (input, count) in TEST_TOP_V6 {
            let cidr: Cidr = input.parse().unwrap();
            // bounded with take(): this iterator used to never terminate here
            assert_eq!(cidr.iter().take(count + 5).count(), count, "Failed: '{input}'");
        }
    }

    #[test]
    fn test_iprange_iter_v4() {
        let ip_range: IpRange = IpRange::new(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
        )
        .unwrap();
        let ips: Vec<IpAddr> = ip_range.iter().collect();
        let expected: Vec<IpAddr> = vec![
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
        ];
        assert_eq!(ips, expected);
    }

    #[test]
    fn test_iprange_iter_v6() {
        let ip_range: IpRange = IpRange::new(
            IpAddr::V6(Ipv6Addr::from(1u128)),
            IpAddr::V6(Ipv6Addr::from(5u128)),
        )
        .unwrap();
        let ips: Vec<IpAddr> = ip_range.iter().collect();
        let expected: Vec<IpAddr> = vec![
            IpAddr::V6(Ipv6Addr::from(1u128)),
            IpAddr::V6(Ipv6Addr::from(2u128)),
            IpAddr::V6(Ipv6Addr::from(3u128)),
            IpAddr::V6(Ipv6Addr::from(4u128)),
            IpAddr::V6(Ipv6Addr::from(5u128)),
        ];
        assert_eq!(ips, expected);
    }

    #[test]
    fn test_cidr_parse_errors() {
        // (input, expected error, its Display: same text as the old String errors)
        #[rustfmt::skip]
        let tests: Vec<(&str, AddressError, &str)> = vec![
            ("nope",         AddressError::InvalidAddr("nope".into()),        "invalid IP address: 'nope'"),
            ("1.2.3.4/8/8",  AddressError::InvalidCidrFmt("1.2.3.4/8/8".into()), "invalid CIDR format (too many slashes): '1.2.3.4/8/8'"),
            ("x/8",          AddressError::InvalidCidrAddr("x".into()),       "invalid IP address in CIDR: 'x'"),
            ("10.0.0.0/300", AddressError::InvalidCidrPrefix("300".into()),   "invalid prefix in CIDR: '300'"),
            ("10.0.0.0/33",  AddressError::InvalidV4Prefix(33),               "invalid IPv4 prefix in CIDR: '33'"),
            ("::/129",       AddressError::InvalidV6Prefix(129),              "invalid IPv6 prefix in CIDR: '129'"),
        ];

        for (input, expected, msg) in tests {
            let err: AddressError = input.parse::<Cidr>().unwrap_err();
            assert_eq!(err, expected, "Failed: '{input}'");
            assert_eq!(err.to_string(), msg, "Failed: '{input}'");
        }
    }

    #[test]
    fn test_iprange_accessors_and_errors() {
        let (a, b): (IpAddr, IpAddr) = ("10.0.0.1".parse().unwrap(), "10.0.0.9".parse().unwrap());
        let r: IpRange = IpRange::new(a, b).unwrap();
        assert_eq!((r.beg(), r.end(), r.len()), (a, b, 9));

        assert_eq!(IpRange::new(b, a), Err(AddressError::RangeOrder(b, a)));
        // mismatch keeps the caller's argument order
        let v6: IpAddr = "::1".parse().unwrap();
        assert_eq!(IpRange::new(v6, a), Err(AddressError::Mismatch(v6, a)));
        assert_eq!(IpRange::new(a, v6), Err(AddressError::Mismatch(a, v6)));
    }

    #[test]
    fn test_iprange_iter_top() {
        for (beg, end, count) in TEST_TOP_RANGE {
            let r: IpRange = IpRange::new(beg.parse().unwrap(), end.parse().unwrap()).unwrap();
            let ips: Vec<IpAddr> = r.iter().take(count + 5).collect();
            assert_eq!(ips.len(), count, "Failed: '{beg}-{end}'");
            assert_eq!(ips[count - 1], end.parse::<IpAddr>().unwrap());
        }
    }

    #[test]
    fn test_iter_size_hint() {
        let cidr: Cidr = TEST_V4.parse().unwrap();
        let mut it: IpIterator = cidr.iter();
        assert_eq!(it.size_hint(), (4, Some(4)));
        it.next();
        assert_eq!(it.size_hint(), (3, Some(3)));
        assert_eq!(it.by_ref().count(), 3);
        assert_eq!(it.size_hint(), (0, Some(0)));
        assert_eq!(it.next(), None); // fused

        // full v6 space: 2^128 addresses cannot be counted in a usize
        let all: Cidr = "::/0".parse().unwrap();
        assert_eq!(all.iter().size_hint(), (usize::MAX, None));
    }
}
