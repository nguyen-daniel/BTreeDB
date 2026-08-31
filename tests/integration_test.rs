use btreedb::btree::BTree;
use btreedb::pager::Pager;
use std::fs::OpenOptions;

/// Creates a temporary database file for testing.
/// Returns a tuple of (File, TempPath) where TempPath ensures cleanup.
fn create_temp_db() -> (std::fs::File, tempfile::TempPath) {
    let temp_file = tempfile::NamedTempFile::new().expect("Failed to create temp file");
    temp_file.into_parts()
}

/// Opens an existing database file for testing.
fn open_db_file(path: &std::path::Path) -> std::fs::File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("Failed to open database file")
}

fn assert_no_empty_non_root_leaves(btree: &mut BTree) {
    let root = btree.root_page_id();
    let leaves = btree.leaf_occupancies().expect("leaf occupancies");
    assert!(!leaves.is_empty(), "tree must have at least one leaf");
    for (page_id, n) in leaves {
        if page_id != root {
            assert!(n > 0, "empty non-root leaf at page {page_id}");
        }
    }
}

#[test]
fn test_large_scale_insertion() {
    // Create a temporary database file
    let (file, _temp_path) = create_temp_db();

    // Initialize a new database
    let pager = Pager::new(file);
    let mut btree = BTree::new(pager).expect("Failed to create BTree");

    // Perform large-scale insertion (1000 keys) to trigger multiple B-Tree node splits
    // With MAX_LEAF_KEYS = 3, we expect many leaf nodes, which will trigger
    // multiple splits and potentially create internal nodes and root splits
    const NUM_KEYS: usize = 1000;

    println!("Inserting {} keys...", NUM_KEYS);
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let value = format!("value_{}", i);
        btree
            .insert(&key, &value)
            .expect(&format!("Failed to insert key {}", i));
    }

    // Verify all keys can be retrieved
    println!("Verifying all {} keys...", NUM_KEYS);
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let expected_value = format!("value_{}", i);
        match btree.get(&key).expect("Failed to get key") {
            Some(value) => assert_eq!(value, expected_value, "Value mismatch for key {}", key),
            None => panic!("Key {} not found", key),
        }
    }

    // Sync all data to disk before closing
    btree.sync().expect("Failed to sync database");

    // Drop the BTree to close the file
    drop(btree);

    // The temp file will be automatically cleaned up when temp_path is dropped
    println!("Test completed successfully");
}

#[test]
fn test_persistence_across_sessions() {
    // Create a temporary database file
    let (file, temp_path) = create_temp_db();
    let db_path = temp_path.to_path_buf();

    // First session: Initialize database and insert data
    {
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to create BTree");

        // Store the initial root page ID for verification
        let initial_root_id = btree.root_page_id();
        println!("Initial root page ID: {}", initial_root_id);

        // Insert some test data
        const NUM_KEYS: usize = 100;
        for i in 0..NUM_KEYS {
            let key = format!("persist_key_{:04}", i);
            let value = format!("persist_value_{}", i);
            btree
                .insert(&key, &value)
                .expect(&format!("Failed to insert key {}", i));
        }

        // Sync and close
        btree.sync().expect("Failed to sync database");
        drop(btree);
    }

    // Second session: Re-open the database file and verify persistence
    {
        let file = open_db_file(&db_path);
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to re-open BTree");

        // Persistence Check: Verify that the root page ID is correctly reloaded from disk
        // When we re-open the database, BTree::new() reads the header from page 0,
        // which contains the root_page_id. This test verifies that:
        // 1. The header was correctly written to disk in the first session
        // 2. The header is correctly read from disk in the second session
        // 3. The root_page_id stored in the header matches the actual root of the tree
        // 4. All data inserted in the first session is accessible in the second session
        let reloaded_root_id = btree.root_page_id();
        println!("Reloaded root page ID: {}", reloaded_root_id);

        // Verify all previously inserted keys are still accessible
        const NUM_KEYS: usize = 100;
        for i in 0..NUM_KEYS {
            let key = format!("persist_key_{:04}", i);
            let expected_value = format!("persist_value_{}", i);
            match btree.get(&key).expect("Failed to get key") {
                Some(value) => assert_eq!(
                    value, expected_value,
                    "Value mismatch for key {} after persistence",
                    key
                ),
                None => panic!("Key {} not found after persistence", key),
            }
        }

        // Insert additional data in the second session
        btree
            .insert("new_key", "new_value")
            .expect("Failed to insert new key");

        // Verify the new key is accessible
        match btree.get("new_key").expect("Failed to get new key") {
            Some(value) => assert_eq!(value, "new_value"),
            None => panic!("New key not found"),
        }

        // Sync and close
        btree.sync().expect("Failed to sync database");
        drop(btree);
    }

    // Third session: Verify data from both sessions persists
    {
        let file = open_db_file(&db_path);
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to re-open BTree again");

        // Verify data from first session
        const NUM_KEYS: usize = 100;
        for i in 0..NUM_KEYS {
            let key = format!("persist_key_{:04}", i);
            let expected_value = format!("persist_value_{}", i);
            match btree.get(&key).expect("Failed to get key") {
                Some(value) => assert_eq!(value, expected_value),
                None => panic!("Key {} not found in third session", key),
            }
        }

        // Verify data from second session
        match btree.get("new_key").expect("Failed to get new key") {
            Some(value) => assert_eq!(value, "new_value"),
            None => panic!("New key not found in third session"),
        }

        drop(btree);
    }

    // The temp file will be automatically cleaned up when temp_path is dropped
    println!("Persistence test completed successfully");
}

