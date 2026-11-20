//! The addresses outside the internet: one list, [`RANGES`], that a fetch
//! an extension asks for refuses unless the address is named literally, and
//! that every later client a script, a server or a configuration can point
//! somewhere takes from here rather than copies (`docs/security.md`,
//! section 2).
//!
//! The list is what IANA's special-purpose address registries mark as not
//! reachable on the internet, with the machine's own addresses, the local
//! networks' and the clouds' metadata services: [`classify`] gives the kind
//! of the longest range that holds an address. An IPv6 address that stands
//! for an IPv4 one is the kind of that IPv4 address: the IPv4-mapped form
//! (`::ffff:a.b.c.d`), the translated form (`::ffff:0:a.b.c.d`), the
//! deprecated IPv4-compatible form (`::a.b.c.d`), NAT64's well-known prefix
//! (`64:ff9b::a.b.c.d`), which a network with DNS64 answers names with, and
//! 6to4 (`2002:aabb:ccdd::`). A network's own NAT64 prefix cannot be known
//! from here (a network tells its hosts of it, RFC 7050), nor can an ISATAP
//! address's (`prefix::5efe:a.b.c.d`): an address under one is read as the
//! internet. Nor are the machine's own addresses, or its network's, when
//! they are addresses of the internet (a dual-stack network's IPv6 ones):
//! nothing here knows them.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// What kind of address outside the internet an address is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressKind {
    /// This machine: `127.0.0.0/8`, `::1`.
    Loopback,
    /// No address: `0.0.0.0/8`, `::`. Connecting to it reaches this machine
    /// on some systems.
    Unspecified,
    /// The local link: `169.254.0.0/16`, `fe80::/10`.
    LinkLocal,
    /// A private network: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
    /// IPv6's deprecated site-local `fec0::/10`, and NAT64's local-use
    /// prefix `64:ff9b:1::/48`, which a network's own translator may map to
    /// its private addresses.
    Private,
    /// A carrier's shared space (carrier-grade NAT): `100.64.0.0/10`.
    Shared,
    /// IPv6's private networks: `fc00::/7`.
    UniqueLocal,
    /// A group: `224.0.0.0/4`, `ff00::/8`.
    Multicast,
    /// The local network's broadcast: `255.255.255.255`.
    Broadcast,
    /// A cloud's metadata service, which hands out the machine's
    /// credentials: `169.254.169.254` (most clouds), `fd00:ec2::254` (AWS),
    /// `168.63.129.16` (Azure's platform endpoint) and `192.0.0.192`
    /// (Oracle's).
    Metadata,
    /// Set aside, and not reached on the internet: the IETF's protocol
    /// assignments, benchmarking, documentation, the future-use block,
    /// IPv6's discard prefix and its segment-routing identifiers. Each block
    /// is refused whole, the few services inside one that are reachable
    /// (an anycast relay, an AS112 server) included.
    Reserved,
}

impl AddressKind {
    /// What the kind is, in words a script's author and a user can read.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Loopback => "a loopback address (this machine)",
            Self::Unspecified => "an unspecified address (this machine, on some systems)",
            Self::LinkLocal => "a link-local address",
            Self::Private => "a private network's address",
            Self::Shared => "a shared address (carrier-grade NAT)",
            Self::UniqueLocal => "a unique local address (a private IPv6 network)",
            Self::Multicast => "a multicast address",
            Self::Broadcast => "the broadcast address",
            Self::Metadata => "a cloud metadata service's address",
            Self::Reserved => "a reserved address, not one of the internet",
        }
    }
}

impl std::fmt::Display for AddressKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}

/// A range of addresses outside the internet: every address whose first
/// `prefix` bits are `network`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    network: IpAddr,
    prefix: u8,
    kind: AddressKind,
}

impl Range {
    /// The range's first address.
    #[must_use]
    pub const fn network(&self) -> IpAddr {
        self.network
    }

    /// How many leading bits the range's addresses share.
    #[must_use]
    pub const fn prefix(&self) -> u8 {
        self.prefix
    }

