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
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
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
    pub fn iter(&self) -> CidrIterator {
        CidrIterator::new(*self)
    }
}

impl IntoIterator for Cidr {
    type Item = IpAddr;
    type IntoIter = CidrIterator;

    fn into_iter(self) -> Self::IntoIter {
        CidrIterator::new(self)
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{SLASH}{}", self.addr, self.prefix)
    }
}

impl FromStr for Cidr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.contains(SLASH) {
            let addr: IpAddr = s
                .trim()
                .parse::<IpAddr>()
                .map_err(|_| format!("{ERR_INV_ADDR}: '{s}'"))?;
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
            return Err(format!("{ERR_CIDR_FMT}: '{s}'"));
        }

        let addr: &str = parts[0].trim();
        let prefix: &str = parts[1].trim();

        let addr: IpAddr = addr
            .parse::<IpAddr>()
            .map_err(|_| format!("{ERR_CIDR_INV_ADDR}: '{addr}'"))?;

        let prefix: u8 = prefix
            .parse::<u8>()
            .map_err(|_| format!("{ERR_CIDR_INV_PRE}: '{prefix}'"))?;

        match addr {
            IpAddr::V4(_) => {
                if prefix > IPV4_BITS {
                    return Err(format!("{ERR_CIDR_INV_V4}: '{prefix}'"));
                }
            }
            IpAddr::V6(_) => {
                if prefix > IPV6_BITS {
                    return Err(format!("{ERR_CIDR_INV_V6}: '{prefix}'"));
                }
            }
        }

        Ok(Cidr { addr, prefix })
    }
}

/* ---------------------------------- */

/// Iterator over all [IpAddr]s in a CIDR range.
pub struct CidrIterator {
    fam: IpFam,
    current: u128,
    end: u128,
}

impl CidrIterator {
    pub fn new(cidr: Cidr) -> Self {
        let range: Range = cidr_to_range(cidr);

        debug_assert_eq!(
            range.len(),
            cidr.len(),
            "CidrIterator: length mismatch between 'Cidr' and 'Range' structs"
        );

        CidrIterator {
            fam: range.fam,
            current: range.beg,
            end: range.end,
        }
    }
}

impl Iterator for CidrIterator {
    type Item = IpAddr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current > self.end {
            return None;
        }

        let ip: IpAddr = int_to_ip(self.fam, self.current);
        self.current = self.current.saturating_add(1);

        Some(ip)
    }
}

/* -------------------------------------------------------------------------- */

/// Inclusive range of IP addresses (endpoints are included).
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct IpRange {
    pub beg: IpAddr,
    pub end: IpAddr,
}

impl IpRange {
    /// Create a new [IpRange]. Ensures that IP families match and order is correct.
    pub fn new(beg: IpAddr, end: IpAddr) -> Result<Self, AddressError> {
        // Validate same IP version
        match (beg, end) {
            (IpAddr::V4(a), IpAddr::V6(b)) | (IpAddr::V6(b), IpAddr::V4(a)) => {
                return Err(AddressError::Mismatch(a.into(), b.into()));
            }
            _ => {}
        }

        // Validate order
        if beg > end {
            return Err(AddressError::RangeOrder(beg, end));
        }

        Ok(Self { beg, end })
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

    /// Return an iterator over all [IpAddr]s in the range.
    pub fn iter(&self) -> IpRangeIterator {
        IpRangeIterator {
            current: self.beg,
            end: self.end,
            done: false,
        }
    }
}

impl IntoIterator for IpRange {
    type Item = IpAddr;
    type IntoIter = IpRangeIterator;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/* ---------------------------------- */

/// Iterator over an IP range.
pub struct IpRangeIterator {
    current: IpAddr,
    end: IpAddr,
    done: bool,
}

impl Iterator for IpRangeIterator {
    type Item = IpAddr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let result = self.current;

        if self.current == self.end {
            self.done = true;
        } else {
            self.current = match self.current {
                IpAddr::V4(ipv4) => IpAddr::V4(Ipv4Addr::from(u32::from(ipv4).saturating_add(1))),
                IpAddr::V6(ipv6) => IpAddr::V6(Ipv6Addr::from(u128::from(ipv6).saturating_add(1))),
            };
        }

        Some(result)
    }
}

/* -------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_V4: &str = "192.168.1.0/30";
    const TEST_V6: &str = "::/126";
    const TEST_LEN: &str = "10.0.0.0/8";

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
}
