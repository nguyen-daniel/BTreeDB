//! Insert fat values until the tree splits, dump it, reopen, and prove a key persists.
//!
//!   cargo run --release --example split_demo

use btreedb::btree::BTree;
use std::fs;
use std::path::PathBuf;
use std::process;

fn main() -> std::io::Result<()> {
    let dir = std::env::temp_dir().join("btreedb_split_demo");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    let db_path: PathBuf = dir.join("demo.db");

    const N: usize = 40;
    let value = "x".repeat(200);

    println!("== insert {N} keys with 200-byte values ==");
    {
        let mut btree = BTree::open(&db_path)?;
        for i in 1..=N {
            let key = format!("key{i:03}");
            btree.insert(&key, &value)?;
        }

        let stats = btree.stats()?;
        println!("Database Statistics:");
        println!("  Keys:           {}", stats.key_count);
        println!("  Tree Height:    {}", stats.tree_height);
        println!("  Total Pages:    {}", stats.page_count);
        println!("  Leaf Nodes:     {}", stats.leaf_count);
        println!("  Internal Nodes: {}", stats.internal_count);
        println!();
        println!("Tree Structure:");
        print!("{}", btree.dump_tree()?);

        if stats.tree_height < 2 || stats.leaf_count < 2 {
            eprintln!("error: expected a split (height >= 2, more than one leaf)");
            process::exit(1);
        }

        btree.sync()?;
        println!("synced and closed");
    }

    println!();
    println!("== reopen {} ==", db_path.display());
    {
        let mut btree = BTree::open(&db_path)?;
        for key in ["key001", "key040"] {
            match btree.get(key)? {
                Some(v) if v.len() == 200 => println!("get {key} -> 200-byte value (persisted)"),
                other => {
                    eprintln!("error: expected persisted {key}, got {other:?}");
                    process::exit(1);
                }
            }
        }
        let stats = btree.stats()?;
        println!(
            "reopen stats: keys={} height={} leaves={}",
            stats.key_count, stats.tree_height, stats.leaf_count
        );
    }

    Ok(())
}
