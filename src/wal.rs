//! Write-Ahead Logging (WAL) module for crash recovery and durability.
//!
//! The WAL ensures that all page modifications are logged before being applied
//! to the main database file, enabling recovery after crashes.

use crate::pager::PAGE_SIZE;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Magic bytes for WAL file identification.
const WAL_MAGIC: &[u8] = b"BTREEWAL";
const WAL_MAGIC_LEN: usize = 8;

/// WAL file header size.
const WAL_HEADER_SIZE: usize = 32;

/// WAL record header size: record_len (4) + page_id (4) + checksum (4) = 12 bytes
const WAL_RECORD_HEADER_SIZE: usize = 12;

/// A single WAL record representing a page modification.
#[derive(Debug, Clone)]
pub struct WalRecord {
    /// Page ID that was modified
    pub page_id: u32,
    /// Checksum of the page data
    pub checksum: u32,
    /// The page data (4096 bytes)
    pub data: [u8; PAGE_SIZE],
}

impl WalRecord {
    /// Creates a new WAL record.
    pub fn new(page_id: u32, data: [u8; PAGE_SIZE]) -> Self {
        let checksum = Self::compute_checksum(&data);
        WalRecord {
            page_id,
            checksum,
            data,
        }
    }

    /// Computes a simple checksum of the data.
    fn compute_checksum(data: &[u8]) -> u32 {
        let mut sum: u32 = 0;
        for chunk in data.chunks(4) {
            let mut bytes = [0u8; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            sum = sum.wrapping_add(u32::from_le_bytes(bytes));
        }
        sum
    }

    /// Verifies the checksum of the record.
    pub fn verify_checksum(&self) -> bool {
        self.checksum == Self::compute_checksum(&self.data)
    }

    /// Serializes the record to a writer.
    pub fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Record length (excluding the length field itself)
        let record_len = WAL_RECORD_HEADER_SIZE - 4 + PAGE_SIZE;
        writer.write_u32::<LittleEndian>(record_len as u32)?;
        writer.write_u32::<LittleEndian>(self.page_id)?;
        writer.write_u32::<LittleEndian>(self.checksum)?;
        writer.write_all(&self.data)?;
        Ok(())
    }

    /// Deserializes a record from a reader.
    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Option<Self>> {
        // Read record length
        let record_len = match reader.read_u32::<LittleEndian>() {
            Ok(len) => len,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let expected_len = WAL_RECORD_HEADER_SIZE - 4 + PAGE_SIZE;
        if record_len as usize != expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid WAL record length: {}", record_len),
            ));
        }

        let page_id = reader.read_u32::<LittleEndian>()?;
        let checksum = reader.read_u32::<LittleEndian>()?;

        let mut data = [0u8; PAGE_SIZE];
        reader.read_exact(&mut data)?;

        let record = WalRecord {
            page_id,
            checksum,
            data,
        };

        if !record.verify_checksum() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("WAL record checksum mismatch for page {}", page_id),
            ));
        }

        Ok(Some(record))
    }
}

/// Write-Ahead Log manager.
pub struct WAL {
    /// Path to the WAL file (kept for potential future use)
    #[allow(dead_code)]
    path: PathBuf,
    /// File handle for the WAL
    file: File,
    /// Current write position in the WAL
    write_offset: u64,
    /// Whether the WAL is enabled
    enabled: bool,
}

impl WAL {
    /// Creates or opens a WAL file for the given database path.
    pub fn open(db_path: &Path) -> io::Result<Self> {
        let wal_path = Self::wal_path(db_path);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&wal_path)?;

        let mut wal = WAL {
            path: wal_path,
            file,
            write_offset: 0,
            enabled: true,
        };

        // Initialize or validate header
        let file_len = wal.file.seek(SeekFrom::End(0))?;
        if file_len == 0 {
            // New WAL file, write header
            wal.write_header()?;
        } else {
            // Existing WAL file, validate header
            wal.validate_header()?;
            wal.write_offset = file_len;
        }

