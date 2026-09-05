# BTreeDB

The recruiter-facing doc is the [README](../README.md). This page is only a boundary note.

## What this is

An on-disk B-tree key-value store in Rust: 4KB pages, binary leaf/internal nodes, insert/get/delete with split and borrow/merge, range cursors, and WAL-first crash recovery. Each `insert`/`delete` is one CRC32 WAL frame; reopen replays complete frames and ignores a torn last frame.

On-disk format is **v2**. Older files (and wrapping-sum WALs) fail to open. There is no migrator.

## What this is not

Not a SQL engine. Not multi-client. Not transactions, compression, or concurrency — those files compile only with `--features experimental` and are not on the write path.

A longer generated product spec (personas, P0 tables, phase checklists) is archived at [archive/PRD.md](archive/PRD.md).
