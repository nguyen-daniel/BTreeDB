//! Concurrency module for multi-threaded access.
//!
//! Provides lock management for concurrent database access using
//! a read-write lock pattern: multiple readers or single writer.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// A lock for a single page.
#[derive(Debug)]
pub struct PageLock {
    /// Number of active readers
    readers: AtomicU32,
    /// Whether a writer holds the lock
    writer: AtomicBool,
    /// Writer waiting flag for priority
    writer_waiting: AtomicBool,
}

impl PageLock {
    /// Creates a new unlocked page lock.
    pub fn new() -> Self {
        PageLock {
            readers: AtomicU32::new(0),
            writer: AtomicBool::new(false),
            writer_waiting: AtomicBool::new(false),
        }
    }

    /// Returns the number of active readers.
    pub fn reader_count(&self) -> u32 {
        self.readers.load(Ordering::SeqCst)
    }

    /// Returns true if a writer holds the lock.
    pub fn is_write_locked(&self) -> bool {
        self.writer.load(Ordering::SeqCst)
    }

    /// Returns true if the lock is completely free.
    pub fn is_free(&self) -> bool {
        !self.is_write_locked() && self.reader_count() == 0
    }
}

impl Default for PageLock {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of attempting to acquire a lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockResult {
    /// Lock was successfully acquired
    Acquired,
    /// Lock could not be acquired (would block)
    WouldBlock,
}

/// A guard that releases a read lock when dropped.
pub struct ReadGuard {
    lock: Arc<PageLock>,
    page_id: u32,
}

impl ReadGuard {
    /// Returns the page ID this guard is for.
    pub fn page_id(&self) -> u32 {
        self.page_id
    }
}

impl Drop for ReadGuard {
    fn drop(&mut self) {
        self.lock.readers.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A guard that releases a write lock when dropped.
pub struct WriteGuard {
    lock: Arc<PageLock>,
    page_id: u32,
}

impl WriteGuard {
    /// Returns the page ID this guard is for.
    pub fn page_id(&self) -> u32 {
        self.page_id
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        self.lock.writer.store(false, Ordering::SeqCst);
    }
}

/// Manages locks for all pages.
pub struct LockManager {
    /// Map of page IDs to their locks
    page_locks: RwLock<HashMap<u32, Arc<PageLock>>>,
    /// Global lock for database-wide operations
    global_lock: Mutex<()>,
}

impl LockManager {
    /// Creates a new lock manager.
    pub fn new() -> Self {
        LockManager {
            page_locks: RwLock::new(HashMap::new()),
            global_lock: Mutex::new(()),
        }
    }

    /// Gets or creates a lock for a page.
    fn get_or_create_lock(&self, page_id: u32) -> Arc<PageLock> {
        // Try to get existing lock with read lock
        {
            let locks = self.page_locks.read().unwrap();
            if let Some(lock) = locks.get(&page_id) {
                return Arc::clone(lock);
            }
        }

        // Create new lock with write lock
        let mut locks = self.page_locks.write().unwrap();
        locks
            .entry(page_id)
            .or_insert_with(|| Arc::new(PageLock::new()))
            .clone()
    }

    /// Attempts to acquire a read lock on a page (non-blocking).
    pub fn try_acquire_read(&self, page_id: u32) -> Result<ReadGuard, LockResult> {
        let lock = self.get_or_create_lock(page_id);

        // Check if a writer has the lock or is waiting
        if lock.writer.load(Ordering::SeqCst) || lock.writer_waiting.load(Ordering::SeqCst) {
            return Err(LockResult::WouldBlock);
        }

        // Increment reader count
        lock.readers.fetch_add(1, Ordering::SeqCst);

        // Double-check writer didn't acquire lock while we were incrementing
        if lock.writer.load(Ordering::SeqCst) {
            lock.readers.fetch_sub(1, Ordering::SeqCst);
            return Err(LockResult::WouldBlock);
        }

        Ok(ReadGuard { lock, page_id })
    }

    /// Acquires a read lock on a page (blocking with spin).
    pub fn acquire_read(&self, page_id: u32) -> ReadGuard {
        loop {
            match self.try_acquire_read(page_id) {
                Ok(guard) => return guard,
                Err(_) => {
                    std::hint::spin_loop();
                }
            }
        }
    }

    /// Attempts to acquire a write lock on a page (non-blocking).
    pub fn try_acquire_write(&self, page_id: u32) -> Result<WriteGuard, LockResult> {
        let lock = self.get_or_create_lock(page_id);

        // Set writer waiting flag
        lock.writer_waiting.store(true, Ordering::SeqCst);

        // Try to acquire writer lock
        let acquired = lock
            .writer
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();

        if !acquired {
            lock.writer_waiting.store(false, Ordering::SeqCst);
            return Err(LockResult::WouldBlock);
        }

        // Wait for readers to finish
        if lock.readers.load(Ordering::SeqCst) > 0 {
            lock.writer.store(false, Ordering::SeqCst);
            lock.writer_waiting.store(false, Ordering::SeqCst);
            return Err(LockResult::WouldBlock);
        }

        lock.writer_waiting.store(false, Ordering::SeqCst);
        Ok(WriteGuard { lock, page_id })
    }

    /// Acquires a write lock on a page (blocking with spin).
    pub fn acquire_write(&self, page_id: u32) -> WriteGuard {
        loop {
            match self.try_acquire_write(page_id) {
                Ok(guard) => return guard,
                Err(_) => {
                    std::hint::spin_loop();
                }
            }
        }
    }

    /// Returns the number of pages with active locks.
    pub fn active_lock_count(&self) -> usize {
        let locks = self.page_locks.read().unwrap();
        locks.values().filter(|l| !l.is_free()).count()
    }

    /// Acquires the global lock for database-wide operations.
    pub fn acquire_global(&self) -> std::sync::MutexGuard<'_, ()> {
        self.global_lock.lock().unwrap()
    }

    /// Cleans up unused locks (locks that are completely free).
    pub fn cleanup(&self) {
        let mut locks = self.page_locks.write().unwrap();
        locks.retain(|_, lock| !lock.is_free());
    }
}

impl Default for LockManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe reference counter for tracking active connections.
#[derive(Debug)]
pub struct ConnectionPool {
    /// Number of active connections
    active: AtomicU32,
    /// Maximum allowed connections
    max_connections: u32,
}

impl ConnectionPool {
    /// Creates a new connection pool.
    pub fn new(max_connections: u32) -> Self {
        ConnectionPool {
            active: AtomicU32::new(0),
            max_connections,
        }
    }

