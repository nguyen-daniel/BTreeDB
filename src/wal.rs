//! Write-Ahead Logging (WAL) module for crash recovery and durability.
//!
//! Page writes are logged as **frames**. One frame holds one or more page
//! images (a single `write_page`, or every page from an `insert` / `delete`
//! write group), a CRC32, and is fsynced once. Recovery replays complete
//! frames only. A torn last frame (short read) or a corrupt last frame
//! (checksum / invalid length) is discarded, including any bytes after the
//! last good frame, so the next append does not write past unparseable data.

use crate::pager::PAGE_SIZE;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Magic bytes for WAL file identification.
const WAL_MAGIC: &[u8] = b"BTREEWAL";
const WAL_MAGIC_LEN: usize = 8;

/// WAL frame format version stored after the magic bytes.
const WAL_FORMAT_VERSION: u16 = 2;

/// WAL file header size.
pub const WAL_HEADER_SIZE: usize = 32;

/// Upper bound on pages in one frame (a single insert/delete writes a handful).
const MAX_FRAME_PAGES: u32 = 256;

/// Frame prefix after `frame_len`: page_count (4) + checksum (4).
const WAL_FRAME_META_SIZE: usize = 8;

/// Bytes of one page entry: page_id (4) + data (PAGE_SIZE).
const WAL_PAGE_ENTRY_SIZE: usize = 4 + PAGE_SIZE;

/// A single page image inside a WAL frame.
#[derive(Debug, Clone)]
pub struct WalRecord {
    /// Page ID that was modified
    pub page_id: u32,
    /// CRC32 of the page data
    pub checksum: u32,
    /// The page data (4096 bytes)
    pub data: [u8; PAGE_SIZE],
}

impl WalRecord {
    /// Creates a new WAL record.
    pub fn new(page_id: u32, data: [u8; PAGE_SIZE]) -> Self {
        let checksum = crc32_of(&data);
        WalRecord {
            page_id,
            checksum,
            data,
        }
    }

    /// Verifies the checksum of the record.
    pub fn verify_checksum(&self) -> bool {
        self.checksum == crc32_of(&self.data)
    }
}

/// One atomic WAL unit: one or more page images, checksummed together.
#[derive(Debug, Clone)]
pub struct WalFrame {
    /// Pages in this frame, in write order (last write to a page wins).
    pub pages: Vec<WalRecord>,
}

impl WalFrame {
    /// Builds a frame from page images.
    pub fn from_pages(pages: &[(u32, [u8; PAGE_SIZE])]) -> Self {
        WalFrame {
            pages: pages
                .iter()
                .map(|(page_id, data)| WalRecord::new(*page_id, *data))
                .collect(),
        }
    }

    /// On-disk size of a frame with `page_count` pages, including `frame_len`.
    pub fn encoded_len(page_count: usize) -> u64 {
        (4 + WAL_FRAME_META_SIZE + page_count * WAL_PAGE_ENTRY_SIZE) as u64
    }

    fn payload_len(page_count: usize) -> u32 {
        (WAL_FRAME_META_SIZE + page_count * WAL_PAGE_ENTRY_SIZE) as u32
    }