#[test]
fn test_root_splitting_persistence() {
    // This test specifically verifies that root splits are correctly persisted
    // When a root leaf node splits, a new internal root is created and the
    // header must be updated with the new root page ID

    let (file, temp_path) = create_temp_db();
    let db_path = temp_path.to_path_buf();

    // Insert enough keys to force root splitting
    // With MAX_LEAF_KEYS = 3, inserting 4+ keys will cause the root to split
    {
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to create BTree");

        let initial_root = btree.root_page_id();
        println!("Initial root before splits: {}", initial_root);

        // Insert keys to trigger root split
        // We need more than 3 keys to trigger a split, and then more to potentially
        // cause the new internal root to also need updating
        for i in 0..50 {
            let key = format!("split_key_{:04}", i);
            let value = format!("split_value_{}", i);
            btree
                .insert(&key, &value)
                .expect(&format!("Failed to insert key {}", i));
        }

        let final_root = btree.root_page_id();
        println!("Final root after splits: {}", final_root);

        // If root split occurred, the root page ID should have changed
        // (unless it split and then we happened to get the same page ID, which is unlikely)

        // Verify all keys are accessible
        for i in 0..50 {
            let key = format!("split_key_{:04}", i);
            let expected_value = format!("split_value_{}", i);
            match btree.get(&key).expect("Failed to get key") {
                Some(value) => assert_eq!(value, expected_value),
                None => panic!("Key {} not found", key),
            }
        }

        btree.sync().expect("Failed to sync database");
        drop(btree);
    }

    // Re-open and verify the root was correctly persisted
    {
        let file = open_db_file(&db_path);
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to re-open BTree");

        // Persistence Check: The root page ID should be correctly reloaded from the header
        // This verifies that when the root split occurred, the header was updated
        // with the new root page ID, and that this new root ID is correctly read
        // when the database is re-opened
        let reloaded_root = btree.root_page_id();
        println!("Reloaded root: {}", reloaded_root);

        // Verify all keys are still accessible after persistence
        for i in 0..50 {
            let key = format!("split_key_{:04}", i);
            let expected_value = format!("split_value_{}", i);
            match btree.get(&key).expect("Failed to get key") {
                Some(value) => assert_eq!(value, expected_value),
                None => panic!("Key {} not found after root split persistence", key),
            }
        }

        drop(btree);
    }

    println!("Root splitting persistence test completed successfully");
}

