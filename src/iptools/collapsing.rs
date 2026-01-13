// Copyright (c) 2026 Mikko Tanner. All rights reserved.
// Licensed under the MIT License or the Apache License, Version 2.0.
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::{
    strings::*,
    structs::{Cidr, IpFam, IpRange, Range},
    AddressError, IPV4_BITS, IPV6_BITS,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/**
Collapse a list of CIDRs into an equivalent, minimal set of CIDRs.
- removes redundant sub-prefixes
- merges adjacent/overlapping ranges

Input CIDRs may be non-normalized host addresses; they will be normalized
to the network address.

If `max_gap` > 0, nearby ranges separated by <= `max_gap` IPs will be
fuzzily merged as well (over-approximation).
*/
pub fn collapse_cidrs(input: &[Cidr], max_gap: u128) -> Vec<Cidr> {
    let mut ranges: Vec<Range> = input.iter().map(|c| cidr_to_range(*c)).collect();

    // 1) Sort ranges
    ranges.sort_by(|a, b| a.cmp_key().cmp(&b.cmp_key()));

    // 2) Merge overlaps/adjacent within each family
    let mut merged: Vec<Range> = merge_ranges(&ranges);

    // 2b) Fuzzy merge nearby with gaps <= max_gap
    if max_gap > 0 {
        merged = merge_ranges_fuzzy(&merged, max_gap);
    }

    // 3) Convert each merged range back into minimal CIDRs
    let mut out: Vec<Cidr> = Vec::new();
    for r in merged {
        out.extend(range_to_cidrs(r));
    }
    out
}

/**
Collapse a list of IPs into an equivalent, minimal set of CIDRs.

If `max_gap` > 0, nearby ranges separated by <= `max_gap` IPs will be
fuzzily merged as well (over-approximation).
*/
pub fn collapse_ips(input: &[IpAddr], max_gap: u128) -> Vec<Cidr> {
    let cidrs: Vec<Cidr> = input.iter().map(|&ip| ip_to_host_cidr(ip)).collect();
    collapse_cidrs(&cidrs, max_gap)
}

/**
Collapse a list of strings (CIDRs or IPs) into an equivalent, minimal set of CIDRs.

If `max_gap` > 0, nearby ranges separated by <= `max_gap` IPs will be
fuzzily merged as well (over-approximation).
*/
pub fn collapse_strings(input: &[impl AsRef<str>], max_gap: u128) -> Vec<Cidr> {
    let mut cidrs: Vec<Cidr> = Vec::with_capacity(input.len());
    for s in input {
        if s.as_ref().contains(SLASH) {
            if let Ok(cidr) = s.as_ref().parse::<Cidr>() {
                cidrs.push(cidr);
            }
        } else if let Ok(ip) = s.as_ref().parse::<IpAddr>() {
            cidrs.push(ip_to_host_cidr(ip));
        }
    }
    collapse_cidrs(&cidrs, max_gap)
}

/// Convert a single IP (host) to an equivalent CIDR (/32 or /128).
pub fn ip_to_host_cidr(ip: IpAddr) -> Cidr {
    match ip {
        IpAddr::V4(_) => Cidr {
            addr: ip,
            prefix: IPV4_BITS,
        },
        IpAddr::V6(_) => Cidr {
            addr: ip,
            prefix: IPV6_BITS,
        },
    }
}

/**
Collapse a list of inclusive IP ranges into an equivalent, minimal set of CIDRs.

This does *not* enumerate IPs and hence scales to very large ranges.
*/
pub fn collapse_ranges(input: &[IpRange]) -> Result<Vec<Cidr>, AddressError> {
    let mut ranges: Vec<Range> = Vec::with_capacity(input.len());

    for r in input.iter().copied() {
        let rr: Range = iprange_to_range(r)?;
        ranges.push(rr);
    }

    // 1) Sort ranges
    ranges.sort_by(|a, b| a.cmp_key().cmp(&b.cmp_key()));

    // 2) Merge overlaps/adjacent within each family
    let merged: Vec<Range> = merge_ranges(&ranges);

    // 3) Convert merged ranges to minimal CIDRs
    let mut out: Vec<Cidr> = Vec::new();
    for r in merged {
        out.extend(range_to_cidrs(r));
    }
    Ok(out)
}

/**
Collapse a list of inclusive IP ranges into an equivalent, minimal set of CIDRs.

Fuzzily merges nearby ranges separated by <= `max_gap` IPs (over-approximation).
*/
pub fn collapse_ranges_fuzzy(input: &[IpRange], max_gap: u128) -> Result<Vec<Cidr>, AddressError> {
    let mut ranges: Vec<Range> = Vec::with_capacity(input.len());

    for r in input.iter().copied() {
        let rr: Range = iprange_to_range(r)?;
        ranges.push(rr);
    }

    // 1) Sort ranges
    ranges.sort_by(|a, b| a.cmp_key().cmp(&b.cmp_key()));

    // 2) Merge overlaps/adjacent within each family
    let mut merged: Vec<Range> = merge_ranges(&ranges);

    // 2b) Fuzzy merge nearby with gaps <= max_gap
    merged = merge_ranges_fuzzy(&merged, max_gap);

    // 3) Convert merged ranges to minimal CIDRs
    let mut out: Vec<Cidr> = Vec::with_capacity(merged.len());
    for r in merged {
        out.extend(range_to_cidrs(r));
    }
    Ok(out)
}

/// Convenience overload for call sites which have tuples.
pub fn collapse_ranges_tuples(input: &[(IpAddr, IpAddr)]) -> Result<Vec<Cidr>, AddressError> {
    let v: Vec<IpRange> = input
        .iter()
        .copied()
        .map(|(beg, end)| IpRange::new(beg, end))
        .collect::<Result<Vec<IpRange>, AddressError>>()?;
    collapse_ranges(&v)
}

/* ---------------------------------- */

/// Convert a CIDR to an inclusive range.
pub(crate) fn cidr_to_range(c: Cidr) -> Range {
    match c.addr {
        IpAddr::V4(a) => {
            let pre: u8 = c.prefix.min(IPV4_BITS);
            let ip: u32 = u32::from_be_bytes(a.octets());
            let mask: u32 = mask_u128(IPV4_BITS, pre) as u32;
            let net: u32 = ip & mask;
            let end: u32 = net | !mask;
            Range {
                fam: IpFam::V4,
                beg: net as u128,
                end: end as u128,
            }
        }
        IpAddr::V6(a) => {
            let pre: u8 = c.prefix.min(IPV6_BITS);
            let ip: u128 = u128::from_be_bytes(a.octets());
            let mask: u128 = mask_u128(IPV6_BITS, pre);
            let net: u128 = ip & mask;
            let end: u128 = net | !mask;
            Range {
                fam: IpFam::V6,
                beg: net,
                end,
            }
        }
    }
}

/// Merge overlapping/adjacent ranges within each IP family. Input must be sorted.
#[inline]
fn merge_ranges(sorted: &[Range]) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::with_capacity(sorted.len());
    for r in sorted.iter().copied() {
        if let Some(last) = out.last_mut() {
            if last.fam == r.fam {
                // overlap or adjacency?
                if r.beg <= last.end.saturating_add(1) {
                    if r.end > last.end {
                        last.end = r.end;
                    }
                    continue;
                }
            }
        }
        out.push(r);
    }
    out
}

