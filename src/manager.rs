//! Database Manager module for handling multiple database instances.
//!
//! Provides a `DatabaseManager` that can open, manage, and close
//! multiple named database instances in a single process.

use crate::btree::BTree;
use crate::pager::Pager;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

/// Configuration options for opening a database.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Path to the database file
    pub path: PathBuf,
    /// Whether to create the database if it doesn't exist
    pub create_if_missing: bool,
    /// Whether to open in read-only mode
    pub read_only: bool,
}

impl DatabaseConfig {
    /// Creates a new configuration with default settings.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        DatabaseConfig {
            path: path.into(),
            create_if_missing: true,
            read_only: false,
        }
    }

    /// Sets whether to create the database if it doesn't exist.
    pub fn create_if_missing(mut self, create: bool) -> Self {
        self.create_if_missing = create;
        self
    }

    /// Sets whether to open in read-only mode.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

/// A handle to an open database.
pub struct DatabaseHandle {
    /// The B-Tree database instance
    btree: BTree,
    /// Configuration used to open this database
    config: DatabaseConfig,
    /// Whether the database has been modified
    dirty: bool,
}

impl DatabaseHandle {
    /// Returns a reference to the B-Tree.
    pub fn btree(&self) -> &BTree {
        &self.btree
    }

    /// Returns a mutable reference to the B-Tree.
    pub fn btree_mut(&mut self) -> &mut BTree {
        self.dirty = true;
        &mut self.btree
    }

    /// Returns the configuration.
    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// Returns whether the database has been modified.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Syncs the database to disk.
    pub fn sync(&mut self) -> io::Result<()> {
        self.btree.sync()?;
        self.dirty = false;
        Ok(())
    }
}

/// Manages multiple database instances.
pub struct DatabaseManager {
    /// Map of database names to their handles
    databases: HashMap<String, DatabaseHandle>,
}

impl DatabaseManager {
    /// Creates a new database manager.
    pub fn new() -> Self {
        DatabaseManager {
            databases: HashMap::new(),
        }
    }

    /// Opens a database with the given name and configuration.
    /// Returns an error if a database with this name is already open.
    pub fn open(&mut self, name: &str, config: DatabaseConfig) -> io::Result<()> {
        if self.databases.contains_key(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Database '{}' is already open", name),
            ));
        }

        let file = open_database_file(&config)?;
        let pager = Pager::new(file);
        let btree = BTree::new(pager)?;

        self.databases.insert(
            name.to_string(),
            DatabaseHandle {
                btree,
                config,
                dirty: false,
            },
        );

        Ok(())
    }

    /// Opens a database with the given name and path using default configuration.
    pub fn open_path(&mut self, name: &str, path: impl Into<PathBuf>) -> io::Result<()> {
        self.open(name, DatabaseConfig::new(path))
    }

    /// Returns a reference to an open database.
    pub fn get(&self, name: &str) -> Option<&DatabaseHandle> {
        self.databases.get(name)
    }

    /// Returns a mutable reference to an open database.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut DatabaseHandle> {
        self.databases.get_mut(name)
    }

    /// Returns true if a database with the given name is open.
    pub fn is_open(&self, name: &str) -> bool {
        self.databases.contains_key(name)
    }

    /// Returns the number of open databases.
    pub fn count(&self) -> usize {
        self.databases.len()
    }

    /// Returns the names of all open databases.
    pub fn names(&self) -> Vec<&str> {
        self.databases.keys().map(|s| s.as_str()).collect()
    }

    /// Closes a database, syncing it to disk first.
    pub fn close(&mut self, name: &str) -> io::Result<()> {
        match self.databases.remove(name) {
            Some(mut handle) => {
                handle.sync()?;
                Ok(())
            }
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Database '{}' is not open", name),
            )),
        }
    }

    /// Syncs all open databases to disk.
    pub fn sync_all(&mut self) -> io::Result<()> {
        for handle in self.databases.values_mut() {
            handle.sync()?;
        }
        Ok(())
    }

    /// Closes all open databases, syncing them first.
    pub fn close_all(&mut self) -> io::Result<()> {
        self.sync_all()?;
        self.databases.clear();
        Ok(())
    }
}

impl Default for DatabaseManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DatabaseManager {
    fn drop(&mut self) {
        // Try to sync all databases on drop, but don't propagate errors
        let _ = self.sync_all();
    }
}

/// Opens a database file based on the configuration.
fn open_database_file(config: &DatabaseConfig) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);

    if config.read_only {
        if !config.path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Database file not found: {}", config.path.display()),
            ));
        }
    } else {
        options.write(true);
        if config.create_if_missing {
            options.create(true);
        }
    }

    options.open(&config.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_database_manager_open_close() {
        let dir = tempdir().unwrap();
        let mut manager = DatabaseManager::new();

        // Open a database
        let db_path = dir.path().join("test1.db");
        manager.open_path("test1", &db_path).unwrap();

        assert!(manager.is_open("test1"));
        assert_eq!(manager.count(), 1);

        // Close the database
        manager.close("test1").unwrap();
        assert!(!manager.is_open("test1"));
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_database_manager_multiple_databases() {
        let dir = tempdir().unwrap();
        let mut manager = DatabaseManager::new();

        // Open multiple databases
        manager.open_path("db1", dir.path().join("db1.db")).unwrap();
        manager.open_path("db2", dir.path().join("db2.db")).unwrap();
        manager.open_path("db3", dir.path().join("db3.db")).unwrap();

        assert_eq!(manager.count(), 3);

        // Access each database
        {
            let handle = manager.get_mut("db1").unwrap();
            handle.btree_mut().insert("key1", "value1").unwrap();
        }
        {
            let handle = manager.get_mut("db2").unwrap();
            handle.btree_mut().insert("key2", "value2").unwrap();
        }

        // Verify data is isolated
        {
            let handle = manager.get_mut("db1").unwrap();
            assert_eq!(
                handle.btree_mut().get("key1").unwrap(),
                Some("value1".to_string())
            );
            assert_eq!(handle.btree_mut().get("key2").unwrap(), None);
        }

        // Close all
        manager.close_all().unwrap();
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_database_manager_duplicate_open() {
        let dir = tempdir().unwrap();
        let mut manager = DatabaseManager::new();

        let db_path = dir.path().join("test.db");
        manager.open_path("test", &db_path).unwrap();

        // Try to open again with same name
        let result = manager.open_path("test", &db_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_database_config() {
        let config = DatabaseConfig::new("/path/to/db")
            .create_if_missing(false)
            .read_only(true);

        assert_eq!(config.path, PathBuf::from("/path/to/db"));
        assert!(!config.create_if_missing);
        assert!(config.read_only);
    }
}