#[test]
fn test_inserts_after_reopen_no_page_overwrite() {
    // This test verifies the fix for the next_page_id bug.
    // Previously, next_page_id was estimated as root_page_id + 1 on reopen,
    // which could cause page overwrites when the tree had grown beyond the root.
    // Now, next_page_id is derived from the actual file size.

    let (file, temp_path) = create_temp_db();
    let db_path = temp_path.to_path_buf();

    const KEYS_SESSION_1: usize = 500;
    const KEYS_SESSION_2: usize = 500;

    // First session: Insert many keys to create multiple pages/splits
    {
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to create BTree");

        println!("Session 1: Inserting {} keys...", KEYS_SESSION_1);
        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let value = format!("session1_value_{}", i);
            btree
                .insert(&key, &value)
                .unwrap_or_else(|_| panic!("Failed to insert key {}", i));
        }

        // Verify all session 1 keys are present
        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let expected = format!("session1_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(
                result,
                Some(expected),
                "Session 1 key {} missing before close",
                i
            );
        }

        btree.sync().expect("Failed to sync");
        drop(btree);
    }

    // Second session: Reopen and insert MORE keys
    // This is where the bug would manifest - new pages would overwrite existing ones
    {
        let file = open_db_file(&db_path);
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to re-open BTree");

        // First, verify session 1 keys are still accessible
        println!(
            "Session 2: Verifying {} keys from session 1...",
            KEYS_SESSION_1
        );
        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let expected = format!("session1_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(
                result,
                Some(expected),
                "Session 1 key {} missing after reopen (before session 2 inserts)",
                i
            );
        }

        // Now insert more keys - this should NOT overwrite session 1 data
        println!("Session 2: Inserting {} NEW keys...", KEYS_SESSION_2);
        for i in 0..KEYS_SESSION_2 {
            let key = format!("session2_key_{:04}", i);
            let value = format!("session2_value_{}", i);
            btree
                .insert(&key, &value)
                .unwrap_or_else(|_| panic!("Failed to insert session 2 key {}", i));
        }

        // Verify ALL keys (from both sessions) are present
        println!(
            "Session 2: Verifying all {} keys...",
            KEYS_SESSION_1 + KEYS_SESSION_2
        );

        // Check session 1 keys are STILL present (this would fail with the old bug)
        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let expected = format!("session1_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(
                result,
                Some(expected),
                "Session 1 key {} was OVERWRITTEN by session 2 inserts! next_page_id bug!",
                i
            );
        }

        // Check session 2 keys are present
        for i in 0..KEYS_SESSION_2 {
            let key = format!("session2_key_{:04}", i);
            let expected = format!("session2_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(result, Some(expected), "Session 2 key {} missing", i);
        }

        btree.sync().expect("Failed to sync");
        drop(btree);
    }

    // Third session: Final verification that everything persisted correctly
    {
        let file = open_db_file(&db_path);
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to re-open BTree for final check");

        println!(
            "Session 3: Final verification of all {} keys...",
            KEYS_SESSION_1 + KEYS_SESSION_2
        );

        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let expected = format!("session1_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(
                result,
                Some(expected),
                "Session 1 key {} missing in final check",
                i
            );
        }

        for i in 0..KEYS_SESSION_2 {
            let key = format!("session2_key_{:04}", i);
            let expected = format!("session2_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(
                result,
                Some(expected),
                "Session 2 key {} missing in final check",
                i
            );
        }

        drop(btree);
    }

    println!("Insert-after-reopen test completed successfully - no page overwrites!");
}

#[test]
fn test_delete_single_key() {
    let (file, _temp_path) = create_temp_db();
    let pager = Pager::new(file);
    let mut btree = BTree::new(pager).expect("Failed to create BTree");

    // Insert a key
    btree.insert("key1", "value1").expect("Failed to insert");

    // Verify it exists
    assert_eq!(btree.get("key1").unwrap(), Some("value1".to_string()));

    // Delete it
    let deleted = btree.delete("key1").expect("Failed to delete");
    assert!(deleted, "Key should have been deleted");

    // Verify it's gone
    assert_eq!(btree.get("key1").unwrap(), None);

    // Try to delete again - should return false
    let deleted_again = btree.delete("key1").expect("Failed to delete again");
    assert!(!deleted_again, "Key should not exist to delete");

    println!("Single key deletion test completed successfully");
}

