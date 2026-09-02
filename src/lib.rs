pub mod btree;
pub mod cursor;
pub mod node;
pub mod pager;
pub mod wal;

/// On-disk page size in bytes. Shared by the pager, node codec, and WAL.
pub const PAGE_SIZE: usize = 4096;

/// Database file format version stored in the page-0 header.
///
/// Version 2 is a breaking on-disk change: the header already stored
/// `format_version` and `page_size`, but files were still labeled v1.
/// Older files fail to open with a clear error.
pub const FORMAT_VERSION: u16 = 2;

#[cfg(feature = "experimental")]
pub mod backup;
#[cfg(feature = "experimental")]
pub mod compression;
#[cfg(feature = "experimental")]
pub mod concurrency;
#[cfg(feature = "experimental")]
pub mod manager;
#[cfg(feature = "experimental")]
pub mod transaction;
#[cfg(feature = "experimental")]
pub mod value;