    fn checksum_records(pages: &[WalRecord]) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&(pages.len() as u32).to_le_bytes());
        for record in pages {
            hasher.update(&record.page_id.to_le_bytes());
            hasher.update(&record.data);
        }
        hasher.finalize()
    }

    fn checksum_pages(pages: &[(u32, [u8; PAGE_SIZE])]) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&(pages.len() as u32).to_le_bytes());
        for (page_id, data) in pages {
            hasher.update(&page_id.to_le_bytes());
            hasher.update(data);
        }
        hasher.finalize()
    }

    /// Serializes the frame to a writer.
    pub fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.pages.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WAL frame must contain at least one page",
            ));
        }
        if self.pages.len() > MAX_FRAME_PAGES as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("WAL frame has too many pages: {}", self.pages.len()),
            ));
        }

        let checksum = Self::checksum_records(&self.pages);

        writer.write_u32::<LittleEndian>(Self::payload_len(self.pages.len()))?;
        writer.write_u32::<LittleEndian>(self.pages.len() as u32)?;
        writer.write_u32::<LittleEndian>(checksum)?;
        for record in &self.pages {
            writer.write_u32::<LittleEndian>(record.page_id)?;
            writer.write_all(&record.data)?;
        }
        Ok(())
    }

    /// Deserializes a frame. `Ok(None)` means end of file or a torn last frame.
    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Option<Self>> {
        let payload_len = match reader.read_u32::<LittleEndian>() {
            Ok(len) => len,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        };

        let max_payload = WAL_FRAME_META_SIZE as u32 + MAX_FRAME_PAGES * WAL_PAGE_ENTRY_SIZE as u32;
        if payload_len < WAL_FRAME_META_SIZE as u32 || payload_len > max_payload {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid WAL frame length: {payload_len}"),
            ));
        }

        let mut payload = vec![0u8; payload_len as usize];
        match reader.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }

        let mut cursor = io::Cursor::new(&payload);
        let page_count = cursor.read_u32::<LittleEndian>()?;
        let checksum = cursor.read_u32::<LittleEndian>()?;

        let expected_len = Self::payload_len(page_count as usize);
        if page_count == 0 || page_count > MAX_FRAME_PAGES || payload_len != expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid WAL frame page_count: {page_count}"),
            ));
        }

        let mut pages = Vec::with_capacity(page_count as usize);
        for _ in 0..page_count {
            let page_id = cursor.read_u32::<LittleEndian>()?;
            let mut data = [0u8; PAGE_SIZE];
            cursor.read_exact(&mut data)?;
            pages.push((page_id, data));
        }

        if checksum != Self::checksum_pages(&pages) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WAL frame checksum mismatch",
            ));
        }

        Ok(Some(WalFrame::from_pages(&pages)))
    }
}