#[test]
fn test_delete_multiple_keys() {
    let (file, _temp_path) = create_temp_db();
    let pager = Pager::new(file);
    let mut btree = BTree::new(pager).expect("Failed to create BTree");

    // Insert multiple keys
    const NUM_KEYS: usize = 20;
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let value = format!("value_{}", i);
        btree.insert(&key, &value).expect("Failed to insert");
    }

    // Delete every other key
    for i in (0..NUM_KEYS).step_by(2) {
        let key = format!("key_{:04}", i);
        let deleted = btree.delete(&key).expect("Failed to delete");
        assert!(deleted, "Key {} should have been deleted", key);
    }

    // Verify deleted keys are gone and remaining keys still exist
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let result = btree.get(&key).expect("Failed to get");
        if i % 2 == 0 {
            assert_eq!(result, None, "Deleted key {} should be gone", key);
        } else {
            let expected = format!("value_{}", i);
            assert_eq!(result, Some(expected), "Key {} should still exist", key);
        }
    }

    println!("Multiple key deletion test completed successfully");
}

#[test]
fn test_delete_all_keys() {
    let (file, _temp_path) = create_temp_db();
    let pager = Pager::new(file);
    let mut btree = BTree::new(pager).expect("Failed to create BTree");

    // Insert keys
    const NUM_KEYS: usize = 50;
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let value = format!("value_{}", i);
        btree.insert(&key, &value).expect("Failed to insert");
    }

    // Delete all keys
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let deleted = btree.delete(&key).expect("Failed to delete");
        assert!(deleted, "Key {} should have been deleted", key);
    }

    // Verify all keys are gone
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let result = btree.get(&key).expect("Failed to get");
        assert_eq!(result, None, "Key {} should be gone", key);
    }

    // Insert new keys after deletion
    for i in 0..10 {
        let key = format!("new_key_{}", i);
        let value = format!("new_value_{}", i);
        btree
            .insert(&key, &value)
            .expect("Failed to insert new key");
    }

    // Verify new keys exist
    for i in 0..10 {
        let key = format!("new_key_{}", i);
        let expected = format!("new_value_{}", i);
        let result = btree.get(&key).expect("Failed to get new key");
        assert_eq!(result, Some(expected), "New key {} should exist", key);
    }

    println!("Delete all keys test completed successfully");
}

#[test]
fn test_delete_persistence() {
    let (file, temp_path) = create_temp_db();
    let db_path = temp_path.to_path_buf();

    // First session: Insert and delete some keys
    {
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to create BTree");

        // Insert 20 keys
        for i in 0..20 {
            let key = format!("key_{:04}", i);
            let value = format!("value_{}", i);
            btree.insert(&key, &value).expect("Failed to insert");
        }

        // Delete keys 0-9
        for i in 0..10 {
            let key = format!("key_{:04}", i);
            btree.delete(&key).expect("Failed to delete");
        }

        btree.sync().expect("Failed to sync");
        drop(btree);
    }

    // Second session: Verify deletions persisted
    {
        let file = open_db_file(&db_path);
        let pager = Pager::new(file);
        let mut btree = BTree::new(pager).expect("Failed to re-open BTree");

        // Keys 0-9 should be gone
        for i in 0..10 {
            let key = format!("key_{:04}", i);
            let result = btree.get(&key).expect("Failed to get");
            assert_eq!(
                result, None,
                "Deleted key {} should persist as deleted",
                key
            );
        }

        // Keys 10-19 should still exist
        for i in 10..20 {
            let key = format!("key_{:04}", i);
            let expected = format!("value_{}", i);
            let result = btree.get(&key).expect("Failed to get");
            assert_eq!(result, Some(expected), "Key {} should persist", key);
        }

        drop(btree);
    }

    println!("Delete persistence test completed successfully");
}

#[test]
fn test_delete_and_reinsert() {
    let (file, _temp_path) = create_temp_db();
    let pager = Pager::new(file);
    let mut btree = BTree::new(pager).expect("Failed to create BTree");

    // Insert a key
    btree
        .insert("key1", "original_value")
        .expect("Failed to insert");
    assert_eq!(
        btree.get("key1").unwrap(),
        Some("original_value".to_string())
    );

    // Delete the key
    btree.delete("key1").expect("Failed to delete");
    assert_eq!(btree.get("key1").unwrap(), None);

    // Reinsert with different value
    btree
        .insert("key1", "new_value")
        .expect("Failed to reinsert");
    assert_eq!(btree.get("key1").unwrap(), Some("new_value".to_string()));

    println!("Delete and reinsert test completed successfully");
}

