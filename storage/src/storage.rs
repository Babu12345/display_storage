//! Peristent storage API
//! Wraps around the FlashStorgage API but will
//! allow for more safety and ease of use when storing a lot
//! of data that should be organized well.
use crate::error::{self, Result};
use core::usize;
use embedded_storage::Storage;

use storage_macros::ValidateAddrSpace;
use strum::{EnumIter, IntoEnumIterator};

/// Allows access to the peristent storage of the device
pub struct PersistentStorage<'a, STORAGE>
where
    STORAGE: Storage,
{
    flash_storage: STORAGE,
    internal_buffer: &'a mut [u8],
}

/// Reserved and used content sections for storage
#[derive(Clone, Copy, PartialEq, Debug, EnumIter, ValidateAddrSpace)]
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
    /// Storage wifi credential information
    #[addr_space(0x32c59a, 0x32c699)]
    WifiCredentials,
    /// Storage for textual display information
    #[addr_space(0x32c69a, 0x32c799)]
    DisplayText,
    /// Storage for URL information
    #[addr_space(0x32c79a, 0x32c899)]
    DisplayURL,
    /// Storage for saved MQTT topics (up to 24 topics)
    #[addr_space(0x32c89a, 0x32d499)]
    MqttTopics,
    /// Max cycles before display full refresh
    #[addr_space(0x32d49a, 0x32d49b)]
    MaxCyclesBeforeFullRefresh,
    /// Minimum update interval between display refreshes
    #[addr_space(0x32d49c, 0x32d49f)]
    MinUpdateInterval,
    /// Last successful update timestamp (seconds since boot, persists across reconnects)
    #[addr_space(0x32d4a0, 0x32d4a7)]
    LastUpdateTimestamp,
    /// WiFi error flag - set when WiFi fails, cleared after displaying error on next boot
    #[addr_space(0x32d4a8, 0x32d4a8)]
    WifiErrorFlag,
    /// Reserved Phy init (4KB)
    #[addr_space(0x32d4a9, 0x32e4a8, reserved)]
    ReservedPhyInit,
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
