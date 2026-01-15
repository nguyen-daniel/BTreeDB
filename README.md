# BTreeDB

A clean, educational implementation of a persistent B-Tree database in Rust. This project implements a key-value store with on-disk persistence, featuring a REPL interface for interactive database operations.

## Project Structure

### Core Modules
- **`src/pager.rs`** - Manages file I/O operations, reading and writing 4KB pages to/from disk
- **`src/node.rs`** - Defines the B-Tree node structure (Internal and Leaf nodes) with serialization/deserialization
- **`src/btree.rs`** - Implements the B-Tree data structure with insert, search, delete, and split operations
- **`src/cursor.rs`** - Cursor for tree traversal and range queries
- **`src/value.rs`** - Multiple value type support (String, Integer, Float, Binary, Null)

### Durability & Transactions
- **`src/wal.rs`** - Write-Ahead Logging for crash recovery and durability
- **`src/transaction.rs`** - Transaction support with commit/rollback and savepoints
- **`src/backup.rs`** - Database backup and restore functionality

### Advanced Features
- **`src/compression.rs`** - Value compression using run-length encoding
- **`src/manager.rs`** - Multiple database instance management
- **`src/concurrency.rs`** - Page-level locking and connection pooling

### Application
- **`src/main.rs`** - Interactive REPL (Read-Eval-Print Loop) for database operations
- **`src/lib.rs`** - Library module exports

### Testing & CI
- **`tests/integration_test.rs`** - Integration test suite for database operations and persistence
- **`benches/bench.rs`** - Performance benchmarks using Criterion
- **`.github/workflows/ci.yml`** - GitHub Actions CI/CD workflow

## Installation

```bash
# Clone the repository
git clone <your-repo-url>
cd BTreeDB

# Build the project
cargo build --release
```

## Dependencies

### Runtime Dependencies

- **`byteorder`** - For serializing/deserializing binary data (little-endian)
- **`rustyline`** - For the interactive REPL interface with command history

### Development Dependencies

- **`tempfile`** - For safe temporary file management in tests
- **`criterion`** - For performance benchmarking with statistical analysis

## Usage

### Running the REPL

Start the interactive database REPL:

```bash
cargo run
# or
./target/release/btreedb
```

This will create or open a `btree.db` file in the current directory.

### Commands

The REPL supports the following commands:

#### Set a Key-Value Pair

```bash
btreedb> set name Alice
OK
btreedb> set age 30
OK
btreedb> set message Hello, World!
OK
```

#### Get a Value

```bash
btreedb> get name
Alice
btreedb> get age
30
btreedb> get nonexistent
(nil)
```

#### Delete a Key

```bash
btreedb> delete name
OK
btreedb> delete nonexistent
(nil)
```

#### Scan Range

```bash
btreedb> scan
key1 -> value1
key2 -> value2
(2 results)

btreedb> scan a z
apple -> fruit
banana -> fruit
(2 results)
```

#### Database Statistics

```bash
btreedb> .stats
Database Statistics:
  Keys:           100
  Tree Height:    3
  Total Pages:    25
  Leaf Nodes:     20
  Internal Nodes: 4
```

#### Tree Dump

```bash
btreedb> .dump
Tree Structure:
[Internal] keys: [key_50]
  [Leaf] keys: [key_01, key_02, ...]
  [Leaf] keys: [key_51, key_52, ...]
```

#### Exit

```bash
btreedb> .exit
All data flushed to disk. Goodbye!
```

The `.exit` command ensures all dirty pages are flushed to disk using `file.sync_all()` before closing.

## Architecture Overview

The database uses a B-Tree structure with the following components:

### Page Layout

- **Page Size**: 4KB (4096 bytes)
- **Page 0**: Reserved for database header (first 100 bytes)
  - Magic bytes: "BTREEDB" (7 bytes)
  - Root page ID (4 bytes, little-endian)
  - Reserved space (89 bytes)
- **Page 1+**: B-Tree nodes

### Node Types

1. **Leaf Nodes**:
   - Store key-value pairs (Strings)
   - Maximum 3 keys per leaf (configurable via `MAX_LEAF_KEYS`, reduced to support 1KB values)
   - When full, split into two nodes