#[test]
fn test_1000_keys_persistence_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("keys1000.db");

    {
        let mut btree = BTree::open(&db_path).expect("open");
        for i in 0..1000 {
            btree
                .insert(&format!("key_{:04}", i), &format!("value_{}", i))
                .expect("insert");
        }
        let stats = btree.stats().expect("stats");
        assert_eq!(stats.key_count, 1000);
        assert!(
            stats.tree_height >= 2,
            "expected node splits, height={}",
            stats.tree_height
        );
        assert!(
            stats.leaf_count > 1,
            "expected multiple leaves after splits"
        );
        btree.sync().expect("sync");
    }

    {
        let mut btree = BTree::open(&db_path).expect("reopen");
        let stats = btree.stats().expect("stats");
        assert_eq!(stats.key_count, 1000);
        assert!(stats.tree_height >= 2);
        for i in 0..1000 {
            let key = format!("key_{:04}", i);
            assert_eq!(
                btree.get(&key).expect("get"),
                Some(format!("value_{}", i)),
                "missing {key} after reopen"
            );
        }
    }
}

#[test]
fn test_range_scan_after_splits() {
    use btreedb::cursor::Cursor;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("scan.db");
    let mut btree = BTree::open(&db_path).expect("open");
    for i in 0..80 {
        btree
            .insert(&format!("k{:03}", i), &format!("v{i}"))
            .expect("insert");
    }
    let rows = Cursor::scan_range(&mut btree, Some("k010"), Some("k020")).expect("scan");
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[0].0, "k010");
    assert_eq!(rows[9].0, "k019");
    let all = Cursor::scan_range(&mut btree, None, None).expect("scan all");
    assert_eq!(all.len(), 80);
}

#[test]
fn test_wal_recovers_zeroed_page() {
    use btreedb::pager::{Pager, PAGE_SIZE};
    use btreedb::wal::WAL;
    use std::fs::OpenOptions;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("crash.db");

    {
        let mut btree = BTree::open(&db_path).expect("open");
        btree.insert("alpha", "one").expect("insert");
        btree.sync().expect("sync");
    }

    let original = {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&db_path)
            .expect("open db");
        let mut pager = Pager::new(file);
        pager.get_page(1).expect("read leaf")
    };

    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&db_path)
            .expect("open db");
        let mut pager = Pager::new(file);
        pager
            .write_page(1, &[0u8; PAGE_SIZE])
            .expect("zero leaf (no WAL)");
    }

    {
        let mut wal = WAL::open(&db_path).expect("wal");
        wal.log_page(1, &original).expect("log original leaf");
    }

    let mut btree = BTree::open(&db_path).expect("recover");
    assert_eq!(
        btree.get("alpha").expect("get"),
        Some("one".to_string()),
        "WAL replay should restore the leaf"
    );
}

