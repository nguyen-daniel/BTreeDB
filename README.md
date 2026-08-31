# BTreeDB

[![CI](https://github.com/nguyen-daniel/BTreeDB/actions/workflows/ci.yml/badge.svg)](https://github.com/nguyen-daniel/BTreeDB/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

On-disk B-tree key-value store in Rust: **4KB pages**, binary node serialization, **splits and merges**, **magic-byte header**, **range scans**, and a **REPL**. Released under the MIT License. Integration tests insert **1,000+ keys**, assert height/splits, delete until a merge, and **reopen** the file.

This README describes the **integrated core** (pager → nodes → B-tree → cursor → REPL). Extra modules exist in-tree as experiments; they are not the resume story.

## Core (what is wired)

| Piece | Role |
|-------|------|
| `src/pager.rs` | 4KB page I/O. `Pager::open` recovers WAL then logs new writes |
| `src/node.rs` | Leaf / internal nodes, binary layout |
| `src/btree.rs` | Insert, get, delete, splits, borrow/merge, header (`BTREEDB` magic) |
| `src/cursor.rs` | Range scan `[start, end)` |
| `src/main.rs` | REPL: `set` / `get` / `delete` / `scan` / `.stats` / `.dump` |
| `src/wal.rs` | Write-ahead log **on the open/write path** (`BTree::open` / `Pager::open`): log page, then apply; replay on open |

`Pager::new(file)` (tests that pass a raw `File`) does **not** attach WAL. The REPL and `BTree::open(path)` do.

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
| Corrupt header fails open | `test_wrong_magic_fails_open` |
| WAL recover | `test_wal_recovers_zeroed_page` |

Binary vs JSONL size ([docs/format_bench.json](docs/format_bench.json), 1000 keys): B-tree **2,457,600** bytes vs JSONL **32,890** bytes (`btree_over_jsonl_size` **74.72**). 4KB pages plus `MAX_LEAF_KEYS = 3` leave a lot of unused space per leaf; JSONL is a dump, not a database. Re-run with `cargo run --release --bin bench_format`.

## Reproduce

```bash
cargo test --test integration_test
cargo run --release --bin bench_format
```