        Ok(wal)
    }

    /// Creates a disabled (no-op) WAL for testing.
    pub fn disabled() -> Self {
        // Create a dummy file that won't be used
        WAL {
            path: PathBuf::new(),
            file: unsafe { std::mem::zeroed() }, // Never used
            write_offset: 0,
            enabled: false,
        }
    }

    /// Returns the WAL file path for a database path.
    pub fn wal_path(db_path: &Path) -> PathBuf {
        let mut wal_path = db_path.to_path_buf();
        let file_name = wal_path.file_name().unwrap_or_default().to_string_lossy();
        wal_path.set_file_name(format!("{}-wal", file_name));
        wal_path
    }

    /// Writes the WAL header.
    fn write_header(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;

        let mut header = [0u8; WAL_HEADER_SIZE];
        header[..WAL_MAGIC_LEN].copy_from_slice(WAL_MAGIC);

        self.file.write_all(&header)?;
        self.file.sync_all()?;

        self.write_offset = WAL_HEADER_SIZE as u64;
        Ok(())
    }

    /// Validates the WAL header.
    fn validate_header(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;

        let mut header = [0u8; WAL_HEADER_SIZE];
        self.file.read_exact(&mut header)?;

        if &header[..WAL_MAGIC_LEN] != WAL_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid WAL magic bytes",
            ));
        }

        Ok(())
    }

    /// Logs a page modification to the WAL.
    pub fn log_page(&mut self, page_id: u32, data: &[u8; PAGE_SIZE]) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let record = WalRecord::new(page_id, *data);

        self.file.seek(SeekFrom::Start(self.write_offset))?;

        {
            let mut writer = BufWriter::new(&mut self.file);
            record.serialize(&mut writer)?;
            writer.flush()?;
        }

        // Sync to ensure durability
        self.file.sync_all()?;

        self.write_offset += (WAL_RECORD_HEADER_SIZE + PAGE_SIZE) as u64;

        Ok(())
    }

    /// Returns the current WAL size in bytes.
    pub fn size(&self) -> u64 {
        self.write_offset
    }

    /// Returns true if there are any records in the WAL.
    pub fn has_records(&self) -> bool {
        self.write_offset > WAL_HEADER_SIZE as u64
    }

    /// Reads all records from the WAL for recovery.
    pub fn read_records(&mut self) -> io::Result<Vec<WalRecord>> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        let mut records = Vec::new();

        self.file.seek(SeekFrom::Start(WAL_HEADER_SIZE as u64))?;
        let mut reader = BufReader::new(&mut self.file);

        loop {
            match WalRecord::deserialize(&mut reader) {
                Ok(Some(record)) => records.push(record),
                Ok(None) => break, // End of file
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
        }

        Ok(records)
    }

    /// Checkpoints the WAL by truncating it (called after all records are applied).
    pub fn checkpoint(&mut self) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Truncate the file to just the header
        self.file.set_len(WAL_HEADER_SIZE as u64)?;
        self.write_offset = WAL_HEADER_SIZE as u64;
        self.file.sync_all()?;

        Ok(())
    }

    /// Syncs the WAL to disk.
    pub fn sync(&mut self) -> io::Result<()> {
        if self.enabled {
            self.file.sync_all()
        } else {
            Ok(())
        }
    }

    /// Deletes the WAL file.
    pub fn delete(db_path: &Path) -> io::Result<()> {
        let wal_path = Self::wal_path(db_path);
        if wal_path.exists() {
            std::fs::remove_file(wal_path)?;
        }
        Ok(())
    }
}

/// Recovery module for replaying WAL on startup.
pub mod recovery {
    use super::*;
    use crate::pager::Pager;

    /// Recovers a database by replaying the WAL if it exists.
    /// Returns the number of records replayed.
    pub fn recover(db_path: &Path, pager: &mut Pager) -> io::Result<usize> {
        let wal_path = WAL::wal_path(db_path);

        if !wal_path.exists() {
            return Ok(0);
        }

        let mut wal = WAL::open(db_path)?;

        if !wal.has_records() {
            return Ok(0);
        }

        // Read all records
        let records = wal.read_records()?;
        let count = records.len();

        // Apply each record to the database
        for record in records {
            pager.write_page(record.page_id, &record.data)?;
        }

        // Sync the database
        pager.file_mut().sync_all()?;

        // Checkpoint the WAL (clear it)
        wal.checkpoint()?;

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_wal_record_serialize_deserialize() {
        let mut data = [0u8; PAGE_SIZE];
        data[0] = 0x42;
        data[100] = 0xAB;
        data[PAGE_SIZE - 1] = 0xFF;

        let record = WalRecord::new(42, data);
        assert!(record.verify_checksum());

        let mut buffer = Vec::new();
        record.serialize(&mut buffer).unwrap();

        let mut cursor = std::io::Cursor::new(buffer);
        let deserialized = WalRecord::deserialize(&mut cursor).unwrap().unwrap();

        assert_eq!(record.page_id, deserialized.page_id);
        assert_eq!(record.checksum, deserialized.checksum);
        assert_eq!(record.data, deserialized.data);
    }

    #[test]
    fn test_wal_open_and_write() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Create a dummy database file
        File::create(&db_path).unwrap();

        let mut wal = WAL::open(&db_path).unwrap();
        assert!(!wal.has_records());

        // Write a record
        let mut data = [0u8; PAGE_SIZE];
        data[0] = 0x42;
        wal.log_page(1, &data).unwrap();

        assert!(wal.has_records());

        // Read records
        let records = wal.read_records().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].page_id, 1);
        assert_eq!(records[0].data[0], 0x42);

        // Checkpoint
        wal.checkpoint().unwrap();
        assert!(!wal.has_records());
    }

    #[test]
    fn test_wal_multiple_records() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        File::create(&db_path).unwrap();

        let mut wal = WAL::open(&db_path).unwrap();

        // Write multiple records
        for i in 0..10 {
            let mut data = [0u8; PAGE_SIZE];
            data[0] = i as u8;
            wal.log_page(i, &data).unwrap();
        }

        let records = wal.read_records().unwrap();
        assert_eq!(records.len(), 10);

        for (i, record) in records.iter().enumerate() {
            assert_eq!(record.page_id, i as u32);
            assert_eq!(record.data[0], i as u8);
        }
    }
}
