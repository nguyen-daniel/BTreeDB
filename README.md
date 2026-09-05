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

A split, merge, root change, or freelist update that happens inside that call is therefore one atomic WAL unit: recovery replays a **complete** frame in full, or ignores a **torn last frame**. It does not replay a prefix of the group. Ungrouped `write_page` calls (tests, recovery apply) still log a one-page frame.

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

```
btreedb> set name Alice
btreedb> get name
btreedb> scan a z
btreedb> .stats
btreedb> .exit
```

## Proof

Captured REPL session ([docs/repl_session.txt](docs/repl_session.txt)):

```
btreedb> set name Alice
OK
btreedb> get name
Alice
btreedb> scan
name -> Alice
(1 results)
btreedb> .stats
Database Statistics:
  Keys:           1
  Tree Height:    1
  Total Pages:    2
  Leaf Nodes:     1
  Internal Nodes: 0
```

| Claim | Test |
|-------|------|
| 1000 keys + splits + reopen | `test_1000_keys_persistence_reopen` |
| Delete merge + reopen | `test_delete_merge_reopen`, `test_delete_internal_merge` |
| Range scan | `test_range_scan_after_splits`, `cursor` unit tests |
| Magic header / persistence | `test_persistence_across_sessions`, `test_root_splitting_persistence` |
| Corrupt header fails open | `test_wrong_magic_fails_open`, `test_wrong_format_version_fails_open`, `test_wrong_page_size_fails_open` |
| WAL recover | `test_wal_recovers_zeroed_page`, `test_wal_recovers_unsynced_insert` |
| Split/merge WAL group | `test_wal_group_recovers_split_atomically`, `test_incomplete_split_frame_is_ignored` |
| Random ops + reopen | `test_random_ops_reopen_consistent` |
| Page freelist | `test_freelist_reuses_pages_after_merge` |

Binary vs JSONL size ([docs/format_bench.json](docs/format_bench.json), 1000 keys): B-tree **65,536** bytes vs JSONL **32,890** bytes (`btree_over_jsonl_size` **1.99**). Leaves pack to a 4KB byte budget; remaining overhead is page padding plus internal nodes. JSONL is a dump, not a database. Re-run with `cargo run --release --bin bench_format`.

## Reproduce

```bash
cargo test --test integration_test
cargo run --release --bin bench_format
```
