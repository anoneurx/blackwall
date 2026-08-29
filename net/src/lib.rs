//! # blackwall-net
//!
//! A portable, `no_std` network protocol library for the Black Wall Core kernel.
//!
//! ## Layer Overview
//!
//! | Module      | RFC / Standard       | Description                          |
//! |-------------|----------------------|--------------------------------------|
//! | `ethernet`  | IEEE 802.3           | Layer 2 frame parsing & serialization |
//! | `arp`       | RFC 826              | Address resolution & cache table      |
//! | `ipv4`      | RFC 791              | Internet datagram & checksum          |
//! | `ipv6`      | RFC 8200             | 128-bit addressing & fixed header     |
//! | `icmp`      | RFC 792              | Echo request/reply, unreachable       |
//! | `udp`       | RFC 768              | Datagram sockets & socket pool        |
//! | `tcp`       | RFC 9293             | Stream sockets & state machine        |
//! | `dns`       | RFC 1035             | A-record query builder & resolver     |
//! | `firewall`  | —                   | Stateless packet filter / ACL         |
//! | `stack`     | —                   | Unified network stack orchestrator    |

#![no_std]

extern crate alloc;

pub mod arp;
pub mod dns;
pub mod ethernet;
pub mod firewall;
pub mod icmp;
pub mod ipv4;
pub mod ipv6;
pub mod stack;
pub mod tcp;
pub mod udp;
