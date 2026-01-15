use btreedb::btree::BTree;
use btreedb::cursor::Cursor;
use rustyline::DefaultEditor;
use std::fs::OpenOptions;
use std::io;

fn main() -> io::Result<()> {
    // Open or create the database file
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open("btree.db")?;

    // Create the pager and BTree
    let pager = btreedb::pager::Pager::new(file);
    let mut btree = BTree::new(pager)?;

    // Create the REPL editor
    let mut rl = DefaultEditor::new()
        .map_err(|e| io::Error::other(format!("Failed to initialize REPL: {}", e)))?;

    println!("B-Tree Database REPL");
    println!("Commands:");
    println!("  set <key> <value>  - Insert or update a key-value pair");
    println!("  get <key>          - Retrieve a value by key");
    println!("  delete <key>       - Delete a key-value pair");
    println!("  scan [start] [end] - Scan keys in range [start, end)");
    println!("  .stats             - Show database statistics");
    println!("  .dump              - Dump tree structure");
    println!("  .exit              - Exit and flush all data to disk");
    println!();

    loop {
        let readline = rl.readline("btreedb> ");
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Handle dot commands
                if line == ".exit" {
                    break;
                }

                if line == ".stats" {
                    match btree.stats() {
                        Ok(stats) => {
                            println!("Database Statistics:");
                            println!("  Keys:           {}", stats.key_count);
                            println!("  Tree Height:    {}", stats.tree_height);
                            println!("  Total Pages:    {}", stats.page_count);
                            println!("  Leaf Nodes:     {}", stats.leaf_count);
                            println!("  Internal Nodes: {}", stats.internal_count);
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                    continue;
                }

                if line == ".dump" {
                    match btree.dump_tree() {
                        Ok(output) => {
                            println!("Tree Structure:");
                            print!("{}", output);
                        }
                        Err(e) => println!("Error: {}", e),
                    }
                    continue;
                }

                // Parse the command
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                match parts[0] {
                    "set" => {
                        if parts.len() < 3 {
                            println!("Error: Usage: set <key> <value>");
                            continue;
                        }
                        let key = parts[1];
                        let value = parts[2..].join(" "); // Handle values with spaces

                        match btree.insert(key, &value) {
                            Ok(_) => println!("OK"),
                            Err(e) => println!("Error: {}", e),
                        }
                    }
                    "get" => {
                        if parts.len() < 2 {
                            println!("Error: Usage: get <key>");
                            continue;
                        }
                        let key = parts[1];

                        match btree.get(key) {
                            Ok(Some(value)) => println!("{}", value),
                            Ok(None) => println!("(nil)"),
                            Err(e) => println!("Error: {}", e),
                        }
                    }
                    "delete" => {
                        if parts.len() < 2 {
                            println!("Error: Usage: delete <key>");
                            continue;
                        }
                        let key = parts[1];

                        match btree.delete(key) {
                            Ok(true) => println!("OK"),
                            Ok(false) => println!("(nil)"),
                            Err(e) => println!("Error: {}", e),
                        }
                    }
                    "scan" => {
                        // Parse optional start and end keys
                        let start_key = parts.get(1).copied();
                        let end_key = parts.get(2).copied();

                        match Cursor::scan_range(&mut btree, start_key, end_key) {
                            Ok(results) => {
                                if results.is_empty() {
                                    println!("(empty)");
                                } else {
                                    let count = results.len();
                                    for (key, value) in results {
                                        println!("{} -> {}", key, value);
                                    }
                                    println!("({} results)", count);
                                }
                            }
                            Err(e) => println!("Error: {}", e),
                        }
                    }
                    _ => {
                        println!(
                            "Unknown command: {}. Use 'set', 'get', 'delete', 'scan', or '.exit'",
                            parts[0]
                        );
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(e) => {
                println!("Error: {}", e);
                break;
            }
        }
    }

    // Sync all data to disk before exiting
    sync_and_exit(btree)?;

    Ok(())
}

fn sync_and_exit(mut btree: BTree) -> io::Result<()> {
    btree.sync()?;
    println!("All data flushed to disk. Goodbye!");
    Ok(())
}
