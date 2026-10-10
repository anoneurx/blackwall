//! # blackwall-fs
//!
//! A portable, `no_std` filesystem library for the Black Wall Core kernel.
//!
//! ## Module Overview
//!
//! | Module  | Description                                                      |
//! |---------|------------------------------------------------------------------|
//! | `vfs`   | Virtual Filesystem layer — traits, VfsManager, mount table       |
//! | `block` | Block device trait + in-memory device for tests / RAM disks      |
//! | `ramfs` | In-memory RAM filesystem (root `/` mount, init ELF hosting)      |
//! | `ext2`  | Read/write Ext2 driver (direct + indirect blocks, create/mkdir)  |
//! | `ext4`  | Ext4 stub — extents + journal awareness (write path planned)     |
//! | `bwfs`  | Black Wall native filesystem — flat key-value package store      |

#![no_std]

extern crate alloc;

pub mod block;
pub mod bwfs;
pub mod ext2;
pub mod ext4;
pub mod ramfs;
pub mod vfs;
