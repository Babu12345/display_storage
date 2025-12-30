//! Peristent storage API
//! Wraps around the FlashStorgage API but will
//! allow for more safety and ease of use when storing a lot
//! of data that should be organized well.
use crate::error::{self, Result};
use core::{fmt::Debug, usize};
use embedded_storage::Storage;

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
#[derive(Clone, Copy, PartialEq, Debug, EnumIter)]
pub enum StorageContents {
    /// First 0x9000 addresses are reserved for safety
    /// See https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-guides/partition-tables.html#partition-tables for more info
    ReservedStart,
    /// Stores whether this was the first frame. Increase size to make
    /// it more likely that there is no collisons and proper detection occurs
    FirstFrameMetadata,
    /// Stores how many cycles have happened since the last full Normal refresh
    DisplayCycleCountMetadata,
    /// Stores the last frame of the eink display
    Frame,
    /// Storage wifi credential information
    WifiCredentials,
    /// Storage for textual display information
    DisplayText,
    /// Storage for URL information
    DisplayURL,
    /// Storage for saved MQTT topics (up to 24 topics)
    MqttTopics,
    /// Max cycles before display full refresh
    MaxCyclesBeforeFullRefresh,
    /// Minimum update interval between display refreshes
    MinUpdateInterval,
    /// Reserved Phy init
    /// See https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-guides/partition-tables.html#partition-tables for more info
    ReservedPhyInit,
    /// Reserved Factory
    /// See https://docs.espressif.com/projects/esp-idf/en/stable/esp32c3/api-guides/partition-tables.html#partition-tables for more info
    ReservedFactory,
    /// Last few addresses are reserved for safety
    ReservedEnd,
}

trait AddrSpace: IntoEnumIterator + Debug + PartialEq + Copy {
    fn get_address(self) -> (u32, u32);
    fn is_address_reserved(self) -> bool;
    #[allow(dead_code)]
    fn validate() {
        for item_i in Self::iter() {
            assert!(
                item_i.get_address().0 <= item_i.get_address().1,
                "Low memory address of {:?} must be less than or equal to higher address",
                item_i
            );
            for item_j in Self::iter() {
                if item_i == item_j {
                    continue;
                }
                assert!(
                    !(item_i.get_address().0 >= item_j.get_address().0
                        && item_i.get_address().0 <= item_j.get_address().1)
                        && !(item_i.get_address().1 >= item_j.get_address().0
                            && item_i.get_address().1 <= item_j.get_address().1),
                    "Memory addresses cannot cross. Problematic addresses: {:?} and {:?}",
                    item_i,
                    item_j
                );
            }
        }
    }
}

impl AddrSpace for StorageContents {
    /// Maps the content type to the starting address (inclusive) and end address (inclusive)
    /// DO NOT INTERSECT ANY OF THESE ADDRESSES OR SUBSEQUENT VALIDATIONS WILL PANIC
    ///
    /// For every bit of address space 1 Byte is stored
    /// Assuming a total size of ~4MBs we'll say that the last address is 0x3fffff
    fn get_address(self) -> (u32, u32) {
        match self {
            Self::ReservedStart => (0x0000, 0x8fff),
            Self::FirstFrameMetadata => (0x9000, 0x9000),
            Self::DisplayCycleCountMetadata => (0x9001, 0x9001),
            Self::Frame => (0x9002, 0xca99), // Only allocated enough space to store 1 400x300 frames
            Self::WifiCredentials => (0xca9a, 0xcb99), // Allocated enough for 256 bytes
            Self::DisplayText => (0xcb9a, 0xcc99), // Allocated enough for 256 bytes
            Self::DisplayURL => (0xcc9a, 0xcd99), // Allocated enough for 256 bytes
            Self::MqttTopics => (0xcd9a, 0xd999), // Allocated 3072 bytes for ~24 topics (128 bytes each)
            Self::MaxCyclesBeforeFullRefresh => (0xd99a, 0xd99b), // 2 bytes for u16
            Self::MinUpdateInterval => (0xd99c, 0xd99f), // 4 bytes for u32
            Self::ReservedPhyInit => (0xf000, 0xffff),
            Self::ReservedFactory => (0x10000, 0x110000),
            Self::ReservedEnd => (0x3ffffe, 0x3fffff),
        }
    }

    fn is_address_reserved(self) -> bool {
        match self {
            Self::ReservedStart => true,
            Self::ReservedPhyInit => true,
            Self::ReservedFactory => true,
            Self::ReservedEnd => true,
            Self::FirstFrameMetadata => false,
            Self::DisplayCycleCountMetadata => false,
            Self::Frame => false,
            Self::WifiCredentials => false,
            Self::DisplayText => false,
            Self::DisplayURL => false,
            Self::MqttTopics => false,
            Self::MaxCyclesBeforeFullRefresh => false,
            Self::MinUpdateInterval => false,
        }
    }
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
mod storage_tests {
    use super::*;

    #[test]
    fn verify_valid_storage() {
        StorageContents::validate();
    }
}