2. **Internal Nodes**:
   - Store keys and child page IDs
   - For n keys, there are n+1 children
   - Keys separate the ranges of child nodes

### Serialization Format

Each node is serialized into a 4096-byte buffer:

- **Byte 0**: Node type (0 = Leaf, 1 = Internal)
- **Bytes 1-4**: Number of keys (u32, little-endian)
- **Data**:
  - Leaf: Key-value pairs (each with length prefix + bytes)
  - Internal: Keys (with length prefixes) followed by child page IDs (u32 each)
- **Remainder**: Zero-padded to exactly 4096 bytes

### Operations

1. **Insert**: Recursively traverses the tree to find the appropriate leaf, inserts the key-value pair, and splits if necessary
2. **Search**: Recursively traverses the tree following key ranges to find the target leaf, then searches within the leaf
3. **Split**: When a leaf exceeds the maximum capacity, it splits in half, creating a new leaf and updating the parent internal node

## Key Features

- **Persistent Storage**: All data is stored on disk in a binary format
- **B-Tree Structure**: Efficient O(log n) search, insert, and delete operations
- **CRUD Operations**: Full create, read, update, delete support with proper rebalancing
- **Range Queries**: Cursor-based iteration and range scanning
- **Multiple Value Types**: Support for strings, integers, floats, binary data, and null
- **Write-Ahead Logging**: Crash recovery through WAL replay
- **Transaction Support**: ACID transactions with commit, rollback, and savepoints
- **Value Compression**: Automatic compression for large repetitive values
- **Backup/Restore**: Hot backup capability without stopping the database
- **Multi-Database**: Manage multiple database instances in a single process
- **Concurrent Access**: Page-level locking with multiple readers/single writer
- **Interactive REPL**: User-friendly command-line interface with history support
- **Robust Deserialization**: Bounds checking prevents crashes from corrupted files
- **Safe Page Allocation**: Page IDs derived from file size on reopen (no overwrites)
- **Comprehensive Testing**: Integration tests verify correctness and persistence
- **Performance Benchmarks**: Criterion-based benchmarks measure performance at scale
- **CI/CD**: Automated testing and code quality checks via GitHub Actions

## Implementation Details

### Leaf Node Splitting

When a leaf node contains more than 3 key-value pairs (MAX_LEAF_KEYS):
1. Split the pairs in half
2. Create a new leaf node with the right half
3. Update the original leaf with the left half
4. Return the first key of the new node as the separator
5. Update the parent internal node with the separator key and new child pointer
6. If splitting the root, create a new internal root node

### Page Management

- The `Pager` struct manages all file I/O operations
- Pages are read and written at 4KB boundaries
- Page writes are flushed but not synced on every write for performance
- `sync_all()` is called on `.exit` to ensure all data is persisted to disk
- The pager uses `std::io::Seek` to jump to the correct file offset

## Development

### Running Tests

The project includes comprehensive integration tests that verify:
- Large-scale insertion (1000+ keys) triggering multiple node splits
- Data persistence across database sessions
- Root splitting and header updates
- Insert-after-reopen correctness (prevents page overwrites)

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run only integration tests
cargo test --test integration_test
```

### Running Benchmarks

Performance benchmarks measure various aspects of the database:

```bash
# Run all benchmarks
cargo bench --bench bench

# Throughput (Writes/Sec): Measure how many 1KB records can be inserted per second
cargo bench --bench bench -- write_throughput

# Lookup Latency (ms): Compare B-Tree index lookup vs linear scan
cargo bench --bench bench -- lookup_latency

# Storage Efficiency: Compare raw data size to total file size (overhead percentage)
cargo bench --bench bench -- storage_efficiency

# Recovery Time: Measure how quickly the database reloads its state after a crash simulation
cargo bench --bench bench -- recovery_time
```

The benchmarks generate HTML reports in `target/criterion/` showing:
- Throughput measurements (writes per second) at various milestones
- Lookup latency comparison between B-Tree index and linear scan
- Storage efficiency metrics (raw data size vs total file size with overhead percentage)
- Recovery time measurements for different database sizes
- Statistical analysis with confidence intervals

### Code Quality

The project uses automated code quality checks:

```bash
# Check code formatting
cargo fmt --check

