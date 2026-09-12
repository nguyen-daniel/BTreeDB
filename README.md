# BTreeDB

[![CI](https://github.com/nguyen-daniel/BTreeDB/actions/workflows/ci.yml/badge.svg)](https://github.com/nguyen-daniel/BTreeDB/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

On-disk B-tree key-value store in Rust: **4KB pages**, binary node serialization, **splits and merges**, **magic-byte header** (format version **2** + page size), **range scans**, and a **REPL**. Released under the MIT License. Integration tests insert **1,000+ keys**, assert height/splits, delete until a merge, and **reopen** the file through `BTree::open` (WAL attached).

**On-disk v2 is a breaking change.** Version 1 files (and older WAL wrapping-sum files) fail to open with a clear error. There is no migrator.

This README describes the **integrated core** (pager → nodes → B-tree → cursor → REPL). Extra modules exist in-tree as experiments; they are not the resume story.

## Core (what is wired)

| Piece | Role |
|-------|------|
| `src/pager.rs` | 4KB page I/O. `Pager::open` recovers WAL then logs new writes. `insert`/`delete` group page writes into one WAL frame; the DB file is flushed and fsynced on `BTree::sync` / recovery |
| `src/node.rs` | Leaf / internal nodes, binary layout; leaves pack to a page byte budget |
| `src/btree.rs` | Insert, get, delete, splits, borrow/merge, header (`BTREEDB` + format version **2**), page freelist. Internals cap at `MAX_INTERNAL_KEYS = 10` |
| `src/cursor.rs` | Range scan `[start, end)` |
| `src/main.rs` | REPL: `set` / `get` / `delete` / `scan` / `.stats` / `.dump` |
| `src/wal.rs` | Write-ahead log **on the open/write path** (`BTree::open` / `Pager::open`): one CRC32 frame per logical write group, then apply; replay complete frames on open |

`Pager::new(file)` does **not** attach WAL (used to inject pages in tests). The REPL and `BTree::open(path)` do.

Internal nodes still split at **`MAX_INTERNAL_KEYS = 10`** (a count cap, not a byte budget). That is intentional: height-3 trees stay easy to hit in tests with short keys. Leaves pack until the 4KB page fills. A merged internal must still fit in a page (`Node::internal_fits`).

### Durability

Crash safety is **WAL-first**. `insert`, `delete`, and new-file initialize buffer every page write and append **one WAL frame** (one or more page images, CRC32, one `fsync`). Then those pages are written to the DB file and `flush()`ed, not fsynced.

A split, merge, root change, or freelist update that happens inside that call is therefore one atomic WAL unit: recovery replays a **complete** frame in full, or ignores a **torn or corrupt last frame** (short read, checksum mismatch, or invalid length). It does not replay a prefix of the group. Bytes after the last good frame are truncated so the next append does not write past unparseable data. Ungrouped `write_page` calls (tests, recovery apply) still log a one-page frame.

If `insert`/`delete` fails after touching the in-memory root, page allocator, or freelist — including when `commit_write_group` logs a WAL frame and then fails to apply it to the DB file — those fields are rolled back and the just-logged frame is truncated so a later call cannot use a root or free page that was never committed.

This is **not** group-commit across concurrent clients (the engine is single-threaded), and it is **not** an in-memory page cache: the write-group buffer exists only for the current `insert`/`delete` so later steps can read pages they just wrote.

`BTree::sync()` (REPL `.exit`) fsyncs the DB file and then checkpoints (truncates) the WAL. After a successful sync, the DB file is the durable copy.

WAL frames use CRC32 (`crc32fast`). A wrapping-sum WAL from before v2 will not open.

## Not the core (experimental)

`transaction.rs`, `compression.rs`, `concurrency.rs`, `backup.rs`, `value.rs`, `manager.rs` — compiled only with `--features experimental`. Not required to explain the storage engine.

## How to run

```bash
git clone https://github.com/nguyen-daniel/BTreeDB.git
cd BTreeDB
cargo test
cargo test --features experimental
cargo run --release
```

`cargo run --release` starts the REPL (`default-run = "btreedb"`). The extra binary is `cargo run --release --bin bench_format`.

```
btreedb> set name Alice
btreedb> get name
btreedb> scan a z
btreedb> .stats
btreedb> .dump
btreedb> .exit
```

### Split + persist (one command)

40 keys with 200-byte values force a leaf split. The example dumps the tree (height ≥ 2, more than one leaf), syncs, reopens the file, and `get`s the first and last keys.

```bash
cargo run --release --example split_demo
```

Captured output ([docs/repl_session.txt](docs/repl_session.txt)):

```
== insert 40 keys with 200-byte values ==
Database Statistics:
  Keys:           40
  Tree Height:    2
  Total Pages:    6
  Leaf Nodes:     4
  Internal Nodes: 1

Tree Structure:
[Internal@3] 3 keys: key011, key021, key031
  [Leaf@1] 10 keys: key001, key002, ... key010
  [Leaf@2] 10 keys: key011, key012, ... key020
  [Leaf@4] 10 keys: key021, key022, ... key030
  [Leaf@5] 10 keys: key031, key032, ... key040
synced and closed

== reopen <temp>/btreedb_split_demo/demo.db ==
get key001 -> 200-byte value (persisted)
get key040 -> 200-byte value (persisted)
reopen stats: keys=40 height=2 leaves=4
```

### Crash recovery (one command)

Zeros a leaf after an unsynced insert, then reopens: WAL replay restores `alpha`. A second step truncates the last multi-page split frame; reopen keeps the snapshot and drops the torn write.

```bash
cargo run --release --example crash_recover
```

Captured output ([docs/crash_recover.txt](docs/crash_recover.txt)):

```
== 1. unsynced insert, then a zeroed leaf ==
inserted alpha=one (no BTree::sync; WAL still has the page)
zeroed leaf page 1 in the DB file (Pager::new, no WAL)
reopen recovered alpha -> one

== 2. torn last WAL frame is ignored ==
synced 80 keys (checkpointed WAL)
inserted through a leaf split (last key key_0168)
last WAL frame has 4 pages (split group)
restored pre-split DB; truncated last WAL frame 378300 -> 52 bytes
snapshot key_0000 -> value_0 (kept)
key_0168 -> (nil) (torn split frame ignored)
```

Same stories as `test_wal_recovers_unsynced_insert` and `test_incomplete_split_frame_is_ignored`, printed so you can watch them.

## Proof

| Claim | Test |
|-------|------|
| 1000 keys + splits + reopen | `test_1000_keys_persistence_reopen` |
| Delete merge + reopen | `test_delete_merge_reopen`, `test_delete_internal_merge` |
| Range scan | `test_range_scan_after_splits`, `cursor` unit tests |
| Magic header / persistence | `test_persistence_across_sessions`, `test_root_splitting_persistence` |
| Corrupt header fails open | `test_wrong_magic_fails_open`, `test_wrong_format_version_fails_open`, `test_wrong_page_size_fails_open` |
| WAL recover | `test_wal_recovers_zeroed_page`, `test_wal_recovers_unsynced_insert` |
| Split/merge WAL group | `test_wal_group_recovers_split_atomically`, `test_incomplete_split_frame_is_ignored` |
| Corrupt last WAL frame ignored | `test_corrupt_last_wal_frame_is_ignored`, `test_corrupt_last_frame_is_ignored_and_tail_trimmed` |
| Failed write rolls back allocator | `write_group_error_rolls_back_allocator_and_root`, `commit_apply_failure_rolls_back_wal_and_allocator` |
| Random ops + reopen | `test_random_ops_reopen_consistent` |
| Page freelist | `test_freelist_reuses_pages_after_merge` |

Binary vs JSONL size ([docs/format_bench.json](docs/format_bench.json), 1000 keys): B-tree **65,536** bytes vs JSONL **32,890** bytes (`btree_over_jsonl_size` **1.99**). Leaves pack to a 4KB byte budget; remaining overhead is page padding plus internal nodes. JSONL is a dump, not a database. Re-run with `cargo run --release --bin bench_format`.

## Reproduce

```bash
cargo test --test integration_test
cargo run --release --example split_demo
cargo run --release --example crash_recover
cargo run --release --bin bench_format
```
