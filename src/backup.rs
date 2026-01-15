//! Backup and restore module for database safety.
//!
//! Provides functionality to create hot backups and restore from backups.

use crate::wal::WAL;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Buffer size for copying files (64KB).
const COPY_BUFFER_SIZE: usize = 64 * 1024;

/// Backup metadata.
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// Size of the main database file
    pub db_size: u64,
    /// Size of the WAL file (if any)
    pub wal_size: u64,
    /// Whether WAL was included in backup
    pub includes_wal: bool,
}

/// Creates a backup of the database to the specified destination.
///
/// This performs a "hot backup" by:
/// 1. Copying the main database file
/// 2. Optionally copying the WAL file
///
/// Note: For a consistent backup in production, you should:
/// - Checkpoint the WAL first
/// - Hold a lock during the copy
pub fn backup(db_path: &Path, dest_path: &Path, include_wal: bool) -> io::Result<BackupInfo> {
    // Check source exists
    if !db_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Database file not found: {}", db_path.display()),
        ));
    }

    // Create destination directory if needed
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Copy main database file
    let db_size = copy_file(db_path, dest_path)?;

    // Optionally copy WAL file
    let mut wal_size = 0;
    let mut includes_wal = false;

    if include_wal {
        let wal_src = WAL::wal_path(db_path);
        if wal_src.exists() {
            let wal_dest = WAL::wal_path(dest_path);
            wal_size = copy_file(&wal_src, &wal_dest)?;
            includes_wal = true;
        }
    }

    Ok(BackupInfo {
        db_size,
        wal_size,
        includes_wal,
    })
}

/// Restores a database from a backup.
///
/// This will:
/// 1. Copy the backup database file to the destination
/// 2. Optionally restore the WAL file
pub fn restore(backup_path: &Path, dest_path: &Path, restore_wal: bool) -> io::Result<BackupInfo> {
    // Check backup exists
    if !backup_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Backup file not found: {}", backup_path.display()),
        ));
    }

    // Create destination directory if needed
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Copy main database file
    let db_size = copy_file(backup_path, dest_path)?;

    // Optionally restore WAL file
    let mut wal_size = 0;
    let mut includes_wal = false;

    if restore_wal {
        let wal_src = WAL::wal_path(backup_path);
        if wal_src.exists() {
            let wal_dest = WAL::wal_path(dest_path);
            wal_size = copy_file(&wal_src, &wal_dest)?;
            includes_wal = true;
        }
    }

    Ok(BackupInfo {
        db_size,
        wal_size,
        includes_wal,
    })
}

/// Verifies a backup by checking file existence and readability.
pub fn verify_backup(backup_path: &Path) -> io::Result<BackupInfo> {
    if !backup_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Backup file not found: {}", backup_path.display()),
        ));
    }

    let db_metadata = fs::metadata(backup_path)?;
    let db_size = db_metadata.len();

    // Check WAL
    let wal_path = WAL::wal_path(backup_path);
    let (wal_size, includes_wal) = if wal_path.exists() {
        let wal_metadata = fs::metadata(&wal_path)?;
        (wal_metadata.len(), true)
    } else {
        (0, false)
    };

    // Try to open the file to verify it's readable
    let _file = File::open(backup_path)?;

    Ok(BackupInfo {
        db_size,
        wal_size,
        includes_wal,
    })
}

/// Copies a file from source to destination.
/// Returns the number of bytes copied.
fn copy_file(src: &Path, dest: &Path) -> io::Result<u64> {
    let src_file = File::open(src)?;
    let dest_file = File::create(dest)?;

    let mut reader = BufReader::with_capacity(COPY_BUFFER_SIZE, src_file);
    let mut writer = BufWriter::with_capacity(COPY_BUFFER_SIZE, dest_file);

    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut total_bytes = 0u64;

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        writer.write_all(&buffer[..bytes_read])?;
        total_bytes += bytes_read as u64;
    }

    writer.flush()?;

    // Sync to ensure durability
    writer.get_ref().sync_all()?;

    Ok(total_bytes)
}

/// Deletes a backup and its associated WAL file.
pub fn delete_backup(backup_path: &Path) -> io::Result<()> {
    if backup_path.exists() {
        fs::remove_file(backup_path)?;
    }

    let wal_path = WAL::wal_path(backup_path);
    if wal_path.exists() {
        fs::remove_file(wal_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_backup_and_restore() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let backup_path = dir.path().join("backup").join("test.db.bak");

        // Create a test database file
        let mut file = File::create(&db_path).unwrap();
        file.write_all(b"test database content").unwrap();
        file.sync_all().unwrap();

        // Create backup
        let info = backup(&db_path, &backup_path, false).unwrap();
        assert_eq!(info.db_size, 21);
        assert!(!info.includes_wal);

        // Verify backup
        let verify_info = verify_backup(&backup_path).unwrap();
        assert_eq!(verify_info.db_size, 21);

        // Restore to new location
        let restore_path = dir.path().join("restored.db");
        let restore_info = restore(&backup_path, &restore_path, false).unwrap();
        assert_eq!(restore_info.db_size, 21);

        // Verify content
        let mut content = String::new();
        File::open(&restore_path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "test database content");
    }

    #[test]
    fn test_backup_with_wal() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let backup_path = dir.path().join("test.db.bak");

        // Create database and WAL files
        File::create(&db_path)
            .unwrap()
            .write_all(b"database")
            .unwrap();

        let wal_path = WAL::wal_path(&db_path);
        File::create(&wal_path)
            .unwrap()
            .write_all(b"wal data")
            .unwrap();

        // Backup with WAL
        let info = backup(&db_path, &backup_path, true).unwrap();
        assert!(info.includes_wal);
        assert!(info.wal_size > 0);

        // Verify WAL was copied
        let backup_wal_path = WAL::wal_path(&backup_path);
        assert!(backup_wal_path.exists());
    }

    #[test]
    fn test_delete_backup() {
        let dir = tempdir().unwrap();
        let backup_path = dir.path().join("test.db.bak");

        // Create backup files
        File::create(&backup_path).unwrap();
        let wal_path = WAL::wal_path(&backup_path);
        File::create(&wal_path).unwrap();

        assert!(backup_path.exists());
        assert!(wal_path.exists());

        // Delete backup
        delete_backup(&backup_path).unwrap();

        assert!(!backup_path.exists());
        assert!(!wal_path.exists());
    }
}
