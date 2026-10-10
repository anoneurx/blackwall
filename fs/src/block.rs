//! Block Device Abstraction
//!
//! Filesystem drivers talk to storage through the [`BlockDevice`] trait.  The
//! kernel supplies an implementation backed by a real controller (AHCI,
//! virtio-blk, NVMe); the portable crate ships [`MemBlockDevice`] for tests and
//! for RAM disks.
//!
//! All addressing is in 512-byte logical sectors, matching the ATA/virtio model.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::vfs::VfsError;

/// Canonical logical sector size used for all block addressing.
pub const SECTOR_SIZE: u64 = 512;

/// A random-access block storage device.
///
/// Implementors must be safe to share across CPUs (`Send + Sync`); the
/// filesystem wraps them in an [`Arc`] and issues reads/writes concurrently
/// from different tasks.
pub trait BlockDevice: Send + Sync {
    /// Size of one logical sector in bytes.  Defaults to 512.
    fn sector_size(&self) -> u64 {
        SECTOR_SIZE
    }

    /// Total number of logical sectors on the device.
    fn num_sectors(&self) -> u64;

    /// Read `buf.len()` bytes starting at logical block `lba`.
    ///
    /// `buf.len()` must be an exact multiple of [`Self::sector_size`].
    fn read_sectors(&self, lba: u64, buf: &mut [u8]) -> Result<(), VfsError>;

    /// Write `buf.len()` bytes starting at logical block `lba`.
    ///
    /// `buf.len()` must be an exact multiple of [`Self::sector_size`].
    fn write_sectors(&self, lba: u64, buf: &[u8]) -> Result<(), VfsError>;

    /// Flush any volatile write-back cache to stable storage.
    fn flush(&self) -> Result<(), VfsError> {
        Ok(())
    }

    /// Total size of the device in bytes.
    fn num_bytes(&self) -> u64 {
        self.num_sectors() * self.sector_size()
    }

    /// Read an arbitrary byte range, crossing sector boundaries as needed.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), VfsError> {
        let ss = self.sector_size();
        let mut done = 0usize;
        while done < buf.len() {
            let cur = offset + done as u64;
            let lba = cur / ss;
            let in_sector = (cur % ss) as usize;
            let chunk = (buf.len() - done).min(ss as usize - in_sector);
            let mut sector = vec![0u8; ss as usize];
            self.read_sectors(lba, &mut sector)?;
            buf[done..done + chunk].copy_from_slice(&sector[in_sector..in_sector + chunk]);
            done += chunk;
        }
        Ok(())
    }

    /// Write an arbitrary byte range, crossing sector boundaries as needed.
    fn write_at(&self, offset: u64, buf: &[u8]) -> Result<(), VfsError> {
        let ss = self.sector_size();
        let mut done = 0usize;
        while done < buf.len() {
            let cur = offset + done as u64;
            let lba = cur / ss;
            let in_sector = (cur % ss) as usize;
            let chunk = (buf.len() - done).min(ss as usize - in_sector);
            let mut sector = vec![0u8; ss as usize];
            // Read-modify-write unless the write covers the whole sector.
            if in_sector != 0 || chunk != ss as usize {
                self.read_sectors(lba, &mut sector)?;
            }
            sector[in_sector..in_sector + chunk].copy_from_slice(&buf[done..done + chunk]);
            self.write_sectors(lba, &sector)?;
            done += chunk;
        }
        Ok(())
    }

    /// Read a whole block of `len` bytes as an owned `Vec` (convenience for
    /// filesystem metadata such as superblocks and inode tables).
    fn read_exact(&self, offset: u64, len: usize) -> Result<Vec<u8>, VfsError> {
        let mut buf = vec![0u8; len];
        self.read_at(offset, &mut buf)?;
        Ok(buf)
    }
}