/**
Merge nearby ranges separated by <= `max_gap` IPs (fuzzy over-approximation).

Input must be sorted and previously merged, or it'll be a GIGO situation.
*/
#[inline]
fn merge_ranges_fuzzy(merged: &[Range], max_gap: u128) -> Vec<Range> {
    let mut out: Vec<Range> = Vec::with_capacity(merged.len());
    for r in merged.iter().copied() {
        if let Some(last) = out.last_mut() {
            if last.fam == r.fam {
                let gap: u128 = r.beg.saturating_sub(last.end.saturating_add(1));
                if gap <= max_gap {
                    // swallow the gap by extending end
                    last.end = r.end.max(last.end);
                    continue;
                }
            }
        }
        out.push(r);
    }
    out
}

/// Decompose an inclusive range into the minimal set of CIDRs.
fn range_to_cidrs(r: Range) -> Vec<Cidr> {
    let bits: u8 = match r.fam {
        IpFam::V4 => IPV4_BITS,
        IpFam::V6 => IPV6_BITS,
    };

    // Full address space special-case
    if bits == IPV6_BITS && r.beg == 0 && r.end == u128::MAX {
        #[rustfmt::skip]
        return vec![Cidr { addr: IpAddr::V6(Ipv6Addr::UNSPECIFIED), prefix: 0 }];
    }

    let mut start: u128 = r.beg;
    let end: u128 = r.end;
    let mut out: Vec<Cidr> = Vec::new();

    while start <= end {
        /*
        Largest block aligned at 'start' (power-of-two size).
        If start==0, trailing_zeros is max; handle by setting
        alignment to bits.
        */
        let tz: u8 = start.trailing_zeros() as u8;
        let max_align_prefix: u8 = bits.saturating_sub(tz.min(bits));

        // largest block that fits in remaining range length
        let remaining: u128 = (end - start).saturating_add(1);
        let max_fit_prefix: u8 = bits - floor_log2_u128(remaining);

        let prefix: u8 = max_align_prefix.max(max_fit_prefix);

        // emit CIDR
        out.push(Cidr {
            addr: int_to_ip(r.fam, start),
            prefix,
        });

        // prefix==0 for v6 should have been caught by the full-space
        // special case above; but keep this guard anyway.
        if bits == IPV6_BITS && prefix == 0 {
            break;
        }

        // advance start by block size = 2^(bits-prefix)
        // pow is <= 128; for v6, pow==128 would imply prefix==0, but we guard above
        let block_size_pow: u32 = (bits - prefix) as u32;
        let block_size: u128 = 1u128 << block_size_pow;
        start = start.saturating_add(block_size);
    }

    out
}

