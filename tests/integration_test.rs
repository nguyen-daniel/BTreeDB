use btreedb::btree::BTree;
use std::path::{Path, PathBuf};

/// Temporary directory plus database path. Keep the `TempDir` alive for cleanup.
fn create_temp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    let path = dir.path().join("test.db");
    (dir, path)
}

fn open_btree(path: &Path) -> BTree {
    BTree::open(path).expect("Failed to open BTree")
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
    let (_dir, db_path) = create_temp_db();
    let mut btree = open_btree(&db_path);

    // Leaves pack to a 4KB byte budget; 1000 short keys still split.
    const NUM_KEYS: usize = 1000;

    println!("Inserting {} keys...", NUM_KEYS);
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let value = format!("value_{}", i);
        btree
            .insert(&key, &value)
            .unwrap_or_else(|_| panic!("Failed to insert key {i}"));
    }

    println!("Verifying all {} keys...", NUM_KEYS);
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let expected_value = format!("value_{}", i);
        match btree.get(&key).expect("Failed to get key") {
            Some(value) => assert_eq!(value, expected_value, "Value mismatch for key {}", key),
            None => panic!("Key {} not found", key),
        }
    }

    btree.sync().expect("Failed to sync database");
    drop(btree);
    println!("Test completed successfully");
}

#[test]
fn test_persistence_across_sessions() {
    let (_dir, db_path) = create_temp_db();

    {
        let mut btree = open_btree(&db_path);
        let initial_root_id = btree.root_page_id();
        println!("Initial root page ID: {}", initial_root_id);

        const NUM_KEYS: usize = 100;
        for i in 0..NUM_KEYS {
            let key = format!("persist_key_{:04}", i);
            let value = format!("persist_value_{}", i);
            btree
                .insert(&key, &value)
                .unwrap_or_else(|_| panic!("Failed to insert key {i}"));
        }

        btree.sync().expect("Failed to sync database");
        drop(btree);
    }

    {
        let mut btree = open_btree(&db_path);
        let reloaded_root_id = btree.root_page_id();
        println!("Reloaded root page ID: {}", reloaded_root_id);

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

        btree
            .insert("new_key", "new_value")
            .expect("Failed to insert new key");
        match btree.get("new_key").expect("Failed to get new key") {
            Some(value) => assert_eq!(value, "new_value"),
            None => panic!("New key not found"),
        }

        btree.sync().expect("Failed to sync database");
        drop(btree);
    }

    {
        let mut btree = open_btree(&db_path);

        const NUM_KEYS: usize = 100;
        for i in 0..NUM_KEYS {
            let key = format!("persist_key_{:04}", i);
            let expected_value = format!("persist_value_{}", i);
            match btree.get(&key).expect("Failed to get key") {
                Some(value) => assert_eq!(value, expected_value),
                None => panic!("Key {} not found in third session", key),
            }
        }

        match btree.get("new_key").expect("Failed to get new key") {
            Some(value) => assert_eq!(value, "new_value"),
            None => panic!("New key not found in third session"),
        }

        drop(btree);
    }

    println!("Persistence test completed successfully");
}

