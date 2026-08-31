//! Binary 4KB pages vs JSON-lines for the same 1000 keys.
//! Writes measured sizes — does not assume a 60% I/O win.
//!
//!   cargo run --release --bin bench_format

use btreedb::btree::BTree;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

fn main() -> std::io::Result<()> {
    let dir = std::env::temp_dir().join("btreedb_format_bench");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    let db_path = dir.join("keys.db");
    let json_path = dir.join("keys.jsonl");

    const N: usize = 1000;
    let pairs: Vec<(String, String)> = (0..N)
        .map(|i| (format!("key_{:04}", i), format!("value_{}", i)))
        .collect();

    let t0 = Instant::now();
    {
        let mut btree = BTree::open(&db_path)?;
        for (k, v) in &pairs {
            btree.insert(k, v)?;
        }
        btree.sync()?;
    }
    let btree_ms = t0.elapsed().as_secs_f64() * 1e3;
    let btree_bytes = fs::metadata(&db_path)?.len();
    let wal_path = db_path.with_file_name("keys.db-wal");
    let wal_bytes = fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);

    let t1 = Instant::now();
    {
        let mut f = fs::File::create(&json_path)?;
        for (k, v) in &pairs {
            writeln!(f, "{{\"k\":\"{k}\",\"v\":\"{v}\"}}")?;
        }
        f.flush()?;
    }
    let json_ms = t1.elapsed().as_secs_f64() * 1e3;
    let json_bytes = fs::metadata(&json_path)?.len();

    let size_ratio = btree_bytes as f64 / json_bytes as f64;
    let size_reduction = 1.0 - size_ratio;

    let payload = format!(
        r#"{{
  "n_keys": {N},
  "btree_bytes": {btree_bytes},
  "wal_bytes_after_checkpoint": {wal_bytes},
  "jsonl_bytes": {json_bytes},
  "btree_write_ms": {btree_ms:.3},
  "jsonl_write_ms": {json_ms:.3},
  "btree_over_jsonl_size": {size_ratio:.4},
  "size_reduction_vs_jsonl": {size_reduction:.4},
  "resume_claim": "60% I/O overhead reduction — not asserted; quote this file",
  "note": "Fair comparison: same 1000 string KV pairs. B-tree uses 4KB pages (internal fragmentation). JSONL is a naive dump, not a database."
}}"#
    );

    let out_dir = Path::new("docs");
    fs::create_dir_all(out_dir)?;
    fs::write(out_dir.join("format_bench.json"), payload.as_bytes())?;
    println!("{payload}");
    println!("Wrote docs/format_bench.json");
    Ok(())
}