/// Convert an [IpRange] to a [Range].
fn iprange_to_range(r: IpRange) -> Result<Range, AddressError> {
    match (r.beg, r.end) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let aa: u32 = u32::from_be_bytes(a.octets());
            let bb: u32 = u32::from_be_bytes(b.octets());
            let (beg, end) = if aa <= bb { (aa, bb) } else { (bb, aa) };
            Ok(Range {
                fam: IpFam::V4,
                beg: beg as u128,
                end: end as u128,
            })
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let aa: u128 = u128::from_be_bytes(a.octets());
            let bb: u128 = u128::from_be_bytes(b.octets());
            let (beg, end) = if aa <= bb { (aa, bb) } else { (bb, aa) };
            Ok(Range {
                fam: IpFam::V6,
                beg,
                end,
            })
        }
        (beg, end) => Err(AddressError::Mismatch(beg, end)),
    }
}

/* ---------------------------------- */

/**
Returns a u128 with prefix high bits set, remaining low bits zero.

bits: 32 or 128, prefix: `0..=bits`
*/
#[inline]
fn mask_u128(bits: u8, prefix: u8) -> u128 {
    if prefix == 0 {
        return 0;
    }
    if prefix >= bits {
        return !0u128;
    }
    /*
    Example (bits=32,prefix=24): top 24 bits 1, low 8 bits 0.
    Create full-ones in 'bits' width, then clear low (bits-prefix) bits.
    */
    let all: u128 = if bits == IPV6_BITS {
        !0u128
    } else {
        (1u128 << bits) - 1
    };
    let low: u8 = bits - prefix;
    all & (!((1u128 << low) - 1))
}

/// floor(log2(x)) for x>=1, returns in [0..127]
#[inline]
fn floor_log2_u128(x: u128) -> u8 {
    debug_assert!(x >= 1);
    127u8.saturating_sub(x.leading_zeros() as u8)
}

#[inline]
pub(crate) fn int_to_ip(fam: IpFam, v: u128) -> IpAddr {
    match fam {
        IpFam::V4 => IpAddr::V4(Ipv4Addr::from((v as u32).to_be_bytes())),
        IpFam::V6 => IpAddr::V6(Ipv6Addr::from(v.to_be_bytes())),
    }
}