# Run Clippy linter
cargo clippy -- -D warnings

# Format code
cargo fmt
```

### Continuous Integration

The project includes a GitHub Actions workflow (`.github/workflows/ci.yml`) that runs on every push to `main`:
- Installs stable Rust toolchain
- Caches Rust dependencies for faster builds
- Checks code formatting with `cargo fmt --check`
- Runs Clippy with `-D warnings` for strict linting
- Executes all unit and integration tests

## Example Session

```bash
$ cargo run
B-Tree Database REPL
Commands:
  set <key> <value>  - Insert or update a key-value pair
  get <key>          - Retrieve a value by key
  .exit              - Exit and flush all data to disk

btreedb> set user1 name1
OK
btreedb> set user2 name2
OK
btreedb> get user1
name1
btreedb> set user1 updated_name
OK
btreedb> get user1
updated_name
btreedb> .exit
All data flushed to disk. Goodbye!
```

## Implemented Features

### Core Features (MVP)
- **Persistent Storage**: All data is stored on disk in a binary format
- **B-Tree Structure**: Efficient O(log n) search, insert, and delete operations
- **Automatic Splitting**: Leaf and internal nodes automatically split when they exceed capacity
- **Root Tracking**: Database header tracks the current root page ID
- **Magic Bytes**: File signature ensures database file integrity
- **Interactive REPL**: User-friendly command-line interface with history support
- **Data Safety**: All writes are synced to disk on exit

### Delete Operations (Phase 1)
- Key deletion with proper node rebalancing
- Node merging when underflow occurs
- Root demotion when tree shrinks
- REPL `delete <key>` command

### Cursor and Range Queries (Phase 2)
- `Cursor` struct for efficient tree traversal
- `seek()`, `next()`, `seek_first()` navigation
- `scan_range(start, end)` for range queries
- REPL `scan [start] [end]` command

### Database Statistics (Phase 3)
- Key count, tree height, page count tracking
- Leaf and internal node counts
- `.stats` command for statistics display
- `.dump` command for tree structure visualization

### Multiple Value Types (Phase 4)
- `Value` enum supporting: String, Integer, Float, Binary, Null
- Type-prefixed input parsing (e.g., `i:42` for integer, `f:3.14` for float)
- Backwards-compatible with string-only data

### Write-Ahead Logging (Phase 5)
- WAL file (`*.db-wal`) alongside main database
- Page-level logging with checksums
- Crash recovery by replaying WAL on startup
- Checkpoint mechanism to clear WAL after sync

### Transaction Support (Phase 6)
- Begin/commit/rollback semantics
- Transaction manager for coordination
- Savepoints for partial rollback
- Integration with WAL for durability

### Value Compression (Phase 7)
- Run-length encoding (RLE) for repetitive data
- Automatic compression for values above threshold
- Compression statistics tracking
- Transparent decompression on read

### Backup and Restore (Phase 8)
- Hot backup capability (no downtime required)
- Optional WAL file backup
- Backup verification
- Full restore functionality

### Multiple Database Support (Phase 9)
- `DatabaseManager` for handling multiple instances
- Named database handles
- Independent data isolation
- Configurable database options

### Concurrent Access (Phase 10)
- Page-level read-write locks
- Multiple readers, single writer pattern
- Lock manager with deadlock prevention
- Connection pooling with configurable limits

## Future Improvements

- [ ] Add more benchmark scenarios (search performance, concurrent access)
- [ ] Expand test coverage (edge cases, error handling)
- [ ] Implement MVCC for snapshot isolation
- [ ] Add network protocol for client-server mode
- [ ] Implement secondary indexes

## References

- [B-Tree Data Structure](https://en.wikipedia.org/wiki/B-tree) - Wikipedia article on B-Trees
- [Database Internals](https://www.oreilly.com/library/view/database-internals/9781492040331/) - Book on database implementation
- [SQLite Architecture](https://www.sqlite.org/arch.html) - SQLite's page-based architecture

## License

This project is for educational purposes.

