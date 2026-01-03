use btreedb::btree::BTree;
use btreedb::pager::Pager;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::Instant;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates a fresh B-Tree for benchmarking.
/// Each benchmark gets a clean database to ensure fair comparisons.
fn create_btree() -> (BTree, PathBuf) {
    // Create temp file in workspace to avoid sandbox permission issues
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    let file_path = PathBuf::from(format!("target/bench_db_{}.bin", counter));
    
    // Remove file if it exists
    let _ = std::fs::remove_file(&file_path);
    
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .expect("Failed to open temp file");

    let pager = Pager::new(file);
    let btree = BTree::new(pager).expect("Failed to create BTree");
    (btree, file_path)
}

/// Benchmarks insertion performance at different tree sizes.
/// Measures nanoseconds per insertion when inserting into a tree that already
/// contains a specific number of keys. This shows how insertion performance
/// changes as the tree grows.
fn bench_insertion_at_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("insertion_at_size");
    group.sample_size(10); // Reduce sample size for faster benchmarks with large datasets

    // Test insertion performance at different tree sizes
    // Start conservatively - if 5000 fails, try: vec![1_000, 2_000, 3_000, 4_000, 5_000];
    // Full production: vec![1_000, 5_000, 10_000, 25_000, 50_000, 75_000, 100_000];
    let key_counts = vec![1_000, 2_000, 3_000, 5_000, 10_000, 25_000];

    for &num_keys in &key_counts {
        group.bench_with_input(
            BenchmarkId::new("insert_into_tree", num_keys),
            &num_keys,
            |b, &num_keys| {
                b.iter_with_setup(
                    || {
                        // Setup: Create a B-Tree pre-populated with num_keys - 1 keys
                        // This allows us to measure insertion performance at a specific tree size
                        let (mut btree, temp_file) = create_btree();

                        for i in 0..num_keys - 1 {
                            let key = format!("key_{:08}", i);
                            let value = format!("value_{}", i);
                            btree
                                .insert(&key, &value)
                                .expect("Failed to insert during setup");
                        }

                        // Keep temp_file alive to prevent file deletion
                        (btree, temp_file)
                    },
                    |(mut btree, _temp_file)| {
                        // Benchmark: Insert one more key into the pre-populated tree
                        // This measures insertion performance at the current tree size
                        let key = format!("key_{:08}", num_keys - 1);
                        let value = format!("value_{}", num_keys - 1);
                        btree
                            .insert(black_box(&key), black_box(&value))
                            .expect("Failed to insert during benchmark");
                        black_box(&mut btree);
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmarks sequential insertion performance.
/// Measures the time to insert keys one by one, showing how performance
/// changes as the tree grows from empty to the target size.
fn bench_sequential_insertion(c: &mut Criterion) {
    let mut group = c.benchmark_group("sequential_insertion");
    group.sample_size(10);

    // Start with smaller sizes and gradually increase
    // Full production: vec![1_000, 5_000, 10_000, 25_000, 50_000, 100_000];
    let key_counts = vec![1_000, 2_500, 5_000, 10_000, 25_000, 50_000];

    for &num_keys in &key_counts {
        group.bench_with_input(
            BenchmarkId::new("sequential", num_keys),
            &num_keys,
            |b, &num_keys| {
                b.iter_with_setup(
                    || create_btree(),
                    |(mut btree, _temp_file)| {
                        // Insert all keys sequentially
                        for i in 0..num_keys {
                            let key = format!("key_{:08}", i);
                            let value = format!("value_{}", i);
                            btree
                                .insert(black_box(&key), black_box(&value))
                                .expect("Failed to insert");
                        }
                        black_box(btree);
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmarks write throughput: Measures how many 1KB records can be inserted per second
/// until reaching 1 million entries. Reports writes/sec at various milestones.
fn bench_write_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_throughput");
    group.sample_size(10); // Criterion requires at least 10 samples

    // Test at various milestones (reduced to avoid disk space issues)
    // With 1KB values and MAX_LEAF_KEYS=3, large datasets consume significant disk space
    // 100K records ≈ 100MB+, 250K ≈ 250MB+ per benchmark iteration
    let milestones = vec![
        10_000,
        50_000,
        100_000,
        250_000,
    ];

    for &target_count in &milestones {
        group.bench_with_input(
            BenchmarkId::new("1kb_records", target_count),
            &target_count,
            |b, &target_count| {
                b.iter_with_setup(
                    || create_btree(),
                    |(mut btree, _temp_file)| {
                        // Create 1KB value (1024 bytes)
                        // Note: MAX_LEAF_KEYS is set to 3 to accommodate 1KB values in 4KB pages
                        let value_1kb = "x".repeat(1024);
                        
                        let start = Instant::now();
                        for i in 0..target_count {
                            let key = format!("key_{:010}", i);
                            btree
                                .insert(&key, &value_1kb)
                                .expect("Failed to insert");
                        }
                        let elapsed = start.elapsed();
                        
                        // Calculate and print throughput
                        let writes_per_sec = target_count as f64 / elapsed.as_secs_f64();
                        eprintln!(
                            "Inserted {} records in {:?} ({:.2} writes/sec)",
                            target_count, elapsed, writes_per_sec
                        );
                        
                        black_box(btree);
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmarks lookup latency: Compares B-Tree index lookup vs linear scan.
/// Measures average time to retrieve a specific key.
fn bench_lookup_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("lookup_latency");
    group.sample_size(100);

    // Test at different database sizes (reduced to avoid disk space issues)
    // With 1KB values, 500K records = ~500MB+ per benchmark run
    let db_sizes = vec![1_000, 10_000, 50_000, 100_000];

    for &db_size in &db_sizes {
        // Test B-Tree lookup
        group.bench_with_input(
            BenchmarkId::new("btree_lookup", db_size),
            &db_size,
            |b, &db_size| {
                b.iter_with_setup(
                    || {
                        // Setup: Create a database with db_size entries
                        let (mut btree, file_path) = create_btree();
                        // Use 1KB values (1024 bytes) - MAX_LEAF_KEYS is set to 3 to support this
                        let value_1kb = "x".repeat(1024);
                        
                        // Insert all records
                        for i in 0..db_size {
                            let key = format!("key_{:010}", i);
                            btree.insert(&key, &value_1kb).expect("Failed to insert");
                        }
                        
                        // Sync and drop to ensure data is written
                        btree.sync().expect("Failed to sync");
                        drop(btree);
                        
                        // Get test key (middle key for consistency)
                        let test_key = format!("key_{:010}", db_size / 2);
                        
                        (file_path, test_key)
                    },
                    |(file_path, test_key)| {
                        // Reopen for benchmarking
                        let file = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(&file_path)
                            .expect("Failed to reopen file");
                        let pager = Pager::new(file);
                        let mut btree = BTree::new(pager).expect("Failed to create BTree");
                        // Benchmark: Look up the key
                        let result = btree.get(black_box(&test_key)).expect("Lookup failed");
                        black_box(result);
                    },
                );
            },
        );
        
        // Test linear scan (in-memory for comparison)
        group.bench_with_input(
            BenchmarkId::new("linear_scan", db_size),
            &db_size,
            |b, &db_size| {
                b.iter_with_setup(
                    || {
                        // Create a simple Vec<(String, String)> for linear scan
                        // Use 350 bytes to match write_throughput benchmark
                        let value_1kb = "x".repeat(350);
                        let data: Vec<(String, String)> = (0..db_size)
                            .map(|i| {
                                let key = format!("key_{:010}", i);
                                (key, value_1kb.clone())
                            })
                            .collect();
                        
                        let test_key = format!("key_{:010}", db_size / 2);
                        (data, test_key)
                    },
                    |(data, test_key)| {
                        // Benchmark: Linear scan
                        let result = data.iter().find(|(k, _)| k == &test_key);
                        black_box(result);
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmarks storage efficiency: Compares raw data size to total file size
/// (including B-Tree headers/padding) to show overhead percentage.
fn bench_storage_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_efficiency");
    group.sample_size(10); // Need at least 10 samples for criterion

    // Test at various database sizes (reduced to avoid disk space issues)
    // With 1KB values, large datasets consume significant disk space
    let db_sizes = vec![1_000, 10_000, 50_000, 100_000, 250_000];

    for &db_size in &db_sizes {
        group.bench_with_input(
            BenchmarkId::new("overhead", db_size),
            &db_size,
            |b, &db_size| {
                b.iter_with_setup(
                    || {
                        let (mut btree, file_path) = create_btree();
                        // Use 1KB values (1024 bytes) - MAX_LEAF_KEYS is set to 3 to support this
                        let value_1kb = "x".repeat(1024);
                        
                        // Insert all records
                        for i in 0..db_size {
                            let key = format!("key_{:010}", i);
                            btree.insert(&key, &value_1kb).expect("Failed to insert");
                        }
                        
                        // Sync to ensure all data is written
                        btree.sync().expect("Failed to sync");
                        
                        (btree, file_path)
                    },
                    |(_btree, file_path)| {
                        // Calculate raw data size
                        // Each record: key (14 bytes: "key_" + 10 digits) + value (1024 bytes) = 1038 bytes
                        let key_size = 14; // "key_0000000000"
                        let value_size = 1024;
                        let raw_data_size = db_size as u64 * (key_size + value_size);
                        
                        // Get actual file size
                        let file_size = std::fs::metadata(&file_path)
                            .expect("Failed to get file metadata")
                            .len();
                        
                        // Calculate overhead
                        let overhead_bytes = file_size.saturating_sub(raw_data_size);
                        let overhead_percent = if raw_data_size > 0 {
                            (overhead_bytes as f64 / raw_data_size as f64) * 100.0
                        } else {
                            0.0
                        };
                        
                        eprintln!(
                            "Database size: {} records",
                            db_size
                        );
                        eprintln!(
                            "Raw data size: {} bytes ({:.2} MB)",
                            raw_data_size,
                            raw_data_size as f64 / 1_048_576.0
                        );
                        eprintln!(
                            "File size: {} bytes ({:.2} MB)",
                            file_size,
                            file_size as f64 / 1_048_576.0
                        );
                        eprintln!(
                            "Overhead: {} bytes ({:.2}%)",
                            overhead_bytes, overhead_percent
                        );
                        
                        black_box((raw_data_size, file_size, overhead_percent));
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmarks recovery time: Measures how quickly the database reloads its state
/// (re-reading the root page ID and reconstructing the tree) after a crash simulation.
fn bench_recovery_time(c: &mut Criterion) {
    let mut group = c.benchmark_group("recovery_time");
    group.sample_size(10);

    // Test at various database sizes (reduced to avoid disk space issues)
    // With 1KB values, large datasets consume significant disk space
    let db_sizes = vec![1_000, 10_000, 50_000, 100_000, 250_000];

    for &db_size in &db_sizes {
        group.bench_with_input(
            BenchmarkId::new("reload", db_size),
            &db_size,
            |b, &db_size| {
                b.iter_with_setup(
                    || {
                        // Setup: Create a database with db_size entries
                        let (mut btree, file_path) = create_btree();
                        // Use 1KB values (1024 bytes) - MAX_LEAF_KEYS is set to 3 to support this
                        let value_1kb = "x".repeat(1024);
                        
                        // Insert all records
                        for i in 0..db_size {
                            let key = format!("key_{:010}", i);
                            btree.insert(&key, &value_1kb).expect("Failed to insert");
                        }
                        
                        // Sync to ensure all data is written
                        btree.sync().expect("Failed to sync");
                        
                        // Drop the BTree to simulate a crash
                        drop(btree);
                        
                        file_path
                    },
                    |file_path| {
                        // Benchmark: Reopen the database (recovery)
                        let start = Instant::now();
                        
                        let file = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(&file_path)
                            .expect("Failed to reopen file");
                        let pager = Pager::new(file);
                        let btree = BTree::new(pager).expect("Failed to recover BTree");
                        
                        let elapsed = start.elapsed();
                        
                        // Verify recovery by reading root page ID
                        let root_id = btree.root_page_id();
                        black_box(root_id);
                        
                        eprintln!(
                            "Recovery time for {} records: {:?} ({:.2} ms)",
                            db_size,
                            elapsed,
                            elapsed.as_secs_f64() * 1000.0
                        );
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_insertion_at_size,
    bench_sequential_insertion,
    bench_write_throughput,
    bench_lookup_latency,
    bench_storage_efficiency,
    bench_recovery_time
);
criterion_main!(benches);