/* -------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    const TST_A_1: &str = "192.168.0.0";
    const TST_A_2: &str = "192.168.1.0";
    const RES_T_A: &str = "192.168.0.0/23";

    const TST_B_1: &str = "10.0.0.0";
    const TST_B_2: &str = "10.1.2.0";
    const RES_T_B: &str = "10.0.0.0/8";

    const TST_C_1: &str = "2001:db8::";
    const TST_C_2: &str = "2001:db8:0:0:8000::";
    const RES_T_C: &str = "2001:db8::/64";

    const TST_D_V4: [&str; 4] = ["172.16.0.4", "172.16.0.5", "172.16.0.6", "172.16.0.7"];
    const RES_D_V4: &str = "172.16.0.4/30";

    const TST_D_V6: [&str; 4] = ["2001:db8::4", "2001:db8::5", "2001:db8::6", "2001:db8::7"];
    const RES_D_V6: &str = "2001:db8::4/126";

    const TST_E_V4: [&str; 4] = ["172.16.0.8", "172.16.0.11", "172.16.0.13", "172.16.0.15"];
    const RES_E_V4: &str = "172.16.0.8/29";

    const TST_E_V6: [&str; 4] = ["2001:db8::0", "2001:db8::3", "2001:db8::5", "2001:db8::7"];
    const RES_E_V6: &str = "2001:db8::/125";

    #[test]
    fn test_merges_adjacent_v4() {
        let input = [
            Cidr {
                addr: IpAddr::V4(TST_A_1.parse().unwrap()),
                prefix: 24,
            },
            Cidr {
                addr: IpAddr::V4(TST_A_2.parse().unwrap()),
                prefix: 24,
            },
        ];
        let out = collapse_cidrs(&input, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_T_A);
    }

    #[test]
    fn test_removes_redundant() {
        let input = [
            Cidr {
                addr: IpAddr::V4(TST_B_1.parse().unwrap()),
                prefix: 8,
            },
            Cidr {
                addr: IpAddr::V4(TST_B_2.parse().unwrap()),
                prefix: 24,
            },
        ];
        let out = collapse_cidrs(&input, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_T_B);
    }

    #[test]
    fn test_handles_ipv6_merge() {
        let input = [
            Cidr {
                addr: IpAddr::V6(TST_C_1.parse().unwrap()),
                prefix: 65,
            },
            Cidr {
                addr: IpAddr::V6(TST_C_2.parse().unwrap()),
                prefix: 65,
            },
        ];
        let out = collapse_cidrs(&input, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_T_C);
    }

    #[test]
    fn test_range_to_cidr() {
        let r = Range {
            fam: IpFam::V4,
            beg: 172u128 << 24 | 16u128 << 16 | 0u128 << 8 | 4u128,
            end: 172u128 << 24 | 16u128 << 16 | 0u128 << 8 | 7u128,
        };
        let cidrs = range_to_cidrs(r);
        let cidr_strs: Vec<String> = cidrs.iter().map(|c| c.to_string()).collect();
        assert_eq!(cidr_strs[0], RES_D_V4.to_string());
    }

    #[test]
    fn test_ip_to_host_cidr_v4() {
        let input: Vec<Cidr> = TST_D_V4
            .iter()
            .map(|s| ip_to_host_cidr(s.parse().unwrap()))
            .collect();
        let out = collapse_cidrs(&input, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_D_V4);
    }

    #[test]
    fn test_ip_to_host_cidr_v6() {
        let input: Vec<Cidr> = TST_D_V6
            .iter()
            .map(|s| ip_to_host_cidr(s.parse().unwrap()))
            .collect();
        let out = collapse_cidrs(&input, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_D_V6);
    }

    #[test]
    fn test_cidr_iter_v4() {
        let cidr = collapse_strings(
            TST_D_V4
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .as_slice(),
            0,
        )[0];
        let ip_strs: Vec<String> = cidr.iter().map(|c| c.to_string()).collect();
        let expected: Vec<String> = TST_D_V4.iter().map(|s| s.to_string()).collect();
        assert_eq!(ip_strs, expected);
    }

    #[test]
    fn test_cidr_iter_v6() {
        let cidr = collapse_strings(
            TST_D_V6
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .as_slice(),
            0,
        )[0];
        let ip_strs: Vec<String> = cidr.iter().map(|c| c.to_string()).collect();
        let expected: Vec<String> = TST_D_V6.iter().map(|s| s.to_string()).collect();
        assert_eq!(ip_strs, expected);
    }

    #[test]
    fn test_fuzz_v4() {
        let input: Vec<Cidr> = TST_E_V4
            .iter()
            .map(|s| ip_to_host_cidr(s.parse().unwrap()))
            .collect();
        let out = collapse_cidrs(&input, 2);
        eprintln!("{:?}", out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_E_V4);
    }

    #[test]
    fn test_fuzz_v6() {
        let input: Vec<Cidr> = TST_E_V6
            .iter()
            .map(|s| ip_to_host_cidr(s.parse().unwrap()))
            .collect();
        let out = collapse_cidrs(&input, 2);
        eprintln!("{:?}", out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), RES_E_V6);
    }
}
