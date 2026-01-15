//! Transaction module for ACID transaction support.
//!
//! Provides transaction semantics with commit and rollback capabilities
//! using the Write-Ahead Log (WAL) for durability.

use std::io;

/// Transaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// Transaction is active and can accept operations
    Active,
    /// Transaction has been committed
    Committed,
    /// Transaction has been rolled back
    RolledBack,
}

/// A database transaction.
///
/// Transactions provide atomicity - either all operations succeed (commit)
/// or none of them take effect (rollback).
#[derive(Debug)]
pub struct Transaction {
    /// Unique transaction ID
    id: u64,
    /// Current state of the transaction
    state: TransactionState,
    /// WAL offset when transaction started (for rollback)
    wal_start_offset: u64,
    /// List of modified page IDs in this transaction
    modified_pages: Vec<u32>,
    /// Savepoints for nested transaction support
    savepoints: Vec<Savepoint>,
}

/// A savepoint within a transaction.
#[derive(Debug, Clone)]
pub struct Savepoint {
    /// Name of the savepoint
    pub name: String,
    /// WAL offset at savepoint creation
    pub wal_offset: u64,
    /// Number of modified pages at savepoint
    pub modified_count: usize,
}

impl Transaction {
    /// Creates a new transaction.
    pub fn new(id: u64, wal_start_offset: u64) -> Self {
        Transaction {
            id,
            state: TransactionState::Active,
            wal_start_offset,
            modified_pages: Vec::new(),
            savepoints: Vec::new(),
        }
    }

    /// Returns the transaction ID.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the current state of the transaction.
    pub fn state(&self) -> TransactionState {
        self.state
    }

    /// Returns true if the transaction is active.
    pub fn is_active(&self) -> bool {
        self.state == TransactionState::Active
    }

    /// Returns the WAL offset when the transaction started.
    pub fn wal_start_offset(&self) -> u64 {
        self.wal_start_offset
    }

    /// Records a page modification.
    pub fn record_modification(&mut self, page_id: u32) {
        if !self.modified_pages.contains(&page_id) {
            self.modified_pages.push(page_id);
        }
    }

    /// Returns the list of modified page IDs.
    pub fn modified_pages(&self) -> &[u32] {
        &self.modified_pages
    }

    /// Creates a savepoint.
    pub fn savepoint(&mut self, name: &str, current_wal_offset: u64) {
        self.savepoints.push(Savepoint {
            name: name.to_string(),
            wal_offset: current_wal_offset,
            modified_count: self.modified_pages.len(),
        });
    }

    /// Rolls back to a savepoint.
    /// Returns the WAL offset to truncate to, or None if savepoint not found.
    pub fn rollback_to_savepoint(&mut self, name: &str) -> Option<u64> {
        if let Some(pos) = self.savepoints.iter().position(|s| s.name == name) {
            let savepoint = self.savepoints[pos].clone();

            // Remove savepoints after this one
            self.savepoints.truncate(pos + 1);

            // Remove modifications after this savepoint
            self.modified_pages.truncate(savepoint.modified_count);

            Some(savepoint.wal_offset)
        } else {
            None
        }
    }

    /// Releases a savepoint (removes it without rolling back).
    pub fn release_savepoint(&mut self, name: &str) -> bool {
        if let Some(pos) = self.savepoints.iter().position(|s| s.name == name) {
            self.savepoints.remove(pos);
            true
        } else {
            false
        }
    }

    /// Marks the transaction as committed.
    pub fn commit(&mut self) -> io::Result<()> {
        if self.state != TransactionState::Active {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Cannot commit transaction in state {:?}", self.state),
            ));
        }
        self.state = TransactionState::Committed;
        Ok(())
    }

    /// Marks the transaction as rolled back.
    pub fn rollback(&mut self) -> io::Result<()> {
        if self.state != TransactionState::Active {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Cannot rollback transaction in state {:?}", self.state),
            ));
        }
        self.state = TransactionState::RolledBack;
        Ok(())
    }
}

/// Transaction manager for coordinating transactions.
pub struct TransactionManager {
    /// Counter for generating unique transaction IDs
    next_txn_id: u64,
    /// Currently active transaction (if any)
    active_transaction: Option<Transaction>,
}

impl TransactionManager {
    /// Creates a new transaction manager.
    pub fn new() -> Self {
        TransactionManager {
            next_txn_id: 1,
            active_transaction: None,
        }
    }

    /// Returns true if there is an active transaction.
    pub fn has_active_transaction(&self) -> bool {
        self.active_transaction.is_some()
    }

