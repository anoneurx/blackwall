use crate::ipv4::IpAddress;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallAction {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpCidr {
    pub ip: IpAddress,
    pub prefix: u8,
}

impl IpCidr {
    pub fn new(ip: IpAddress, prefix: u8) -> Self {
        Self { ip, prefix: core::cmp::min(prefix, 32) }
    }

    pub fn matches(&self, test_ip: IpAddress) -> bool {
        if self.prefix == 0 {
            return true;
        }
        let mask = !0u32 << (32 - self.prefix);
        let ip_u32 = u32::from_be_bytes(test_ip.0);
        let cidr_u32 = u32::from_be_bytes(self.ip.0);
        (ip_u32 & mask) == (cidr_u32 & mask)
    }
}

pub struct FirewallRule {
    pub action: FirewallAction,
    pub protocol: Option<u8>, // Some(1) = ICMP, Some(6) = TCP, Some(17) = UDP, None = Any
    pub src_cidr: Option<IpCidr>,
    pub dest_cidr: Option<IpCidr>,
    pub src_port: Option<u16>,
    pub dest_port: Option<u16>,
}

impl FirewallRule {
    pub fn matches(
        &self,
        protocol: u8,
        src_ip: IpAddress,
        dest_ip: IpAddress,
        src_port: u16,
        dest_port: u16,
    ) -> bool {
        // Match Protocol
        if let Some(rule_proto) = self.protocol {
            if rule_proto != protocol {
                return false;
            }
        }

        // Match Src IP CIDR
        if let Some(ref cidr) = self.src_cidr {
            if !cidr.matches(src_ip) {
                return false;
            }
        }

        // Match Dest IP CIDR
        if let Some(ref cidr) = self.dest_cidr {
            if !cidr.matches(dest_ip) {
                return false;
            }
        }

        // Match Src Port
        if let Some(rule_port) = self.src_port {
            if rule_port != src_port {
                return false;
            }
        }

        // Match Dest Port
        if let Some(rule_port) = self.dest_port {
            if rule_port != dest_port {
                return false;
            }
        }

        true
    }
}

pub struct Firewall {
    pub rules: Vec<FirewallRule>,
    pub default_policy: FirewallAction,
}

impl Firewall {
    pub fn new(default_policy: FirewallAction) -> Self {
        Self { rules: Vec::new(), default_policy }
    }

    pub fn add_rule(&mut self, rule: FirewallRule) {
        self.rules.push(rule);
    }

    pub fn check_packet(
        &self,
        protocol: u8,
        src_ip: IpAddress,
        dest_ip: IpAddress,
        src_port: u16,
        dest_port: u16,
    ) -> FirewallAction {
        for rule in &self.rules {
            if rule.matches(protocol, src_ip, dest_ip, src_port, dest_port) {
                return rule.action;
            }
        }
        self.default_policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddress {
        IpAddress([a, b, c, d])
    }

    #[test]
    fn cidr_exact_match() {
        let cidr = IpCidr::new(ip(192, 168, 1, 0), 24);
        assert!(cidr.matches(ip(192, 168, 1, 100)));
        assert!(cidr.matches(ip(192, 168, 1, 255)));
        assert!(!cidr.matches(ip(192, 168, 2, 1)));
    }

    #[test]
    fn cidr_prefix_0_matches_all() {
        let cidr = IpCidr::new(ip(0, 0, 0, 0), 0);
        assert!(cidr.matches(ip(10, 0, 0, 1)));
        assert!(cidr.matches(ip(255, 255, 255, 255)));
    }

    #[test]
    fn cidr_prefix_32_exact() {
        let cidr = IpCidr::new(ip(10, 0, 0, 1), 32);
        assert!(cidr.matches(ip(10, 0, 0, 1)));
        assert!(!cidr.matches(ip(10, 0, 0, 2)));
    }

    #[test]
    fn cidr_slash_8() {
        let cidr = IpCidr::new(ip(10, 0, 0, 0), 8);
        assert!(cidr.matches(ip(10, 255, 255, 255)));
        assert!(!cidr.matches(ip(11, 0, 0, 0)));
    }

    #[test]
    fn firewall_rule_match_protocol() {
        let rule = FirewallRule {
            action: FirewallAction::Deny,
            protocol: Some(6), // TCP
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: None,
        };
        assert!(rule.matches(6, ip(1, 2, 3, 4), ip(5, 6, 7, 8), 0, 80));
        assert!(!rule.matches(17, ip(1, 2, 3, 4), ip(5, 6, 7, 8), 0, 80)); // UDP
    }

    #[test]
    fn firewall_rule_match_dst_port() {
        let rule = FirewallRule {
            action: FirewallAction::Allow,
            protocol: None,
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: Some(22),
        };
        assert!(rule.matches(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 22));
        assert!(!rule.matches(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 80));
    }

    #[test]
    fn firewall_rule_match_src_cidr() {
        let rule = FirewallRule {
            action: FirewallAction::Deny,
            protocol: None,
            src_cidr: Some(IpCidr::new(ip(192, 168, 1, 0), 24)),
            dest_cidr: None,
            src_port: None,
            dest_port: None,
        };
        assert!(rule.matches(1, ip(192, 168, 1, 50), ip(10, 0, 0, 1), 0, 0));
        assert!(!rule.matches(1, ip(10, 0, 0, 1), ip(10, 0, 0, 1), 0, 0));
    }

    #[test]
    fn firewall_check_packet_default_allow() {
        let fw = Firewall::new(FirewallAction::Allow);
        assert_eq!(
            fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 80),
            FirewallAction::Allow
        );
    }

    #[test]
    fn firewall_check_packet_default_deny() {
        let fw = Firewall::new(FirewallAction::Deny);
        assert_eq!(fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 80), FirewallAction::Deny);
    }

    #[test]
    fn firewall_rule_overrides_default() {
        let mut fw = Firewall::new(FirewallAction::Deny);
        fw.add_rule(FirewallRule {
            action: FirewallAction::Allow,
            protocol: Some(17), // UDP
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: Some(53),
        });
        // UDP/53 → Allow (rule matches)
        assert_eq!(
            fw.check_packet(17, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 53),
            FirewallAction::Allow
        );
        // TCP/80 → Deny (no rule, default)
        assert_eq!(fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 80), FirewallAction::Deny);
    }

    #[test]
    fn firewall_first_matching_rule_wins() {
        let mut fw = Firewall::new(FirewallAction::Allow);
        fw.add_rule(FirewallRule {
            action: FirewallAction::Deny,
            protocol: None,
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: None,
        });
        fw.add_rule(FirewallRule {
            action: FirewallAction::Allow,
            protocol: None,
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: None,
        });
        // First rule (Deny) wins
        assert_eq!(fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 0, 80), FirewallAction::Deny);
    }

    #[test]
    fn cidr_slash_16() {
        let cidr = IpCidr::new(ip(172, 16, 0, 0), 16);
        assert!(cidr.matches(ip(172, 16, 255, 1)));
        assert!(!cidr.matches(ip(172, 17, 0, 1)));
    }

    #[test]
    fn cidr_slash_32_exact_ip() {
        let cidr = IpCidr::new(ip(192, 168, 7, 42), 32);
        assert!(cidr.matches(ip(192, 168, 7, 42)));
        assert!(!cidr.matches(ip(192, 168, 7, 43)));
        assert!(!cidr.matches(ip(192, 168, 8, 42)));
        assert!(!cidr.matches(ip(193, 168, 7, 42)));
    }

    #[test]
    fn cidr_boundary_crossing() {
        let cidr = IpCidr::new(ip(192, 168, 1, 0), 24);
        // Last host in subnet still matches
        assert!(cidr.matches(ip(192, 168, 1, 255)));
        // First address of next subnet does not match
        assert!(!cidr.matches(ip(192, 168, 2, 0)));
    }

    #[test]
    fn firewall_icmp_rule() {
        let mut fw = Firewall::new(FirewallAction::Deny);
        fw.add_rule(FirewallRule {
            action: FirewallAction::Allow,
            protocol: Some(1), // ICMP
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: None,
        });
        // ICMP → Allow
        assert_eq!(
            fw.check_packet(1, ip(8, 8, 8, 8), ip(10, 0, 0, 1), 0, 0),
            FirewallAction::Allow
        );
        // TCP → Deny (protocol mismatch, default)
        assert_eq!(
            fw.check_packet(6, ip(8, 8, 8, 8), ip(10, 0, 0, 1), 0, 80),
            FirewallAction::Deny
        );
    }

    #[test]
    fn firewall_tcp_rule_dst_port_range() {
        let mut fw = Firewall::new(FirewallAction::Deny);
        fw.add_rule(FirewallRule {
            action: FirewallAction::Allow,
            protocol: Some(6), // TCP
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: Some(443),
        });
        // TCP/443 → Allow
        assert_eq!(
            fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 1024, 443),
            FirewallAction::Allow
        );
        // TCP/80 → Deny
        assert_eq!(
            fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 1024, 80),
            FirewallAction::Deny
        );
        // TCP/8080 → Deny
        assert_eq!(
            fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 1024, 8080),
            FirewallAction::Deny
        );
    }

    #[test]
    fn firewall_udp_source_port() {
        let rule = FirewallRule {
            action: FirewallAction::Deny,
            protocol: Some(17), // UDP
            src_cidr: None,
            dest_cidr: None,
            src_port: Some(53),
            dest_port: None,
        };
        // Source port 53 matches regardless of destination port
        assert!(rule.matches(17, ip(1, 1, 1, 1), ip(2, 2, 2, 2), 53, 40000));
        assert!(rule.matches(17, ip(1, 1, 1, 1), ip(2, 2, 2, 2), 53, 0));
        // Other source ports do not match
        assert!(!rule.matches(17, ip(1, 1, 1, 1), ip(2, 2, 2, 2), 12345, 53));
        // Wrong protocol does not match even with same ports
        assert!(!rule.matches(6, ip(1, 1, 1, 1), ip(2, 2, 2, 2), 53, 40000));
    }

    #[test]
    fn firewall_multiple_rules_ordered() {
        let mut fw = Firewall::new(FirewallAction::Deny);
        fw.add_rule(FirewallRule {
            action: FirewallAction::Allow,
            protocol: Some(6),
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: Some(22),
        });
        fw.add_rule(FirewallRule {
            action: FirewallAction::Deny,
            protocol: None,
            src_cidr: None,
            dest_cidr: None,
            src_port: None,
            dest_port: None,
        });
        // SSH → Allow via first rule
        assert_eq!(
            fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 50000, 22),
            FirewallAction::Allow
        );
        // Port 80 → caught by deny-all rule
        assert_eq!(
            fw.check_packet(6, ip(0, 0, 0, 0), ip(0, 0, 0, 0), 50000, 80),
            FirewallAction::Deny
        );
    }

    #[test]
    fn firewall_complex_rule() {
        let mut fw = Firewall::new(FirewallAction::Deny);
        fw.add_rule(FirewallRule {
            action: FirewallAction::Allow,
            protocol: Some(6), // TCP
            src_cidr: Some(IpCidr::new(ip(10, 0, 0, 0), 8)),
            dest_cidr: None,
            src_port: None,
            dest_port: Some(22),
        });
        // All fields match → Allow
        assert_eq!(
            fw.check_packet(6, ip(10, 1, 2, 3), ip(192, 168, 1, 1), 50000, 22),
            FirewallAction::Allow
        );
        // Wrong source IP → default
        assert_eq!(
            fw.check_packet(6, ip(11, 0, 0, 1), ip(192, 168, 1, 1), 50000, 22),
            FirewallAction::Deny
        );
        // Wrong destination port → default
        assert_eq!(
            fw.check_packet(6, ip(10, 1, 2, 3), ip(192, 168, 1, 1), 50000, 80),
            FirewallAction::Deny
        );
        // Wrong protocol → default
        assert_eq!(
            fw.check_packet(17, ip(10, 1, 2, 3), ip(192, 168, 1, 1), 50000, 22),
            FirewallAction::Deny
        );
    }

    #[test]
    fn firewall_no_rules_default_allow() {
        let fw = Firewall::new(FirewallAction::Allow);
        assert_eq!(fw.rules.len(), 0);
        assert_eq!(
            fw.check_packet(6, ip(203, 0, 113, 7), ip(198, 51, 100, 9), 1234, 9999),
            FirewallAction::Allow
        );
        assert_eq!(
            fw.check_packet(1, ip(0, 0, 0, 0), ip(255, 255, 255, 255), 0, 0),
            FirewallAction::Allow
        );
    }

    #[test]
    fn firewall_no_rules_default_deny() {
        let fw = Firewall::new(FirewallAction::Deny);
        assert_eq!(fw.rules.len(), 0);
        assert_eq!(
            fw.check_packet(6, ip(203, 0, 113, 7), ip(198, 51, 100, 9), 1234, 9999),
            FirewallAction::Deny
        );
        assert_eq!(
            fw.check_packet(1, ip(0, 0, 0, 0), ip(255, 255, 255, 255), 0, 0),
            FirewallAction::Deny
        );
    }

    #[test]
    fn cidr_private_ranges() {
        // 10.0.0.0/8
        let ten = IpCidr::new(ip(10, 0, 0, 0), 8);
        assert!(ten.matches(ip(10, 0, 0, 1)));
        assert!(ten.matches(ip(10, 200, 30, 40)));
        assert!(!ten.matches(ip(11, 0, 0, 1)));

        // 172.16.0.0/12 spans 172.16.x.x – 172.31.x.x
        let one_seventy_two = IpCidr::new(ip(172, 16, 0, 0), 12);
        assert!(one_seventy_two.matches(ip(172, 16, 0, 1)));
        assert!(one_seventy_two.matches(ip(172, 31, 255, 254)));
        assert!(!one_seventy_two.matches(ip(172, 32, 0, 1)));
        assert!(!one_seventy_two.matches(ip(172, 15, 255, 255)));

        // 192.168.0.0/16
        let one_nine_two = IpCidr::new(ip(192, 168, 0, 0), 16);
        assert!(one_nine_two.matches(ip(192, 168, 0, 1)));
        assert!(one_nine_two.matches(ip(192, 168, 99, 199)));
        assert!(!one_nine_two.matches(ip(192, 169, 0, 1)));
    }
}
