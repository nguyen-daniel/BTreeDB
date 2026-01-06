use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// Page size in bytes (4KB)
pub const PAGE_SIZE: usize = 4096;

/// Pager manages file I/O for a persistent B-Tree database.
/// It handles reading and writing fixed-size pages to/from disk.
pub struct Pager {
    file: File,
}

impl Pager {
    /// Creates a new Pager from an existing file.
    pub fn new(file: File) -> Self {
        Pager { file }
    }

    /// Gets a mutable reference to the underlying file.
    /// This is useful for syncing all data to disk.
    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    /// Returns the total number of pages in the file.
    /// Calculated as file_size / PAGE_SIZE, rounded up.
    /// Returns 0 for empty files.
    pub fn page_count(&mut self) -> std::io::Result<u32> {
        let file_len = self.file.seek(SeekFrom::End(0))?;
        if file_len == 0 {
            Ok(0)
        } else {
            // Round up to account for partially written pages
            Ok(file_len.div_ceil(PAGE_SIZE as u64) as u32)
        }
    }

    /// Reads a page from the file at the given page_id.
    /// Returns a 4096-byte buffer containing the page data.
    /// If the page doesn't exist yet, returns a buffer filled with zeros.
    pub fn get_page(&mut self, page_id: u32) -> std::io::Result<[u8; PAGE_SIZE]> {
        let offset = (page_id as u64) * (PAGE_SIZE as u64);

        // Seek to the correct position
        self.file.seek(SeekFrom::Start(offset))?;

        // Read the page data
        let mut buffer = [0u8; PAGE_SIZE];
        match self.file.read_exact(&mut buffer) {
            Ok(_) => Ok(buffer),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Page doesn't exist yet, return zeros
                Ok([0u8; PAGE_SIZE])
            }
            Err(e) => Err(e),
        }
    }

    /// Writes a page to the file at the given page_id.
    /// The data slice must be exactly PAGE_SIZE bytes.
    pub fn write_page(&mut self, page_id: u32, data: &[u8]) -> std::io::Result<()> {
        if data.len() != PAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Data must be exactly {} bytes, got {}",
                    PAGE_SIZE,
                    data.len()
                ),
            ));
        }

        let offset = (page_id as u64) * (PAGE_SIZE as u64);

        // Seek to the correct position
        self.file.seek(SeekFrom::Start(offset))?;

        // Write the page data
        self.file.write_all(data)?;
        // Flush to ensure data is written (but don't sync to disk for performance)
        self.file.flush()?;
        // Note: sync_data removed for benchmarking - can cause issues with temp files
        // In production, you may want to sync periodically rather than on every write
        // self.file.sync_data()?;

        Ok(())
    }
}
