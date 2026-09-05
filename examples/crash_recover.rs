//! Show WAL replay and a torn last frame. Prints the story; not just a test name.
//!
//!   cargo run --release --example crash_recover

use btreedb::btree::BTree;
use btreedb::pager::{Pager, PAGE_SIZE};
use btreedb::wal::{WalFrame, WAL, WAL_HEADER_SIZE};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
use std::process;

fn main() -> io::Result<()> {
    let dir = std::env::temp_dir().join("btreedb_crash_demo");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;

    recover_zeroed_leaf(&dir.join("unsync.db"))?;
    println!();
    ignore_torn_frame(&dir.join("torn.db"))?;
    Ok(())
}

fn recover_zeroed_leaf(db_path: &Path) -> io::Result<()> {
    println!("== 1. unsynced insert, then a zeroed leaf ==");
    {
        let mut btree = BTree::open(db_path)?;
        btree.insert("alpha", "one")?;
        println!("inserted alpha=one (no BTree::sync; WAL still has the page)");
        drop(btree);
    }

    {
        let file = OpenOptions::new().read(true).write(true).open(db_path)?;
        let mut pager = Pager::new(file);
        pager.write_page(1, &[0u8; PAGE_SIZE])?;
        println!("zeroed leaf page 1 in the DB file (Pager::new, no WAL)");
    }

    let mut btree = BTree::open(db_path)?;
    match btree.get("alpha")? {
        Some(v) => println!("reopen recovered alpha -> {v}"),
        None => {
            eprintln!("error: alpha missing after WAL replay");
            process::exit(1);
        }
    }
    Ok(())
}

fn ignore_torn_frame(db_path: &Path) -> io::Result<()> {
    println!("== 2. torn last WAL frame is ignored ==");
    {
        let mut btree = BTree::open(db_path)?;
        for i in 0..80 {
            btree.insert(&format!("key_{i:04}"), &format!("value_{i}"))?;
        }
        btree.sync()?;
        println!("synced 80 keys (checkpointed WAL)");
    }

    let snapshot = fs::read(db_path)?;
    let wal_path = WAL::wal_path(db_path);

    let split_key = {
        let mut btree = BTree::open(db_path)?;
        let before = btree.stats()?.leaf_count;
        let mut n = 80usize;
        loop {
            btree.insert(&format!("key_{n:04}"), &format!("value_{n}"))?;
            n += 1;
            if btree.stats()?.leaf_count > before {
                break;
            }
            if n > 480 {
                eprintln!("error: expected a leaf split");
                process::exit(1);
            }
        }
        println!("inserted through a leaf split (last key key_{:04})", n - 1);
        format!("key_{:04}", n - 1)
    };

    let wal_bytes = fs::read(&wal_path)?;
    let last_page_count = {
        let mut wal = WAL::open(db_path)?;
        let frames = wal.read_frames()?;
        let last = frames.last().expect("last frame");
        println!(
            "last WAL frame has {} pages (split group)",
            last.pages.len()
        );
        last.pages.len()
    };
    let last_len = WalFrame::encoded_len(last_page_count) as usize;
    let last_start = wal_bytes.len() - last_len;
    let torn_end = last_start + 20;

    fs::write(db_path, snapshot)?;
    let mut torn = wal_bytes[..WAL_HEADER_SIZE].to_vec();
    torn.extend_from_slice(&wal_bytes[last_start..torn_end]);
    fs::write(&wal_path, &torn)?;
    println!(
        "restored pre-split DB; truncated last WAL frame {} -> {} bytes",
        wal_bytes.len(),
        torn.len()
    );

    let mut btree = BTree::open(db_path)?;
    match btree.get("key_0000")? {
        Some(v) => println!("snapshot key_0000 -> {v} (kept)"),
        None => {
            eprintln!("error: snapshot key missing");
            process::exit(1);
        }
    }
    match btree.get(&split_key)? {
        None => println!("{split_key} -> (nil) (torn split frame ignored)"),
        Some(_) => {
            eprintln!("error: torn frame must not apply a prefix");
            process::exit(1);
        }
    }
    Ok(())
}