#[test]
fn test_wrong_magic_fails_open() {
    use btreedb::pager::PAGE_SIZE;
    use std::io::ErrorKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bad_magic.db");

    let mut page = vec![0u8; PAGE_SIZE];
    page[..7].copy_from_slice(b"NOTBTRE");
    std::fs::write(&db_path, &page).expect("write corrupt file");

    let err = match BTree::open(&db_path) {
        Ok(_) => panic!("corrupt header must fail open"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("Invalid magic bytes"),
        "error={err}"
    );

    let remaining = std::fs::read(&db_path).expect("read");
    assert_eq!(
        &remaining[..7],
        b"NOTBTRE",
        "must not wipe a corrupt file into a new database"
    );
}

/// Insert enough keys to split, delete until a leaf merge fires, reopen,
/// then check get/scan and that no non-root leaf is empty.
#[test]
fn test_delete_merge_reopen() {
    use btreedb::cursor::Cursor;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("merge.db");

    // MAX_LEAF_KEYS = 3: 24 sequential keys force multiple leaf splits and
    // an internal-root split (height >= 2, usually 3).
    const INSERT: usize = 24;
    const DELETE: usize = 16;

    let leaf_after = {
        let mut btree = BTree::open(&db_path).expect("open");
        for i in 0..INSERT {
            btree
                .insert(&format!("key_{:04}", i), &format!("value_{}", i))
                .expect("insert");
        }
        let before = btree.stats().expect("stats");
        assert!(
            before.leaf_count > 1,
            "expected splits, leaf_count={}",
            before.leaf_count
        );
        assert!(
            before.tree_height >= 2,
            "expected height >= 2 after splits, height={}",
            before.tree_height
        );

        for i in 0..DELETE {
            let key = format!("key_{:04}", i);
            assert!(btree.delete(&key).expect("delete"), "delete {key}");
        }

        let after = btree.stats().expect("stats");
        assert!(
            after.leaf_count < before.leaf_count,
            "expected a leaf merge: before={} after={}",
            before.leaf_count,
            after.leaf_count
        );
        assert_eq!(after.key_count, (INSERT - DELETE) as u64);

        for i in DELETE..INSERT {
            let key = format!("key_{:04}", i);
            assert_eq!(
                btree.get(&key).expect("get"),
                Some(format!("value_{}", i)),
                "missing {key} after merge"
            );
        }
        for i in 0..DELETE {
            assert_eq!(
                btree.get(&format!("key_{:04}", i)).expect("get"),
                None,
                "deleted key still present"
            );
        }

        let scan = Cursor::scan_range(&mut btree, None, None).expect("scan");
        assert_eq!(scan.len(), INSERT - DELETE);
        for (i, (k, v)) in scan.iter().enumerate() {
            let idx = DELETE + i;
            assert_eq!(k, &format!("key_{:04}", idx));
            assert_eq!(v, &format!("value_{}", idx));
        }

        assert_no_empty_non_root_leaves(&mut btree);
        btree.sync().expect("sync");
        after.leaf_count
    };

    {
        let mut btree = BTree::open(&db_path).expect("reopen");
        let stats = btree.stats().expect("stats");
        assert_eq!(stats.key_count, (INSERT - DELETE) as u64);
        assert_eq!(stats.leaf_count, leaf_after);
        for i in DELETE..INSERT {
            assert_eq!(
                btree.get(&format!("key_{:04}", i)).expect("get"),
                Some(format!("value_{}", i)),
                "missing after reopen"
            );
        }
        let scan = Cursor::scan_range(&mut btree, None, None).expect("scan");
        assert_eq!(scan.len(), INSERT - DELETE);
        assert_no_empty_non_root_leaves(&mut btree);
    }
}

/// Height-3 tree: delete until internal nodes merge or the height drops.
#[test]
fn test_delete_internal_merge() {
    use btreedb::cursor::Cursor;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("internal_merge.db");

    const INSERT: usize = 80;
    const DELETE: usize = 70;

    {
        let mut btree = BTree::open(&db_path).expect("open");
        for i in 0..INSERT {
            btree
                .insert(&format!("key_{:04}", i), &format!("value_{}", i))
                .expect("insert");
        }
        let before = btree.stats().expect("stats");
        assert!(
            before.tree_height >= 3,
            "expected height >= 3 so internal nodes exist, height={}",
            before.tree_height
        );
        assert!(
            before.internal_count >= 2,
            "need sibling internals to merge"
        );

        for i in 0..DELETE {
            let key = format!("key_{:04}", i);
            assert!(btree.delete(&key).expect("delete"), "delete {key}");
        }

        let after = btree.stats().expect("stats");
        assert_eq!(after.key_count, (INSERT - DELETE) as u64);
        assert!(
            after.internal_count < before.internal_count || after.tree_height < before.tree_height,
            "expected internal merge or height drop: internals {}→{}, height {}→{}",
            before.internal_count,
            after.internal_count,
            before.tree_height,
            after.tree_height
        );

        for i in DELETE..INSERT {
            assert_eq!(
                btree.get(&format!("key_{:04}", i)).expect("get"),
                Some(format!("value_{}", i))
            );
        }
        let scan = Cursor::scan_range(&mut btree, None, None).expect("scan");
        assert_eq!(scan.len(), INSERT - DELETE);
        assert_no_empty_non_root_leaves(&mut btree);
        btree.sync().expect("sync");
    }

    {
        let mut btree = BTree::open(&db_path).expect("reopen");
        assert_eq!(
            btree.stats().expect("stats").key_count,
            (INSERT - DELETE) as u64
        );
        for i in DELETE..INSERT {
            assert_eq!(
                btree.get(&format!("key_{:04}", i)).expect("get"),
                Some(format!("value_{}", i)),
                "missing after reopen"
            );
        }
        let scan = Cursor::scan_range(&mut btree, None, None).expect("scan");
        assert_eq!(scan.len(), INSERT - DELETE);
        assert_no_empty_non_root_leaves(&mut btree);
    }
}
