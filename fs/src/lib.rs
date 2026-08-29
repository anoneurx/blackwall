//! # blackwall-fs
//!
//! A portable, `no_std` filesystem library for the Black Wall Core kernel.
//!
//! ## Module Overview
//!
//! | Module  | Description                                                      |
//! |---------|------------------------------------------------------------------|
//! | `vfs`   | Virtual Filesystem layer — traits, VfsManager, mount table       |
//! | `ramfs` | In-memory RAM filesystem (root `/` mount, init ELF hosting)      |
//! | `ext2`  | Read-only Ext2 partition driver (superblock, inodes, dir entries)|
//! | `ext4`  | Ext4 stub — extents + journal awareness (write path planned)     |
//! | `bwfs`  | Black Wall native filesystem — flat key-value package store      |

#![no_std]

extern crate alloc;

pub mod bwfs;
pub mod ext2;
pub mod ext4;
pub mod ramfs;
pub mod vfs;