/// A minimal spin lock so the in-memory device can present `&self` mutation
/// without pulling in an external crate.
struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// SAFETY: access to the inner value is serialised by the `locked` flag.
unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    const fn new(data: T) -> Self {
        Self { locked: AtomicBool::new(false), data: UnsafeCell::new(data) }
    }

    fn lock(&self) -> SpinGuard<'_, T> {
        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        SpinGuard { lock: self }
    }
}

struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> core::ops::Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the lock is held for the guard's lifetime, giving exclusive
        // access; no other guard can exist concurrently.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> core::ops::DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: exclusive access guaranteed by the held spin lock.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// An in-memory block device backed by a growable byte buffer.
///
/// Used for RAM disks and for host-side tests of the filesystem drivers.
pub struct MemBlockDevice {
    data: SpinLock<Vec<u8>>,
    sector_size: u64,
}

impl MemBlockDevice {
    /// Create a zero-filled device of `num_sectors` × 512 bytes.
    pub fn new(num_sectors: u64) -> Self {
        Self {
            data: SpinLock::new(vec![0u8; (num_sectors * SECTOR_SIZE) as usize]),
            sector_size: SECTOR_SIZE,
        }
    }

    /// Wrap an existing byte image.  The length is rounded up to a whole
    /// number of sectors.
    pub fn from_image(image: Vec<u8>) -> Self {
        let mut data = image;
        let rem = data.len() % SECTOR_SIZE as usize;
        if rem != 0 {
            data.resize(data.len() + (SECTOR_SIZE as usize - rem), 0);
        }
        Self { data: SpinLock::new(data), sector_size: SECTOR_SIZE }
    }

    /// Copy the current contents out (primarily for tests / persistence).
    pub fn snapshot(&self) -> Vec<u8> {
        self.data.lock().clone()
    }
}

impl BlockDevice for MemBlockDevice {
    fn sector_size(&self) -> u64 {
        self.sector_size
    }

    fn num_sectors(&self) -> u64 {
        self.data.lock().len() as u64 / self.sector_size
    }

    fn read_sectors(&self, lba: u64, buf: &mut [u8]) -> Result<(), VfsError> {
        let ss = self.sector_size as usize;
        if buf.len() % ss != 0 {
            return Err(VfsError::IOError);
        }
        let start = (lba as usize) * ss;
        let end = start + buf.len();
        let data = self.data.lock();
        if end > data.len() {
            return Err(VfsError::IOError);
        }
        buf.copy_from_slice(&data[start..end]);
        Ok(())
    }

    fn write_sectors(&self, lba: u64, buf: &[u8]) -> Result<(), VfsError> {
        let ss = self.sector_size as usize;
        if buf.len() % ss != 0 {
            return Err(VfsError::IOError);
        }
        let start = (lba as usize) * ss;
        let end = start + buf.len();
        let mut data = self.data.lock();
        if end > data.len() {
            return Err(VfsError::IOError);
        }
        data[start..end].copy_from_slice(buf);
        Ok(())
    }
}

/// Convenience alias for a shareable block device.
pub type SharedBlockDevice = Arc<dyn BlockDevice>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_device_read_write() {
        let dev = MemBlockDevice::new(16);
        assert_eq!(dev.sector_size(), 512);
        assert_eq!(dev.num_sectors(), 16);

        let mut data = [0u8; 512];
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }
        dev.write_sectors(2, &data).unwrap();

        let mut out = [0u8; 512];
        dev.read_sectors(2, &mut out).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn read_write_across_sectors() {
        let dev = MemBlockDevice::new(16);
        let payload = [0xABu8; 700];
        dev.write_at(400, &payload).unwrap();
        let mut out = [0u8; 700];
        dev.read_at(400, &mut out).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn out_of_range_errors() {
        let dev = MemBlockDevice::new(4);
        let mut buf = [0u8; 512];
        assert!(dev.read_sectors(10, &mut buf).is_err());
        assert!(dev.write_sectors(10, &buf).is_err());
    }

    #[test]
    fn from_image_rounds_up() {
        let dev = MemBlockDevice::from_image(vec![1u8; 1000]);
        assert_eq!(dev.num_sectors(), 2);
    }
}
