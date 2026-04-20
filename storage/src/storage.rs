//! Peristent storage API
//! Wraps around the FlashStorgage API but will
//! allow for more safety and ease of use when storing a lot
//! of data that should be organized well.
use crate::error::{self, Result};
use core::usize;
use embedded_storage::Storage;

use storage_macros::AddrSpace;
use strum::{EnumIter, IntoEnumIterator};

/// Allows access to the peristent storage of the device
pub struct PersistentStorage<'a, STORAGE>
where
    STORAGE: Storage,
{
    flash_storage: STORAGE,
    internal_buffer: &'a mut [u8],
}

/// Reserved and used content sections for storage.
///
/// Flash layout supports both development and secure boot modes:
///
/// Development (espflash default bootloader):
/// - 0x00000 - 0x08fff: Bootloader (~32KB)
/// - 0x09000 - 0x0efff: NVS (24KB)
/// - 0x10000 - 0x30ffff: App (factory partition, 3MB)
///
/// Secure Boot V2 + Flash Encryption:
/// - 0x00000 - 0x0ffff: Bootloader (secure boot needs ~48KB, reserving 64KB)
/// - 0x10000 - 0x1ffff: Partition table + NVS/OTA/PHY data
/// - 0x20000 - 0x30ffff: App (factory partition, ~3MB)
///
/// User data starts at 0x310000 (works for both modes)
#[derive(Clone, Copy, PartialEq, Debug, EnumIter, AddrSpace)]
pub enum StorageContents {
    /// Reserved for bootloader, partition table, NVS (0x0000-0x0ffff)
    /// Covers both dev mode (smaller bootloader) and secure boot (larger bootloader)
    #[addr_space(0x0000, 0x0ffff, reserved)]
    ReservedStart,
    /// Reserved for factory app partition (0x10000-0x30ffff, ~3MB)
    /// Dev mode: app at 0x10000, Secure boot: app at 0x20000, but both end at 0x310000
    #[addr_space(0x10000, 0x30ffff, reserved)]
    ReservedFactory,
    /// Stores whether this was the first frame. Increase size to make
    /// it more likely that there is no collisons and proper detection occurs
    #[addr_space(0x310000, 0x310000)]
    FirstFrameMetadata,
    /// Stores how many cycles have happened since the last full Normal refresh
    #[addr_space(0x310001, 0x310001)]
    DisplayCycleCountMetadata,
    /// Stores the last frame of the eink display
    #[addr_space(0x310002, 0x32c599)]
    Frame,
    /// Storage wifi credential information (512 bytes for MAX_NFCDATA_SIZE)
    #[addr_space(0x32c59a, 0x32c799)]
    WifiCredentials,
    /// Storage for textual display information (512 bytes for MAX_NFCDATA_SIZE)
    #[addr_space(0x32c79a, 0x32c999)]
    DisplayText,
    /// Storage for URL information (512 bytes for MAX_NFCDATA_SIZE)
    #[addr_space(0x32c99a, 0x32cb99)]
    DisplayURL,
    /// Storage for saved MQTT topics (up to 24 topics)
    #[addr_space(0x32cb9a, 0x32d799)]
    MqttTopics,
    /// Max cycles before display full refresh
    #[addr_space(0x32d79a, 0x32d79b)]
    MaxCyclesBeforeFullRefresh,
    /// Minimum update interval between display refreshes
    #[addr_space(0x32d79c, 0x32d79f)]
    MinUpdateInterval,
    /// Last successful update timestamp (seconds since boot, persists across reconnects)
    #[addr_space(0x32d7a0, 0x32d7a7)]
    LastUpdateTimestamp,
    /// WiFi error flag - set when WiFi fails, cleared after displaying error on next boot
    #[addr_space(0x32d7a8, 0x32d7a8)]
    WifiErrorFlag,
    /// User registration flag - 0xFF = first time (unregistered), 0x01 = registered
    /// Used to detect first-time setup and trigger onboarding flow
    #[addr_space(0x32d7a9, 0x32d7a9)]
    UserRegistered,
    /// Display mode - 0x00 = LiveUpdates, 0x01 = CustomText, 0x02 = QRCode, 0xFF = unset (default to LiveUpdates)
    /// Persists the current display mode across reboots
    #[addr_space(0x32d7aa, 0x32d7aa)]
    DisplayMode,
    /// Last few addresses are reserved for safety
    #[addr_space(0x7ffffe, 0x7fffff, reserved)]
    ReservedEnd,
}