    /// Returns a reference to the active transaction if any.
    pub fn active_transaction(&self) -> Option<&Transaction> {
        self.active_transaction.as_ref()
    }

    /// Returns a mutable reference to the active transaction if any.
    pub fn active_transaction_mut(&mut self) -> Option<&mut Transaction> {
        self.active_transaction.as_mut()
    }

    /// Begins a new transaction.
    pub fn begin(&mut self, wal_offset: u64) -> io::Result<u64> {
        if self.active_transaction.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot begin transaction: another transaction is active",
            ));
        }

        let txn_id = self.next_txn_id;
        self.next_txn_id += 1;

        self.active_transaction = Some(Transaction::new(txn_id, wal_offset));

        Ok(txn_id)
    }

    /// Commits the active transaction.
    pub fn commit(&mut self) -> io::Result<Transaction> {
        match self.active_transaction.take() {
            Some(mut txn) => {
                txn.commit()?;
                Ok(txn)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No active transaction to commit",
            )),
        }
    }

    /// Rolls back the active transaction.
    pub fn rollback(&mut self) -> io::Result<Transaction> {
        match self.active_transaction.take() {
            Some(mut txn) => {
                txn.rollback()?;
                Ok(txn)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No active transaction to rollback",
            )),
        }
    }

    /// Records a page modification in the active transaction.
    pub fn record_modification(&mut self, page_id: u32) {
        if let Some(txn) = &mut self.active_transaction {
            txn.record_modification(page_id);
        }
    }

    /// Creates a savepoint in the active transaction.
    pub fn savepoint(&mut self, name: &str, wal_offset: u64) -> io::Result<()> {
        match &mut self.active_transaction {
            Some(txn) => {
                txn.savepoint(name, wal_offset);
                Ok(())
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No active transaction for savepoint",
            )),
        }
    }

    /// Rolls back to a savepoint in the active transaction.
    pub fn rollback_to_savepoint(&mut self, name: &str) -> io::Result<u64> {
        match &mut self.active_transaction {
            Some(txn) => match txn.rollback_to_savepoint(name) {
                Some(offset) => Ok(offset),
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Savepoint '{}' not found", name),
                )),
            },
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No active transaction for savepoint rollback",
            )),
        }
    }
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_lifecycle() {
        let mut txn = Transaction::new(1, 100);

        assert!(txn.is_active());
        assert_eq!(txn.id(), 1);
        assert_eq!(txn.wal_start_offset(), 100);

        txn.record_modification(10);
        txn.record_modification(20);
        txn.record_modification(10); // Duplicate

        assert_eq!(txn.modified_pages(), &[10, 20]);

        txn.commit().unwrap();
        assert_eq!(txn.state(), TransactionState::Committed);
    }

    #[test]
    fn test_transaction_rollback() {
        let mut txn = Transaction::new(2, 200);

        txn.record_modification(30);
        txn.rollback().unwrap();

        assert_eq!(txn.state(), TransactionState::RolledBack);
    }

    #[test]
    fn test_savepoints() {
        let mut txn = Transaction::new(3, 300);

        txn.record_modification(1);
        txn.record_modification(2);
        txn.savepoint("sp1", 400);

        txn.record_modification(3);
        txn.record_modification(4);
        txn.savepoint("sp2", 500);

        txn.record_modification(5);

        assert_eq!(txn.modified_pages().len(), 5);

        // Rollback to sp2
        let offset = txn.rollback_to_savepoint("sp2").unwrap();
        assert_eq!(offset, 500);
        assert_eq!(txn.modified_pages().len(), 4);

        // Rollback to sp1
        let offset = txn.rollback_to_savepoint("sp1").unwrap();
        assert_eq!(offset, 400);
        assert_eq!(txn.modified_pages().len(), 2);
    }

    #[test]
    fn test_transaction_manager() {
        let mut mgr = TransactionManager::new();

        assert!(!mgr.has_active_transaction());

        let txn_id = mgr.begin(100).unwrap();
        assert_eq!(txn_id, 1);
        assert!(mgr.has_active_transaction());

        mgr.record_modification(10);

        // Cannot begin another transaction
        assert!(mgr.begin(200).is_err());

        let txn = mgr.commit().unwrap();
        assert_eq!(txn.state(), TransactionState::Committed);
        assert!(!mgr.has_active_transaction());

        // Can begin a new transaction now
        let txn_id = mgr.begin(300).unwrap();
        assert_eq!(txn_id, 2);
    }
}
