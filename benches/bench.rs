use btreedb::btree::BTree;
use btreedb::pager::Pager;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs::OpenOptions;
use tempfile::NamedTempFile;

/// Creates a fresh B-Tree for benchmarking.
/// Each benchmark gets a clean database to ensure fair comparisons.
fn create_btree() -> (BTree, NamedTempFile) {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(temp_file.path())
        .expect("Failed to open temp file");
    
    let pager = Pager::new(file);
    let btree = BTree::new(pager).expect("Failed to create BTree");
    (btree, temp_file)
}

/// Benchmarks insertion performance at different tree sizes.
/// Measures nanoseconds per insertion when inserting into a tree that already
/// contains a specific number of keys. This shows how insertion performance
/// changes as the tree grows.
fn bench_insertion_at_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("insertion_at_size");
    group.sample_size(10); // Reduce sample size for faster benchmarks with large datasets

    // Test insertion performance at different tree sizes
    let key_counts = vec![
        1_000,
        5_000,
        10_000,
        25_000,
        50_000,
        75_000,
        100_000,
    ];

    for &num_keys in &key_counts {
        group.bench_with_input(
            BenchmarkId::new("insert_into_tree", num_keys),
            &num_keys,
            |b, &num_keys| {
                b.iter_with_setup(
                    || {
                        // Setup: Create a B-Tree pre-populated with num_keys - 1 keys
                        // This allows us to measure insertion performance at a specific tree size
                        let (mut btree, _temp_file) = create_btree();
                        
                        for i in 0..num_keys - 1 {
                            let key = format!("key_{:08}", i);
                            let value = format!("value_{}", i);
                            btree
                                .insert(&key, &value)
                                .expect("Failed to insert during setup");
                        }
                        
                        btree
                    },
                    |mut btree| {
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

    let key_counts = vec![
        1_000,
        5_000,
        10_000,
        25_000,
        50_000,
        100_000,
    ];

    for &num_keys in &key_counts {
        group.bench_with_input(
            BenchmarkId::new("sequential", num_keys),
            &num_keys,
            |b, &num_keys| {
                b.iter_with_setup(
                    || create_btree().0,
                    |mut btree| {
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

criterion_group!(benches, bench_insertion_at_size, bench_sequential_insertion);
criterion_main!(benches);