    /// Attempts to acquire a connection.
    pub fn try_acquire(&self) -> Option<ConnectionGuard<'_>> {
        let current = self.active.load(Ordering::SeqCst);
        if current >= self.max_connections {
            return None;
        }

        // Try to increment
        if self
            .active
            .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(ConnectionGuard { pool: self })
        } else {
            // Race condition, try again
            self.try_acquire()
        }
    }

    /// Returns the number of active connections.
    pub fn active_count(&self) -> u32 {
        self.active.load(Ordering::SeqCst)
    }

    /// Returns the maximum number of connections.
    pub fn max_connections(&self) -> u32 {
        self.max_connections
    }

    /// Returns true if the pool is at capacity.
    pub fn is_full(&self) -> bool {
        self.active_count() >= self.max_connections
    }
}

/// A guard that releases a connection when dropped.
pub struct ConnectionGuard<'a> {
    pool: &'a ConnectionPool,
}

impl<'a> Drop for ConnectionGuard<'a> {
    fn drop(&mut self) {
        self.pool.active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_page_lock_basic() {
        let lock = PageLock::new();
        assert!(lock.is_free());
        assert!(!lock.is_write_locked());
        assert_eq!(lock.reader_count(), 0);
    }

    #[test]
    fn test_lock_manager_read() {
        let manager = LockManager::new();

        // Acquire multiple read locks on same page
        let guard1 = manager.acquire_read(1);
        let guard2 = manager.acquire_read(1);

        assert_eq!(guard1.page_id(), 1);
        assert_eq!(guard2.page_id(), 1);

        drop(guard1);
        drop(guard2);
    }

    #[test]
    fn test_lock_manager_write() {
        let manager = LockManager::new();

        let guard = manager.acquire_write(1);
        assert_eq!(guard.page_id(), 1);

        // Try to acquire another write lock (should fail)
        assert!(manager.try_acquire_write(1).is_err());

        drop(guard);

        // Now we can acquire it
        assert!(manager.try_acquire_write(1).is_ok());
    }

    #[test]
    fn test_lock_manager_read_write_conflict() {
        let manager = LockManager::new();

        // Acquire read lock
        let read_guard = manager.acquire_read(1);

        // Try to acquire write lock (should fail)
        assert!(manager.try_acquire_write(1).is_err());

        drop(read_guard);

        // Now we can acquire write lock
        let write_guard = manager.acquire_write(1);

        // Try to acquire read lock (should fail)
        assert!(manager.try_acquire_read(1).is_err());

        drop(write_guard);
    }

    #[test]
    fn test_lock_manager_concurrent() {
        let manager = Arc::new(LockManager::new());
        let mut handles = vec![];

        // Spawn multiple reader threads
        for _ in 0..10 {
            let manager = Arc::clone(&manager);
            handles.push(thread::spawn(move || {
                for page_id in 0..5 {
                    let _guard = manager.acquire_read(page_id);
                    // Simulate some work
                    thread::yield_now();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_connection_pool() {
        let pool = ConnectionPool::new(3);

        assert_eq!(pool.active_count(), 0);
        assert!(!pool.is_full());

        let conn1 = pool.try_acquire().unwrap();
        let conn2 = pool.try_acquire().unwrap();
        let conn3 = pool.try_acquire().unwrap();

        assert_eq!(pool.active_count(), 3);
        assert!(pool.is_full());
        assert!(pool.try_acquire().is_none());

        drop(conn1);
        assert!(!pool.is_full());
        assert!(pool.try_acquire().is_some());

        drop(conn2);
        drop(conn3);
    }
}
