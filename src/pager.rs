use crate::wal::{recovery, WAL};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub use crate::PAGE_SIZE;

/// Pager manages file I/O for a persistent B-Tree database.
/// It handles reading and writing fixed-size pages to/from disk.
/// When opened via [`Pager::open`], writes are logged to the WAL first.
///
/// # Durability policy
///
/// Crash safety is **WAL-first**:
///
/// 1. [`Pager::open`] / [`crate::btree::BTree::open`] attach a WAL.
///    Ungrouped [`write_page`] calls log a one-page frame (`File::sync_all`),
///    then write the page to the database file and `flush()` it. The DB file
///    is **not** fsynced on every write.
/// 2. [`begin_write_group`] / [`commit_write_group`] buffer page writes and
///    append them as **one WAL frame** (one fsync), then apply every page to
///    the DB file. [`crate::btree::BTree::insert`] and `delete` use a group so
///    a split, merge, or root change is one atomic WAL unit. Recovery replays
///    a complete frame in full, or ignores a torn last frame — it does not
///    apply a prefix of the group. Reads during a group see buffered pages
///    (no separate page cache lives across operations).
/// 3. A crash after the WAL fsync is recovered by replaying complete frames
///    on the next open. Replay applies pages, fsyncs the DB file, then
///    checkpoints (truncates) the WAL.
/// 4. [`crate::btree::BTree::sync`] fsyncs the DB file (`sync_all`) and then
///    checkpoints the WAL. After a successful sync, the WAL is empty and the
///    DB file is the durable copy.
/// 5. [`Pager::new`] does **not** attach a WAL. Writes are flushed only; call
///    [`Pager::sync_file`] (or `BTree::sync`) if the caller needs the DB file
///    durable. This path is for tests and tools that inject pages without logging.
pub struct Pager {
    file: File,
    wal: Option<WAL>,
    grouping: bool,
    write_batch: Vec<(u32, [u8; PAGE_SIZE])>,
}

impl Pager {
    /// Creates a new Pager from an existing file (no WAL).
    pub fn new(file: File) -> Self {
        Pager {
            file,
            wal: None,
            grouping: false,
            write_batch: Vec::new(),
        }
    }

    /// Opens a database path, replays any WAL, then attaches WAL for new writes.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let mut pager = Pager {
            file,
            wal: None,
            grouping: false,
            write_batch: Vec::new(),
        };
        recovery::recover(path, &mut pager)?;
        pager.wal = Some(WAL::open(path)?);
        Ok(pager)
    }

    /// Starts buffering page writes into one WAL frame.
    pub fn begin_write_group(&mut self) -> std::io::Result<()> {
        if self.grouping {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "write group already active",
            ));
        }
        self.grouping = true;
        self.write_batch.clear();
        Ok(())
    }

    /// Logs the buffered pages as one WAL frame, then applies them to the file.
    pub fn commit_write_group(&mut self) -> std::io::Result<()> {
        if !self.grouping {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "no write group is active",
            ));
        }
        let pages = std::mem::take(&mut self.write_batch);
        self.grouping = false;
        self.persist_pages(&pages)
    }

    /// Drops buffered writes without logging or applying them.
    pub fn abort_write_group(&mut self) {
        self.write_batch.clear();
        self.grouping = false;
    }

    fn batch_upsert(&mut self, page_id: u32, data: &[u8]) {
        if let Some((_, existing)) = self.write_batch.iter_mut().find(|(id, _)| *id == page_id) {
            existing.copy_from_slice(data);
            return;
        }
        let mut page = [0u8; PAGE_SIZE];
        page.copy_from_slice(data);
        self.write_batch.push((page_id, page));
    }

    fn persist_pages(&mut self, pages: &[(u32, [u8; PAGE_SIZE])]) -> std::io::Result<()> {
        if pages.is_empty() {
            return Ok(());
        }

        if let Some(wal) = &mut self.wal {
            wal.log_pages(pages)?;
        }

        for (page_id, data) in pages {
            let offset = (*page_id as u64) * (PAGE_SIZE as u64);
            self.file.seek(SeekFrom::Start(offset))?;
            self.file.write_all(data)?;
        }
        self.file.flush()?;
        Ok(())
    }

    /// Checkpoints (truncates) the WAL after the main file is durable.
    pub fn checkpoint(&mut self) -> std::io::Result<()> {
        if let Some(wal) = &mut self.wal {
            wal.checkpoint()?;
        }
        Ok(())
    }

    /// Gets a mutable reference to the underlying file.
    /// This is useful for syncing all data to disk.
    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    /// Fsyncs the database file. Does not checkpoint the WAL.
    ///
    /// Used by recovery (before truncating the WAL) and by [`crate::btree::BTree::sync`].
    pub fn sync_file(&mut self) -> std::io::Result<()> {
        self.file.sync_all()
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
    ///
    /// During an active write group, a buffered write to `page_id` is returned
    /// so split/merge/rebalance can read pages they just wrote.
    pub fn get_page(&mut self, page_id: u32) -> std::io::Result<[u8; PAGE_SIZE]> {
        if self.grouping {
            if let Some((_, data)) = self.write_batch.iter().rev().find(|(id, _)| *id == page_id) {
                return Ok(*data);
            }
        }

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
    ///
    /// Inside a write group the page is buffered. Otherwise, with a WAL
    /// attached, the page is fsynced as a one-page frame first; the DB file
    /// is then written and flushed but not fsynced (see the durability policy
    /// on [`Pager`]).
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

        if self.grouping {
            self.batch_upsert(page_id, data);
            return Ok(());
        }

        let mut page = [0u8; PAGE_SIZE];
        page.copy_from_slice(data);
        self.persist_pages(&[(page_id, page)])
    }
}