#[test]
fn test_root_splitting_persistence() {
    let (_dir, db_path) = create_temp_db();

    // Short keys pack ~170 per leaf; insert enough to split the root leaf.
    {
        let mut btree = open_btree(&db_path);

        let initial_root = btree.root_page_id();
        println!("Initial root before splits: {}", initial_root);

        for i in 0..250 {
            let key = format!("split_key_{:04}", i);
            let value = format!("split_value_{}", i);
            btree
                .insert(&key, &value)
                .unwrap_or_else(|_| panic!("Failed to insert key {i}"));
        }

        let final_root = btree.root_page_id();
        println!("Final root after splits: {}", final_root);
        assert_ne!(
            final_root, initial_root,
            "root leaf should split with 250 keys"
        );

        for i in 0..250 {
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

    {
        let mut btree = open_btree(&db_path);
        let reloaded_root = btree.root_page_id();
        println!("Reloaded root: {}", reloaded_root);

        for i in 0..250 {
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
    let (_dir, db_path) = create_temp_db();

    const KEYS_SESSION_1: usize = 500;
    const KEYS_SESSION_2: usize = 500;

    {
        let mut btree = open_btree(&db_path);

        println!("Session 1: Inserting {} keys...", KEYS_SESSION_1);
        for i in 0..KEYS_SESSION_1 {
            let key = format!("session1_key_{:04}", i);
            let value = format!("session1_value_{}", i);
            btree
                .insert(&key, &value)
                .unwrap_or_else(|_| panic!("Failed to insert key {i}"));
        }

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

    {
        let mut btree = open_btree(&db_path);

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

        println!("Session 2: Inserting {} NEW keys...", KEYS_SESSION_2);
        for i in 0..KEYS_SESSION_2 {
            let key = format!("session2_key_{:04}", i);
            let value = format!("session2_value_{}", i);
            btree
                .insert(&key, &value)
                .unwrap_or_else(|_| panic!("Failed to insert session 2 key {i}"));
        }

        println!(
            "Session 2: Verifying all {} keys...",
            KEYS_SESSION_1 + KEYS_SESSION_2
        );

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

        for i in 0..KEYS_SESSION_2 {
            let key = format!("session2_key_{:04}", i);
            let expected = format!("session2_value_{}", i);
            let result = btree.get(&key).expect("Failed to get key");
            assert_eq!(result, Some(expected), "Session 2 key {} missing", i);
        }

        btree.sync().expect("Failed to sync");
        drop(btree);
    }

    {
        let mut btree = open_btree(&db_path);

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
    let (_dir, db_path) = create_temp_db();
    let mut btree = open_btree(&db_path);

    btree.insert("key1", "value1").expect("Failed to insert");
    assert_eq!(btree.get("key1").unwrap(), Some("value1".to_string()));

    let deleted = btree.delete("key1").expect("Failed to delete");
    assert!(deleted, "Key should have been deleted");
    assert_eq!(btree.get("key1").unwrap(), None);

    let deleted_again = btree.delete("key1").expect("Failed to delete again");
    assert!(!deleted_again, "Key should not exist to delete");

    println!("Single key deletion test completed successfully");
}

#[test]
fn test_delete_multiple_keys() {
    let (_dir, db_path) = create_temp_db();
    let mut btree = open_btree(&db_path);

    const NUM_KEYS: usize = 20;
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let value = format!("value_{}", i);
        btree.insert(&key, &value).expect("Failed to insert");
    }

    for i in (0..NUM_KEYS).step_by(2) {
        let key = format!("key_{:04}", i);
        let deleted = btree.delete(&key).expect("Failed to delete");
        assert!(deleted, "Key {} should have been deleted", key);
    }

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
    let (_dir, db_path) = create_temp_db();
    let mut btree = open_btree(&db_path);

    const NUM_KEYS: usize = 50;
    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let value = format!("value_{}", i);
        btree.insert(&key, &value).expect("Failed to insert");
    }

    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let deleted = btree.delete(&key).expect("Failed to delete");
        assert!(deleted, "Key {} should have been deleted", key);
    }

    for i in 0..NUM_KEYS {
        let key = format!("key_{:04}", i);
        let result = btree.get(&key).expect("Failed to get");
        assert_eq!(result, None, "Key {} should be gone", key);
    }

    for i in 0..10 {
        let key = format!("new_key_{}", i);
        let value = format!("new_value_{}", i);
        btree
            .insert(&key, &value)
            .expect("Failed to insert new key");
    }

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
    let (_dir, db_path) = create_temp_db();

    {
        let mut btree = open_btree(&db_path);

        for i in 0..20 {
            let key = format!("key_{:04}", i);
            let value = format!("value_{}", i);
            btree.insert(&key, &value).expect("Failed to insert");
        }

        for i in 0..10 {
            let key = format!("key_{:04}", i);
            btree.delete(&key).expect("Failed to delete");
        }

        btree.sync().expect("Failed to sync");
        drop(btree);
    }

    {
        let mut btree = open_btree(&db_path);

        for i in 0..10 {
            let key = format!("key_{:04}", i);
            let result = btree.get(&key).expect("Failed to get");
            assert_eq!(
                result, None,
                "Deleted key {} should persist as deleted",
                key
            );
        }

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
    let (_dir, db_path) = create_temp_db();
    let mut btree = open_btree(&db_path);

    btree
        .insert("key1", "original_value")
        .expect("Failed to insert");
    assert_eq!(
        btree.get("key1").unwrap(),
        Some("original_value".to_string())
    );

    btree.delete("key1").expect("Failed to delete");
    assert_eq!(btree.get("key1").unwrap(), None);

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
    for i in 0..400 {
        btree
            .insert(&format!("k{:03}", i), &format!("v{i}"))
            .expect("insert");
    }
    let stats = btree.stats().expect("stats");
    assert!(stats.leaf_count > 1, "expected splits before range scan");
    let rows = Cursor::scan_range(&mut btree, Some("k010"), Some("k020")).expect("scan");
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[0].0, "k010");
    assert_eq!(rows[9].0, "k019");
    let all = Cursor::scan_range(&mut btree, None, None).expect("scan all");
    assert_eq!(all.len(), 400);
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

/// Insert without `BTree::sync` so the WAL still has records, then zero the
/// leaf with `Pager::new` (no WAL). Reopen must replay the write-path WAL.
#[test]
fn test_wal_recovers_unsynced_insert() {
    use btreedb::pager::{Pager, PAGE_SIZE};
    use std::fs::OpenOptions;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("unsync.db");

    {
        let mut btree = BTree::open(&db_path).expect("open");
        btree.insert("alpha", "one").expect("insert");
        // No sync: WAL retains the page image.
        drop(btree);
    }

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

    let mut btree = BTree::open(&db_path).expect("recover");
    assert_eq!(
        btree.get("alpha").expect("get"),
        Some("one".to_string()),
        "WAL from BTree::open writes should restore the leaf"
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

fn write_header_page(path: &Path, format_version: u16, page_size: u32) {
    use btreedb::pager::PAGE_SIZE;

    let mut page = vec![0u8; PAGE_SIZE];
    page[..7].copy_from_slice(b"BTREEDB");
    page[7..9].copy_from_slice(&format_version.to_le_bytes());
    page[9..13].copy_from_slice(&page_size.to_le_bytes());
    page[13..17].copy_from_slice(&1u32.to_le_bytes());
    page[17..21].copy_from_slice(&0u32.to_le_bytes());
    std::fs::write(path, page).expect("write header page");
}

#[test]
fn test_wrong_format_version_fails_open() {
    use btreedb::{FORMAT_VERSION, PAGE_SIZE};
    use std::io::ErrorKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("old_version.db");
    write_header_page(&db_path, 1, PAGE_SIZE as u32);

    let err = match BTree::open(&db_path) {
        Ok(_) => panic!("format version 1 must fail open on v{FORMAT_VERSION}"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("Unsupported format version 1"),
        "error={err}"
    );
    assert!(
        err.to_string()
            .contains(&format!("expected {FORMAT_VERSION}")),
        "error={err}"
    );
}

#[test]
fn test_wrong_page_size_fails_open() {
    use btreedb::{FORMAT_VERSION, PAGE_SIZE};
    use std::io::ErrorKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("bad_page.db");
    write_header_page(&db_path, FORMAT_VERSION, 8192);

    let err = match BTree::open(&db_path) {
        Ok(_) => panic!("page size 8192 must fail open"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("Unsupported page size 8192"),
        "error={err}"
    );
    assert!(
        err.to_string().contains(&format!("expected {PAGE_SIZE}")),
        "error={err}"
    );
}

/// Insert enough keys to split, delete until a leaf merge fires, reopen,
/// then check get/scan and that no non-root leaf is empty.
#[test]
fn test_delete_merge_reopen() {
    use btreedb::cursor::Cursor;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("merge.db");

    // Short keys pack ~170 per leaf; 400 keys force multiple leaves.
    const INSERT: usize = 400;
    const DELETE: usize = 300;

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

    // ~170 keys/leaf and MAX_INTERNAL_KEYS=10 → height 3 around ~1500+ keys.
    const INSERT: usize = 2000;
    const DELETE: usize = 1800;

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

#[test]
fn test_freelist_reuses_pages_after_merge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("freelist.db");
    let mut btree = BTree::open(&db_path).expect("open");

    const INSERT: usize = 400;
    for i in 0..INSERT {
        btree
            .insert(&format!("key_{:04}", i), &format!("value_{}", i))
            .expect("insert");
    }
    let pages_after_insert = btree.stats().expect("stats").page_count;
    assert!(pages_after_insert > 3, "expected several pages");

    for i in 0..300 {
        assert!(btree.delete(&format!("key_{:04}", i)).expect("delete"));
    }
    let pages_after_delete = btree.stats().expect("stats").page_count;
    assert_eq!(
        pages_after_delete, pages_after_insert,
        "freelist must not shrink the file"
    );

    for i in 0..300 {
        btree
            .insert(&format!("key_{:04}", i), &format!("value_{}", i))
            .expect("reinsert");
    }
    let pages_after_reinsert = btree.stats().expect("stats").page_count;
    assert_eq!(
        pages_after_reinsert, pages_after_insert,
        "reinsert should reuse freed pages instead of growing the file"
    );
    assert_eq!(btree.stats().expect("stats").key_count, INSERT as u64);
}

#[test]
fn test_oversized_pair_rejected() {
    use btreedb::pager::PAGE_SIZE;
    use std::io::ErrorKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("huge.db");
    let mut btree = BTree::open(&db_path).expect("open");
    let huge = "x".repeat(PAGE_SIZE);
    let err = btree.insert("k", &huge).expect_err("must reject");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(btree.get("k").expect("get").is_none());
}

/// Insert until a leaf split, drop without sync, zero every page in that
/// split's WAL frame, reopen: the whole group must come back together.
fn insert_until_leaf_split(btree: &mut BTree, start: u32) -> u32 {
    let leaves_before = btree.stats().expect("stats").leaf_count;
    let mut n = start;
    loop {
        btree
            .insert(&format!("key_{:04}", n), &format!("value_{}", n))
            .expect("insert");
        n += 1;
        if btree.stats().expect("stats").leaf_count > leaves_before {
            return n;
        }
        assert!(
            n < start + 400,
            "expected a leaf split within 400 short keys"
        );
    }
}

#[test]
fn test_wal_group_recovers_split_atomically() {
    use btreedb::pager::{Pager, PAGE_SIZE};
    use btreedb::wal::WAL;
    use std::fs::OpenOptions;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("group_split.db");

    let key_count = {
        let mut btree = BTree::open(&db_path).expect("open");
        let n = insert_until_leaf_split(&mut btree, 0);
        assert!(btree.stats().expect("stats").tree_height >= 2);
        drop(btree);
        n
    };

    let last_pages = {
        let mut wal = WAL::open(&db_path).expect("wal");
        let frames = wal.read_frames().expect("frames");
        let last = frames.last().expect("unsynced frames");
        assert!(
            last.pages.len() >= 2,
            "split/root change must be one multi-page frame, got {}",
            last.pages.len()
        );
        last.pages.iter().map(|r| r.page_id).collect::<Vec<_>>()
    };

    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&db_path)
            .expect("open db");
        let mut pager = Pager::new(file);
        for page_id in &last_pages {
            pager
                .write_page(*page_id, &[0u8; PAGE_SIZE])
                .expect("zero grouped page");
        }
    }

    let mut btree = BTree::open(&db_path).expect("recover");
    for i in 0..key_count {
        let key = format!("key_{:04}", i);
        assert_eq!(
            btree.get(&key).expect("get"),
            Some(format!("value_{}", i)),
            "missing {key} after grouped-split recovery"
        );
    }
    assert_no_empty_non_root_leaves(&mut btree);
}

/// A torn multi-page split frame must not apply a prefix. Restore the
/// pre-split snapshot and keep only a truncated last frame; reopen stays
/// at the snapshot and remains consistent.
#[test]
fn test_incomplete_split_frame_is_ignored() {
    use btreedb::wal::{WalFrame, WAL, WAL_HEADER_SIZE};

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("torn_split.db");

    {
        let mut btree = BTree::open(&db_path).expect("open");
        for i in 0..80 {
            btree
                .insert(&format!("key_{:04}", i), &format!("value_{}", i))
                .expect("insert");
        }
        btree.sync().expect("sync");
    }

    let snapshot = std::fs::read(&db_path).expect("snapshot db");
    let wal_path = WAL::wal_path(&db_path);

    let split_key = {
        let mut btree = BTree::open(&db_path).expect("open");
        let n = insert_until_leaf_split(&mut btree, 80);
        format!("key_{:04}", n - 1)
    };

    let wal_bytes = std::fs::read(&wal_path).expect("read wal");
    let last_page_count = {
        let mut wal = WAL::open(&db_path).expect("wal");
        let frames = wal.read_frames().expect("frames");
        let last = frames.last().expect("last frame");
        assert!(
            last.pages.len() >= 2,
            "expected multi-page split frame, got {}",
            last.pages.len()
        );
        last.pages.len()
    };
    let last_len = WalFrame::encoded_len(last_page_count) as usize;
    assert!(wal_bytes.len() >= WAL_HEADER_SIZE + last_len);
    let last_start = wal_bytes.len() - last_len;
    // Keep the frame header plus a few bytes so page_count still claims N pages.
    let torn_end = last_start + 20;
    assert!(torn_end < wal_bytes.len());

    std::fs::write(&db_path, snapshot).expect("restore snapshot");
    let mut torn = wal_bytes[..WAL_HEADER_SIZE].to_vec();
    torn.extend_from_slice(&wal_bytes[last_start..torn_end]);
    std::fs::write(&wal_path, torn).expect("write torn wal");

    let mut btree = BTree::open(&db_path).expect("open after torn frame");
    for i in 0..80 {
        let key = format!("key_{:04}", i);
        assert_eq!(
            btree.get(&key).expect("get"),
            Some(format!("value_{}", i)),
            "snapshot key {key} missing"
        );
    }
    assert_eq!(
        btree.get(&split_key).expect("get"),
        None,
        "torn split frame must not apply a prefix"
    );
    assert_no_empty_non_root_leaves(&mut btree);
}

#[test]
fn test_random_ops_reopen_consistent() {
    use btreedb::cursor::Cursor;
    use std::collections::HashMap;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("fuzz.db");
    let mut expected: HashMap<String, String> = HashMap::new();
    let mut rng = 0xC0FFEE_u64;

    fn next_rng(state: &mut u64) -> u64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *state
    }

    for round in 0..4 {
        let mut btree = BTree::open(&db_path).expect("open");
        for _ in 0..60 {
            let r = next_rng(&mut rng);
            let key = format!("k{:03}", r % 40);
            match r % 5 {
                0 | 1 => {
                    let value = format!("v{r}");
                    btree.insert(&key, &value).expect("insert");
                    expected.insert(key, value);
                }
                2 => {
                    btree.delete(&key).expect("delete");
                    expected.remove(&key);
                }
                _ => {
                    assert_eq!(btree.get(&key).expect("get"), expected.get(&key).cloned());
                }
            }
        }
        if round % 2 == 0 {
            btree.sync().expect("sync");
        }
        drop(btree);

        let mut btree = BTree::open(&db_path).expect("reopen");
        for (key, value) in &expected {
            assert_eq!(
                btree.get(key).expect("get").as_deref(),
                Some(value.as_str()),
                "mismatch for {key} after reopen"
            );
        }
        let scan = Cursor::scan_range(&mut btree, None, None).expect("scan");
        assert_eq!(scan.len(), expected.len());
        for (key, value) in &scan {
            assert_eq!(expected.get(key), Some(value));
        }
        assert_no_empty_non_root_leaves(&mut btree);
    }
}
