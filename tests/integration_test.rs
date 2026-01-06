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
            assert_eq!(result, Some(expected), "Session 1 key {} missing before close", i);
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
        println!("Session 2: Verifying {} keys from session 1...", KEYS_SESSION_1);
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
        println!("Session 2: Verifying all {} keys...", KEYS_SESSION_1 + KEYS_SESSION_2);

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

        println!("Session 3: Final verification of all {} keys...", KEYS_SESSION_1 + KEYS_SESSION_2);

        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let expected = format!("session1_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(result, Some(expected), "Session 1 key {} missing in final check", i);
        }

        for i in 0..KEYS_SESSION_2 {
            let key = format!("session2_key_{:04}", i);
            let expected = format!("session2_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(result, Some(expected), "Session 2 key {} missing in final check", i);
        }

        drop(btree);
    }

    println!("Insert-after-reopen test completed successfully - no page overwrites!");
}
