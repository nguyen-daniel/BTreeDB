pub mod btree;
pub mod cursor;
pub mod node;
pub mod pager;
pub mod wal;

/// On-disk page size in bytes. Shared by the pager, node codec, and WAL.
pub const PAGE_SIZE: usize = 4096;

/// Database file format version stored in the page-0 header.
pub const FORMAT_VERSION: u16 = 1;

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