impl<'storage, STORAGE> PersistentStorage<'storage, STORAGE>
where
    STORAGE: Storage,
{
    /// Creates a new persistent storage object
    pub fn new(storage: STORAGE, buffer: &'storage mut [u8]) -> Self {
        Self {
            flash_storage: storage,
            internal_buffer: buffer,
        }
    }

    /// Resets all storage content types. The size of the clearing is SIZE_IN_BYTES for each content
    /// type
    pub fn clear_all_storage<const SIZE_IN_BYTES: usize>(&mut self) -> Result<()> {
        for content in StorageContents::iter() {
            self.clear_storage::<SIZE_IN_BYTES>(content)?
        }
        Ok(())
    }

    /// Resets storage to default value of all 1s
    pub fn clear_storage<const SIZE_IN_BYTES: usize>(
        &mut self,
        content: StorageContents,
    ) -> Result<()> {
        self.write_bytes(content, 0, &[0xFF; SIZE_IN_BYTES])?;
        Ok(())
    }

    /// Writes the byte into storage
    pub fn write_byte(&mut self, content: StorageContents, offset: u32, data: u8) -> Result<()> {
        let data = data.to_be_bytes();
        self.write_bytes(content, offset, &data)?;
        Ok(())
    }

    /// Writes the bytes into storage
    ///
    /// Note that flash memory has an estimated ~100k writes cycles
    /// So try not to use too much unnecessarily
    pub fn write_bytes(
        &mut self,
        content: StorageContents,
        offset: u32,
        data: &[u8],
    ) -> Result<()> {
        assert!(
            !content.is_address_reserved(),
            "Cannot write to a reserved address allocated for {:?}",
            content
        );

        let absolute_offset = content.get_address().0 + offset;
        assert!(
            absolute_offset <= content.get_address().1,
            "Cannot write to address outside of the space allocated to {:?}",
            content
        );

        assert!(
            content.get_address().1 - absolute_offset + 1 >= data.len() as u32,
            "There is not enough space allocated to write this data"
        );
        self.flash_storage
            .write(absolute_offset, data)
            .map_err(|_| error::Error::FlashWriteError)?;
        Ok(())
    }

    /// Reads the byte array from storage.
    /// Be careful when reading to ensure that the
    /// data from the offset to the offset+internal buffer size is valid.
    pub fn read(&mut self, content: StorageContents) -> Result<&[u8]> {
        self.flash_storage
            .read(content.get_address().0, &mut self.internal_buffer)
            .map_err(|_| error::Error::FlashReadError)?;
        Ok(self.internal_buffer)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::convert::Infallible;
    use embedded_storage::ReadStorage;
    use std::collections::BTreeMap;

    /// Byte-addressed mock backed by a `BTreeMap`. Uninitialized reads return
    /// 0xFF to mirror erased-flash semantics.
    #[derive(Default)]
    struct MockStorage {
        map: BTreeMap<u32, u8>,
    }

    impl ReadStorage for MockStorage {
        type Error = Infallible;
        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> core::result::Result<(), Infallible> {
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = self.map.get(&(offset + i as u32)).copied().unwrap_or(0xFF);
            }
            Ok(())
        }
        fn capacity(&self) -> usize {
            0x800000
        }
    }

    impl Storage for MockStorage {
        fn write(&mut self, offset: u32, bytes: &[u8]) -> core::result::Result<(), Infallible> {
            for (i, b) in bytes.iter().enumerate() {
                self.map.insert(offset + i as u32, *b);
            }
            Ok(())
        }
    }

    /// Always-failing storage used to verify error propagation.
    struct FailingStorage;

    #[derive(Debug)]
    struct DummyErr;

    impl ReadStorage for FailingStorage {
        type Error = DummyErr;
        fn read(&mut self, _offset: u32, _bytes: &mut [u8]) -> core::result::Result<(), DummyErr> {
            Err(DummyErr)
        }
        fn capacity(&self) -> usize {
            0x800000
        }
    }

    impl Storage for FailingStorage {
        fn write(&mut self, _offset: u32, _bytes: &[u8]) -> core::result::Result<(), DummyErr> {
            Err(DummyErr)
        }
    }

    #[test]
    fn write_byte_is_read_back() {
        let mut buf = [0u8; 1];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);

        ps.write_byte(StorageContents::UserRegistered, 0, 0x01).unwrap();

        let got = ps.read(StorageContents::UserRegistered).unwrap();
        assert_eq!(got, &[0x01]);
    }

    #[test]
    fn write_bytes_round_trip() {
        let mut buf = [0u8; 4];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];

        ps.write_bytes(StorageContents::MinUpdateInterval, 0, &payload).unwrap();

        let got = ps.read(StorageContents::MinUpdateInterval).unwrap();
        assert_eq!(got, &payload);
    }

    #[test]
    fn write_at_nonzero_offset_is_read_back() {
        // MinUpdateInterval is 4 bytes (0x32d79c..=0x32d79f). Write at offset 2.
        let mut buf = [0u8; 4];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);

        ps.write_bytes(StorageContents::MinUpdateInterval, 2, &[0xAA, 0xBB]).unwrap();

        let got = ps.read(StorageContents::MinUpdateInterval).unwrap();
        // Bytes before the offset stay at erased-flash default (0xFF).
        assert_eq!(got, &[0xFF, 0xFF, 0xAA, 0xBB]);
    }

    #[test]
    fn writes_to_separate_contents_do_not_collide() {
        let mut buf = [0u8; 1];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);

        ps.write_byte(StorageContents::UserRegistered, 0, 0x42).unwrap();
        ps.write_byte(StorageContents::DisplayMode, 0, 0x02).unwrap();

        assert_eq!(ps.read(StorageContents::UserRegistered).unwrap(), &[0x42]);
        assert_eq!(ps.read(StorageContents::DisplayMode).unwrap(), &[0x02]);
    }

    #[test]
    fn read_before_write_returns_erased_flash_default() {
        let mut buf = [0u8; 4];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);

        let got = ps.read(StorageContents::MinUpdateInterval).unwrap();
        assert_eq!(got, &[0xFF; 4]);
    }

    #[test]
    fn clear_storage_restores_erased_default() {
        let mut buf = [0u8; 4];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);

        ps.write_bytes(StorageContents::MinUpdateInterval, 0, &[1, 2, 3, 4]).unwrap();
        ps.clear_storage::<4>(StorageContents::MinUpdateInterval).unwrap();

        assert_eq!(ps.read(StorageContents::MinUpdateInterval).unwrap(), &[0xFF; 4]);
    }

    #[test]
    fn read_uses_content_start_address() {
        // Prove `read` targets the content's start address and fills the full
        // internal buffer, by writing a distinct byte at the exact start.
        let mut buf = [0u8; 2];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);

        ps.write_byte(StorageContents::MinUpdateInterval, 0, 0x7E).unwrap();

        let got = ps.read(StorageContents::MinUpdateInterval).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], 0x7E);
        assert_eq!(got[1], 0xFF);
    }

    #[test]
    #[should_panic(expected = "Cannot write to a reserved address")]
    fn writing_to_reserved_address_panics() {
        let mut buf = [0u8; 1];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);
        ps.write_byte(StorageContents::ReservedStart, 0, 0x00).unwrap();
    }

    #[test]
    #[should_panic(expected = "Cannot write to address outside of the space allocated")]
    fn writing_past_end_of_slot_panics() {
        // UserRegistered is a single byte — offset 1 is past the end.
        let mut buf = [0u8; 1];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);
        ps.write_byte(StorageContents::UserRegistered, 1, 0x00).unwrap();
    }

    #[test]
    #[should_panic(expected = "not enough space allocated")]
    fn writing_more_bytes_than_slot_allows_panics() {
        let mut buf = [0u8; 4];
        let mut ps = PersistentStorage::new(MockStorage::default(), &mut buf);
        // MinUpdateInterval holds 4 bytes; 5 bytes overflows.
        ps.write_bytes(StorageContents::MinUpdateInterval, 0, &[0; 5]).unwrap();
    }

    #[test]
    fn write_error_is_mapped_to_flash_write_error() {
        let mut buf = [0u8; 1];
        let mut ps = PersistentStorage::new(FailingStorage, &mut buf);

        let err = ps
            .write_byte(StorageContents::UserRegistered, 0, 0x01)
            .unwrap_err();
        assert!(matches!(err, error::Error::FlashWriteError));
    }

    #[test]
    fn read_error_is_mapped_to_flash_read_error() {
        let mut buf = [0u8; 1];
        let mut ps = PersistentStorage::new(FailingStorage, &mut buf);

        let err = ps.read(StorageContents::UserRegistered).unwrap_err();
        assert!(matches!(err, error::Error::FlashReadError));
    }
}