    /// What kind of address the range holds.
    #[must_use]
    pub const fn kind(&self) -> AddressKind {
        self.kind
    }

    const fn v4(octets: [u8; 4], prefix: u8, kind: AddressKind) -> Self {
        let [a, b, c, d] = octets;
        Self {
            network: IpAddr::V4(Ipv4Addr::new(a, b, c, d)),
            prefix,
            kind,
        }
    }

    const fn v6(segments: [u16; 8], prefix: u8, kind: AddressKind) -> Self {
        let [a, b, c, d, e, f, g, h] = segments;
        Self {
            network: IpAddr::V6(Ipv6Addr::new(a, b, c, d, e, f, g, h)),
            prefix,
            kind,
        }
    }

    /// Whether `address` is in the range: of its family, with its first
    /// `prefix` bits.
    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        let prefix = u32::from(self.prefix);
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
                u32::from(network) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
                u128::from(network) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

/// Every range outside the internet. An address in two is the kind of the
/// longer one: `169.254.169.254` is metadata, the rest of `169.254.0.0/16`
/// link-local. **Decide with [`classify`]**, not with a range's
/// [`contains`](Range::contains): the ranges alone do not see an IPv6 form
/// of an IPv4 address (`::ffff:127.0.0.1` is in none of them).
pub const RANGES: &[Range] = {
    use AddressKind::{
        Broadcast, LinkLocal, Loopback, Metadata, Multicast, Private, Reserved, Shared,
        UniqueLocal, Unspecified,
    };
    &[
        // IPv4.
        Range::v4([0, 0, 0, 0], 8, Unspecified),
        Range::v4([10, 0, 0, 0], 8, Private),
        Range::v4([100, 64, 0, 0], 10, Shared),
        Range::v4([127, 0, 0, 0], 8, Loopback),
        Range::v4([168, 63, 129, 16], 32, Metadata),
        Range::v4([169, 254, 0, 0], 16, LinkLocal),
        Range::v4([169, 254, 169, 254], 32, Metadata),
        Range::v4([172, 16, 0, 0], 12, Private),
        // The IETF's protocol assignments, and Oracle's metadata service
        // among them.
        Range::v4([192, 0, 0, 0], 24, Reserved),
        Range::v4([192, 0, 0, 192], 32, Metadata),
        // Documentation.
        Range::v4([192, 0, 2, 0], 24, Reserved),
        // 6to4's relays, deprecated.
        Range::v4([192, 88, 99, 0], 24, Reserved),
        Range::v4([192, 168, 0, 0], 16, Private),
        // Benchmarking.
        Range::v4([198, 18, 0, 0], 15, Reserved),
        // Documentation.
        Range::v4([198, 51, 100, 0], 24, Reserved),
        Range::v4([203, 0, 113, 0], 24, Reserved),
        Range::v4([224, 0, 0, 0], 4, Multicast),
        // Set aside for future use.
        Range::v4([240, 0, 0, 0], 4, Reserved),
        Range::v4([255, 255, 255, 255], 32, Broadcast),
        // IPv6.
        Range::v6([0, 0, 0, 0, 0, 0, 0, 0], 128, Unspecified),
        Range::v6([0, 0, 0, 0, 0, 0, 0, 1], 128, Loopback),
        // NAT64's local-use prefix.
        Range::v6([0x64, 0xff9b, 1, 0, 0, 0, 0, 0], 48, Private),
        // Discard only, and a dummy prefix (RFC 9780).
        Range::v6([0x100, 0, 0, 0, 0, 0, 0, 0], 64, Reserved),
        Range::v6([0x100, 0, 0, 1, 0, 0, 0, 0], 64, Reserved),
        // The IETF's protocol assignments: Teredo, benchmarking, ORCHID.
        Range::v6([0x2001, 0, 0, 0, 0, 0, 0, 0], 23, Reserved),
        // Documentation.
        Range::v6([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32, Reserved),
        Range::v6([0x3fff, 0, 0, 0, 0, 0, 0, 0], 20, Reserved),
        // Segment routing's identifiers.
        Range::v6([0x5f00, 0, 0, 0, 0, 0, 0, 0], 16, Reserved),
        Range::v6([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7, UniqueLocal),
        Range::v6([0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254], 128, Metadata),
        Range::v6([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10, LinkLocal),
        // Site-local, deprecated.
        Range::v6([0xfec0, 0, 0, 0, 0, 0, 0, 0], 10, Private),
        Range::v6([0xff00, 0, 0, 0, 0, 0, 0, 0], 8, Multicast),
    ]
};

/// The kind of `address` when it is outside the internet, and `None` for an
/// address of the internet. An IPv6 form of an IPv4 address is that
/// address's kind (the module's documentation lists the forms).
#[must_use]
pub fn classify(address: IpAddr) -> Option<AddressKind> {
    if let IpAddr::V6(v6) = address
        && let Some(inner) = embedded_v4(v6)
    {
        return classify(IpAddr::V4(inner));
    }
    RANGES
        .iter()
        .filter(|range| range.contains(address))
        .max_by_key(|range| range.prefix)
        .map(|range| range.kind)
}

/// The IPv4 address an IPv6 address stands for, in the forms the module's
/// documentation names. `::` and `::1` are IPv6's own.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let [.., a, b, c, d] = address.octets();
    let last = Ipv4Addr::new(a, b, c, d);
    match address.segments() {
        [0, 0, 0, 0, 0, 0, 0, 0 | 1] => None,
        // IPv4-mapped, then translated, then IPv4-compatible.
        [0, 0, 0, 0, 0, 0xffff, _, _]
        | [0, 0, 0, 0, 0xffff, 0, _, _]
        | [0, 0, 0, 0, 0, 0, _, _]
        // NAT64's well-known prefix.
        | [0x64, 0xff9b, 0, 0, 0, 0, _, _] => Some(last),
        // 6to4: the IPv4 address follows the prefix.
        [0x2002, high, low, ..] => Some(Ipv4Addr::from(
            (u32::from(high) << 16) | u32::from(low),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    fn ip(text: &str) -> IpAddr {
        IpAddr::from_str(text).unwrap()
    }

    #[test]
    fn every_range_starts_at_its_network_and_is_listed_once() {
        for (index, range) in RANGES.iter().enumerate() {
            let bits = match range.network {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            };
            assert!(range.prefix <= bits, "{range:?}");
            // The network has no bit set past its prefix: it is the range's
            // first address.
            let next = match range.network {
                IpAddr::V4(network) => (u32::from(network) as u128)
                    .checked_shl(u32::from(range.prefix))
                    .map(|rest| rest & u128::from(u32::MAX)),
                IpAddr::V6(network) => u128::from(network).checked_shl(u32::from(range.prefix)),
            };
            assert_eq!(next.unwrap_or(0), 0, "{range:?}");
            assert!(range.contains(range.network), "{range:?}");
            for other in &RANGES[index + 1..] {
                assert!(
                    (other.network, other.prefix) != (range.network, range.prefix),
                    "{range:?} twice"
                );
            }
        }
    }

    #[test]
    fn every_range_outside_the_internet_is_named() {
        use AddressKind::*;
        for (address, kind) in [
            ("127.0.0.1", Loopback),
            ("127.255.255.254", Loopback),
            ("::1", Loopback),
            ("0.0.0.0", Unspecified),
            ("0.1.2.3", Unspecified),
            ("::", Unspecified),
            ("169.254.0.1", LinkLocal),
            ("169.254.255.255", LinkLocal),
            ("fe80::1", LinkLocal),
            ("febf::1", LinkLocal),
            ("10.0.0.1", Private),
            ("10.255.255.255", Private),
            ("172.16.0.1", Private),
            ("172.31.255.255", Private),
            ("192.168.1.1", Private),
            ("192.168.255.255", Private),
            ("fec0::1", Private),
            ("64:ff9b:1::a9fe:a9fe", Private),
            ("64:ff9b:1:ffff::1", Private),
            ("100.64.0.1", Shared),
            ("100.127.255.255", Shared),
            ("fc00::1", UniqueLocal),
            ("fd00::1", UniqueLocal),
            ("fdff:ffff::1", UniqueLocal),
            ("224.0.0.1", Multicast),
            ("239.255.255.255", Multicast),
            ("ff02::1", Multicast),
            ("ff0e::1", Multicast),
            ("255.255.255.255", Broadcast),
            ("169.254.169.254", Metadata),
            ("fd00:ec2::254", Metadata),
            ("168.63.129.16", Metadata),
            ("192.0.0.192", Metadata),
            ("192.0.0.1", Reserved),
            ("192.0.2.1", Reserved),
            ("192.88.99.1", Reserved),
            ("198.18.0.1", Reserved),
            ("198.19.255.255", Reserved),
            ("198.51.100.1", Reserved),
            ("203.0.113.7", Reserved),
            ("240.0.0.1", Reserved),
            ("255.255.255.254", Reserved),
            ("100::1", Reserved),
            ("100:0:0:1::1", Reserved),
            ("2001::1", Reserved),
            ("2001:1ff:ffff::1", Reserved),
            ("2001:db8::1", Reserved),
            ("3fff::1", Reserved),
            ("3fff:fff::1", Reserved),
            ("5f00::1", Reserved),
        ] {
            assert_eq!(classify(ip(address)), Some(kind), "{address}");
        }
    }

    #[test]
    fn the_internet_is_not_named_even_at_the_edge_of_a_range() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "1.0.0.0",
            "9.255.255.255",
            "11.0.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "168.63.129.15",
            "168.63.129.17",
            "172.15.255.255",
            "172.32.0.0",
            "191.255.255.255",
            "192.0.1.0",
            "192.0.3.0",
            "192.88.98.255",
            "192.88.100.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "198.51.99.255",
            "198.51.101.0",
            "203.0.112.255",
            "203.0.114.0",
            "100.63.255.255",
            "100.128.0.0",
            "169.253.255.255",
            "169.255.0.0",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "2001:200::1",
            "2001:db7:ffff::1",
            "2001:db9::1",
            "3fff:1000::1",
            "64:ff9b:2::1",
            "100:0:0:2::1",
            "ff::1",
            "fe7f::1",
            "fbff::1",
            "fe00::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
            "2a00:1450::1",
        ] {
            assert_eq!(classify(ip(address)), None, "{address}");
        }
    }

    #[test]
    fn an_ipv6_form_of_an_ipv4_address_is_the_kind_of_that_address() {
        use AddressKind::*;
        for (address, kind) in [
            // IPv4-mapped.
            ("::ffff:127.0.0.1", Loopback),
            ("::ffff:10.0.0.1", Private),
            ("::ffff:169.254.169.254", Metadata),
            ("::ffff:100.64.0.1", Shared),
            ("::ffff:0.0.0.0", Unspecified),
            ("::ffff:203.0.113.1", Reserved),
            // Translated.
            ("::ffff:0:192.168.0.1", Private),
            // IPv4-compatible, deprecated.
            ("::127.0.0.1", Loopback),
            ("::10.0.0.1", Private),
            ("::169.254.169.254", Metadata),
            ("::0.0.0.2", Unspecified),
            // NAT64's well-known prefix.
            ("64:ff9b::127.0.0.1", Loopback),
            ("64:ff9b::a9fe:a9fe", Metadata),
            ("64:ff9b::c0a8:101", Private),
            // 6to4.
            ("2002:7f00:1::", Loopback),
            ("2002:a9fe:a9fe::1", Metadata),
            ("2002:a00:5:1::1", Private),
            ("2002:a83f:8110::", Metadata),
        ] {
            assert_eq!(classify(ip(address)), Some(kind), "{address}");
        }
    }
}