fn crc32_of(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

/// Write-Ahead Log manager.
pub struct WAL {
    /// Path to the WAL file (kept for potential future use)
    #[allow(dead_code)]
    path: PathBuf,
    /// File handle for the WAL (`None` when disabled)
    file: Option<File>,
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
            file: Some(file),
            write_offset: 0,
            enabled: true,
        };

        // Initialize or validate header
        let file_len = wal.file_mut()?.seek(SeekFrom::End(0))?;
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
        WAL {
            path: PathBuf::new(),
            file: None,
            write_offset: 0,
            enabled: false,
        }
    }

    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("WAL is disabled"))
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
        {
            let file = self.file_mut()?;
            file.seek(SeekFrom::Start(0))?;

            let mut header = [0u8; WAL_HEADER_SIZE];
            header[..WAL_MAGIC_LEN].copy_from_slice(WAL_MAGIC);
            header[WAL_MAGIC_LEN..WAL_MAGIC_LEN + 2]
                .copy_from_slice(&WAL_FORMAT_VERSION.to_le_bytes());

            file.write_all(&header)?;
            file.sync_all()?;
        }

        self.write_offset = WAL_HEADER_SIZE as u64;
        Ok(())
    }

    /// Validates the WAL header.
    fn validate_header(&mut self) -> io::Result<()> {
        let file = self.file_mut()?;
        file.seek(SeekFrom::Start(0))?;

        let mut header = [0u8; WAL_HEADER_SIZE];
        file.read_exact(&mut header)?;

        if &header[..WAL_MAGIC_LEN] != WAL_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid WAL magic bytes",
            ));
        }

        let version = u16::from_le_bytes(
            header[WAL_MAGIC_LEN..WAL_MAGIC_LEN + 2]
                .try_into()
                .expect("version slice is 2 bytes"),
        );
        if version != WAL_FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported WAL format version {version} (expected {WAL_FORMAT_VERSION})"),
            ));
        }

        Ok(())
    }

    /// Logs a single page as a one-page frame.
    pub fn log_page(&mut self, page_id: u32, data: &[u8; PAGE_SIZE]) -> io::Result<()> {
        self.log_pages(&[(page_id, *data)])
    }

    /// Logs one or more pages as a single fsynced WAL frame.
    pub fn log_pages(&mut self, pages: &[(u32, [u8; PAGE_SIZE])]) -> io::Result<()> {
        if !self.enabled || pages.is_empty() {
            return Ok(());
        }

        let frame = WalFrame::from_pages(pages);
        let offset = self.write_offset;

        self.file_mut()?.seek(SeekFrom::Start(offset))?;

        {
            let mut writer = BufWriter::new(self.file_mut()?);
            frame.serialize(&mut writer)?;
            writer.flush()?;
        }

        self.file_mut()?.sync_all()?;
        self.write_offset += WalFrame::encoded_len(pages.len());

        Ok(())
    }

    /// Returns the current WAL size in bytes.
    pub fn size(&self) -> u64 {
        self.write_offset
    }

    /// Drops bytes after `offset` so a failed DB apply can undo the last frame.
    ///
    /// `offset` must be at or after the header. A mark at or past the current
    /// tail is a no-op.
    pub fn truncate_to(&mut self, offset: u64) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if offset < WAL_HEADER_SIZE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot truncate WAL below header",
            ));
        }
        if offset >= self.write_offset {
            return Ok(());
        }
        let file = self.file_mut()?;
        file.set_len(offset)?;
        file.sync_all()?;
        self.write_offset = offset;
        Ok(())
    }

    /// Returns true if there are any records in the WAL.
    pub fn has_records(&self) -> bool {
        self.write_offset > WAL_HEADER_SIZE as u64
    }

    /// Reads complete frames from the WAL.
    ///
    /// A short last frame, or a last frame that fails checksum / length checks,
    /// is omitted. Bytes after the last good frame are truncated so a later
    /// `log_pages` overwrites the tail instead of appending past it. A bad
    /// frame in the middle of the file also stops replay (the unreadable tail
    /// is not applied); that matches crash-torn tails, which are always last.
    pub fn read_frames(&mut self) -> io::Result<Vec<WalFrame>> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        let mut frames = Vec::new();
        let mut good_end = WAL_HEADER_SIZE as u64;

        {
            let file = self.file_mut()?;
            file.seek(SeekFrom::Start(WAL_HEADER_SIZE as u64))?;
            let mut reader = BufReader::new(file);

            loop {
                match WalFrame::deserialize(&mut reader) {
                    Ok(Some(frame)) => {
                        good_end += WalFrame::encoded_len(frame.pages.len());
                        frames.push(frame);
                    }
                    Ok(None) => break, // End of file or torn last frame
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(e) if e.kind() == io::ErrorKind::InvalidData => break,
                    Err(e) => return Err(e),
                }
            }
        }

        if self.write_offset > good_end {
            let file = self.file_mut()?;
            file.set_len(good_end)?;
            file.sync_all()?;
            self.write_offset = good_end;
        }

        Ok(frames)
    }

    /// Reads all pages from complete frames (flattened) for recovery.
    pub fn read_records(&mut self) -> io::Result<Vec<WalRecord>> {
        Ok(self
            .read_frames()?
            .into_iter()
            .flat_map(|frame| frame.pages)
            .collect())
    }

    /// Checkpoints the WAL by truncating it (called after all records are applied).
    pub fn checkpoint(&mut self) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Truncate the file to just the header
        {
            let file = self.file_mut()?;
            file.set_len(WAL_HEADER_SIZE as u64)?;
            file.sync_all()?;
        }
        self.write_offset = WAL_HEADER_SIZE as u64;

        Ok(())
    }

    /// Syncs the WAL to disk.
    pub fn sync(&mut self) -> io::Result<()> {
        if self.enabled {
            self.file_mut()?.sync_all()
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

        // Complete frames only; a torn or corrupt last frame is dropped.
        let frames = wal.read_frames()?;
        let mut count = 0;

        for frame in frames {
            for record in frame.pages {
                pager.write_page(record.page_id, &record.data)?;
                count += 1;
            }
        }

        // Fsync the database before truncating the WAL (WAL-first durability).
        pager.sync_file()?;

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
    fn test_crc32_replaces_wrapping_add() {
        let mut data = [0u8; PAGE_SIZE];
        data[0] = 1;
        data[1] = 2;
        data[2] = 3;
        data[3] = 4;
        let record = WalRecord::new(0, data);
        assert!(record.verify_checksum());
        assert_eq!(record.checksum, crc32fast::hash(&data));
        // Old wrapping-add of the first word (rest zeros) was 0x04030201.
        assert_ne!(record.checksum, u32::from_le_bytes([1, 2, 3, 4]));
    }

    #[test]
    fn test_wal_frame_serialize_deserialize() {
        let mut data = [0u8; PAGE_SIZE];
        data[0] = 0x42;
        data[100] = 0xAB;
        data[PAGE_SIZE - 1] = 0xFF;

        let frame = WalFrame::from_pages(&[(42, data)]);
        assert!(frame.pages[0].verify_checksum());

        let mut buffer = Vec::new();
        frame.serialize(&mut buffer).unwrap();
        assert_eq!(buffer.len() as u64, WalFrame::encoded_len(1));

        let mut cursor = std::io::Cursor::new(buffer);
        let deserialized = WalFrame::deserialize(&mut cursor).unwrap().unwrap();

        assert_eq!(deserialized.pages.len(), 1);
        assert_eq!(frame.pages[0].page_id, deserialized.pages[0].page_id);
        assert_eq!(frame.pages[0].checksum, deserialized.pages[0].checksum);
        assert_eq!(frame.pages[0].data, deserialized.pages[0].data);
    }

    #[test]
    fn test_wal_multi_page_frame_is_atomic_unit() {
        let mut left = [0u8; PAGE_SIZE];
        let mut right = [0u8; PAGE_SIZE];
        left[0] = 1;
        right[0] = 2;
        let frame = WalFrame::from_pages(&[(1, left), (2, right)]);

        let mut buffer = Vec::new();
        frame.serialize(&mut buffer).unwrap();

        let mut cursor = std::io::Cursor::new(buffer);
        let deserialized = WalFrame::deserialize(&mut cursor).unwrap().unwrap();
        assert_eq!(deserialized.pages.len(), 2);
        assert_eq!(deserialized.pages[0].page_id, 1);
        assert_eq!(deserialized.pages[1].page_id, 2);
        assert_eq!(deserialized.pages[0].data[0], 1);
        assert_eq!(deserialized.pages[1].data[0], 2);
    }

    #[test]
    fn test_wal_frame_checksum_detects_corruption() {
        let mut data = [0u8; PAGE_SIZE];
        data[0] = 0x42;
        let frame = WalFrame::from_pages(&[(1, data)]);
        let mut buffer = Vec::new();
        frame.serialize(&mut buffer).unwrap();
        let flip = buffer.len() - 1;
        buffer[flip] ^= 0xFF;

        let mut cursor = std::io::Cursor::new(buffer);
        let err = WalFrame::deserialize(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("checksum"));
    }

    #[test]
    fn test_torn_frame_is_ignored() {
        let mut a = [0u8; PAGE_SIZE];
        let mut b = [0u8; PAGE_SIZE];
        a[0] = 1;
        b[0] = 2;
        let frame = WalFrame::from_pages(&[(1, a), (2, b)]);
        let mut buffer = Vec::new();
        frame.serialize(&mut buffer).unwrap();
        buffer.truncate(buffer.len() / 2);

        let mut cursor = std::io::Cursor::new(buffer);
        assert!(WalFrame::deserialize(&mut cursor).unwrap().is_none());
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
    fn test_wal_disabled_is_noop() {
        let mut wal = WAL::disabled();
        let mut data = [0u8; PAGE_SIZE];
        data[0] = 0x42;
        wal.log_page(1, &data).unwrap();
        assert!(!wal.has_records());
        assert!(wal.read_records().unwrap().is_empty());
        wal.checkpoint().unwrap();
        wal.sync().unwrap();
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

    #[test]
    fn test_wal_log_pages_one_frame() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        File::create(&db_path).unwrap();

        let mut wal = WAL::open(&db_path).unwrap();
        let mut p1 = [0u8; PAGE_SIZE];
        let mut p2 = [0u8; PAGE_SIZE];
        p1[0] = 9;
        p2[0] = 8;
        wal.log_pages(&[(3, p1), (4, p2)]).unwrap();

        let frames = wal.read_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pages.len(), 2);
        assert_eq!(frames[0].pages[0].page_id, 3);
        assert_eq!(frames[0].pages[1].page_id, 4);
    }

    #[test]
    fn test_wal_truncate_to_undoes_last_frame() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        File::create(&db_path).unwrap();

        let mut wal = WAL::open(&db_path).unwrap();
        let mut first = [0u8; PAGE_SIZE];
        let mut second = [0u8; PAGE_SIZE];
        first[0] = 1;
        second[0] = 2;
        wal.log_pages(&[(1, first)]).unwrap();
        let mark = wal.size();
        wal.log_pages(&[(2, second)]).unwrap();
        assert_eq!(wal.read_frames().unwrap().len(), 2);

        wal.truncate_to(mark).unwrap();
        let frames = wal.read_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pages[0].page_id, 1);

        wal.log_pages(&[(3, second)]).unwrap();
        let frames = wal.read_frames().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].pages[0].page_id, 3);
    }

    #[test]
    fn test_wal_rejects_old_format_version() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        File::create(&db_path).unwrap();

        let wal_path = WAL::wal_path(&db_path);
        let mut header = [0u8; WAL_HEADER_SIZE];
        header[..WAL_MAGIC_LEN].copy_from_slice(WAL_MAGIC);
        // version 0 (legacy zeros after magic) must fail open
        std::fs::write(&wal_path, header).unwrap();

        let err = match WAL::open(&db_path) {
            Ok(_) => panic!("old WAL format version must fail open"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("Unsupported WAL format version"),
            "error={err}"
        );
    }

    #[test]
    fn test_corrupt_last_frame_is_ignored_and_tail_trimmed() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        File::create(&db_path).unwrap();

        let mut wal = WAL::open(&db_path).unwrap();
        let mut first = [0u8; PAGE_SIZE];
        first[0] = 1;
        wal.log_page(1, &first).unwrap();
        let good_size = wal.size();
        drop(wal);

        let mut second = [0u8; PAGE_SIZE];
        second[0] = 2;
        let frame = WalFrame::from_pages(&[(2, second)]);
        let mut extra = Vec::new();
        frame.serialize(&mut extra).unwrap();
        let flip = extra.len() - 1;
        extra[flip] ^= 0xFF;

        let wal_path = WAL::wal_path(&db_path);
        {
            let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
            file.write_all(&extra).unwrap();
        }

        let mut wal = WAL::open(&db_path).unwrap();
        let frames = wal.read_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pages[0].page_id, 1);
        assert_eq!(frames[0].pages[0].data[0], 1);
        assert_eq!(wal.size(), good_size);
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), good_size);

        let mut third = [0u8; PAGE_SIZE];
        third[0] = 3;
        wal.log_page(3, &third).unwrap();
        let frames = wal.read_frames().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].pages[0].page_id, 3);
        assert_eq!(frames[1].pages[0].data[0], 3);
    }

    #[test]
    fn test_invalid_last_frame_length_is_ignored() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        File::create(&db_path).unwrap();

        let mut wal = WAL::open(&db_path).unwrap();
        let mut first = [0u8; PAGE_SIZE];
        first[0] = 9;
        wal.log_page(1, &first).unwrap();
        let good_size = wal.size();
        drop(wal);

        let wal_path = WAL::wal_path(&db_path);
        {
            let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
            file.write_all(&0u32.to_le_bytes()).unwrap();
            file.write_all(&[0xAAu8; 8]).unwrap();
        }

        let mut wal = WAL::open(&db_path).unwrap();
        let frames = wal.read_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pages[0].data[0], 9);
        assert_eq!(wal.size(), good_size);
    }
}
