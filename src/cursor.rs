//! Cursor module for B-Tree traversal and range queries.
//!
//! Provides a `Cursor` struct for efficient iteration over key-value pairs
//! and range scanning capabilities.

use crate::btree::BTree;
use crate::node::Node;
use std::io;

/// A cursor for traversing the B-Tree.
///
/// The cursor maintains a position in the tree and supports:
/// - Seeking to a specific key
/// - Moving to the next/previous key-value pair
/// - Range scanning
pub struct Cursor<'a> {
    btree: &'a mut BTree,
    /// Stack of (page_id, index) pairs representing the path from root to current position
    path: Vec<(u32, usize)>,
    /// Whether the cursor is positioned at a valid entry
    valid: bool,
}

impl<'a> Cursor<'a> {
    /// Creates a new cursor for the given B-Tree.
    pub fn new(btree: &'a mut BTree) -> Self {
        Cursor {
            btree,
            path: Vec::new(),
            valid: false,
        }
    }

    /// Seeks to the first key >= the given key.
    /// If found, positions the cursor at that key and returns true.
    /// If no such key exists, returns false and the cursor becomes invalid.
    pub fn seek(&mut self, key: &str) -> io::Result<bool> {
        self.path.clear();
        self.valid = false;

        let root_id = self.btree.root_page_id();
        self.seek_recursive(root_id, key)
    }

    /// Recursively seeks to the first key >= target.
    fn seek_recursive(&mut self, page_id: u32, key: &str) -> io::Result<bool> {
        let page_buffer = self.btree.pager().get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { pairs, .. } => {
                // Find the first key >= target
                for (i, (k, _)) in pairs.iter().enumerate() {
                    if k.as_str() >= key {
                        self.path.push((page_id, i));
                        self.valid = true;
                        return Ok(true);
                    }
                }
                // No key >= target in this leaf
                // Position at the end of this leaf (invalid position for iteration)
                self.path.push((page_id, pairs.len()));
                self.valid = false;
                Ok(false)
            }
            Node::Internal { keys, children, .. } => {
                // Find the child that might contain the key
                let mut child_index = children.len() - 1;
                for (i, k) in keys.iter().enumerate() {
                    if key < k.as_str() {
                        child_index = i;
                        break;
                    }
                }
                self.path.push((page_id, child_index));
                self.seek_recursive(children[child_index], key)
            }
        }
    }

    /// Seeks to the first (smallest) key in the tree.
    pub fn seek_first(&mut self) -> io::Result<bool> {
        self.path.clear();
        self.valid = false;

        let root_id = self.btree.root_page_id();
        self.seek_first_recursive(root_id)
    }

    /// Recursively seeks to the leftmost leaf.
    fn seek_first_recursive(&mut self, page_id: u32) -> io::Result<bool> {
        let page_buffer = self.btree.pager().get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { pairs, .. } => {
                if pairs.is_empty() {
                    self.valid = false;
                    Ok(false)
                } else {
                    self.path.push((page_id, 0));
                    self.valid = true;
                    Ok(true)
                }
            }
            Node::Internal { children, .. } => {
                self.path.push((page_id, 0));
                self.seek_first_recursive(children[0])
            }
        }
    }

    /// Returns true if the cursor is positioned at a valid entry.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Returns the current key-value pair if the cursor is valid.
    pub fn current(&mut self) -> io::Result<Option<(String, String)>> {
        if !self.valid {
            return Ok(None);
        }

        let (page_id, index) = *self.path.last().unwrap();
        let page_buffer = self.btree.pager().get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { pairs, .. } => {
                if index < pairs.len() {
                    Ok(Some(pairs[index].clone()))
                } else {
                    Ok(None)
                }
            }
            Node::Internal { .. } => {
                // Cursor should always point to a leaf
                Ok(None)
            }
        }
    }

    /// Moves the cursor to the next key-value pair.
    /// Returns true if successful, false if at the end.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> io::Result<bool> {
        if !self.valid {
            return Ok(false);
        }

        // Get current leaf position
        let (page_id, index) = *self.path.last().unwrap();
        let page_buffer = self.btree.pager().get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { pairs, .. } => {
                let next_index = index + 1;
                if next_index < pairs.len() {
                    // Move to next entry in same leaf
                    if let Some(last) = self.path.last_mut() {
                        last.1 = next_index;
                    }
                    Ok(true)
                } else {
                    // Need to move to next leaf
                    self.advance_to_next_leaf()
                }
            }
            Node::Internal { .. } => {
                self.valid = false;
                Ok(false)
            }
        }
    }

    /// Advances the cursor to the next leaf node.
    fn advance_to_next_leaf(&mut self) -> io::Result<bool> {
        // Pop the current leaf
        self.path.pop();

        // Walk up the tree until we find a node where we can go right
        while let Some((page_id, child_index)) = self.path.pop() {
            let page_buffer = self.btree.pager().get_page(page_id)?;
            let node = Node::deserialize(&page_buffer)?;

            match node {
                Node::Internal { children, .. } => {
                    let next_child_index = child_index + 1;
                    if next_child_index < children.len() {
                        // Move to next child
                        self.path.push((page_id, next_child_index));
                        // Go to leftmost leaf of this subtree
                        return self.seek_first_recursive(children[next_child_index]);
                    }
                    // Continue popping up
                }
                Node::Leaf { .. } => {
                    // Should not happen
                    break;
                }
            }
        }

        // Reached the end of the tree
        self.valid = false;
        Ok(false)
    }

    /// Scans all key-value pairs in the given range [start, end).
    /// Returns a vector of (key, value) pairs.
    pub fn scan_range(
        btree: &mut BTree,
        start_key: Option<&str>,
        end_key: Option<&str>,
    ) -> io::Result<Vec<(String, String)>> {
        let mut cursor = Cursor::new(btree);
        let mut results = Vec::new();

        // Position cursor at start
        let found = match start_key {
            Some(key) => cursor.seek(key)?,
            None => cursor.seek_first()?,
        };

        if !found {
            return Ok(results);
        }

        // Iterate until end
        loop {
            if !cursor.is_valid() {
                break;
            }

            if let Some((key, value)) = cursor.current()? {
                // Check end condition
                if let Some(end) = end_key {
                    if key.as_str() >= end {
                        break;
                    }
                }
                results.push((key, value));
            } else {
                break;
            }

            if !cursor.next()? {
                break;
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pager::Pager;
    use tempfile::NamedTempFile;

    fn create_test_btree() -> (BTree, tempfile::TempPath) {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let (file, path) = temp_file.into_parts();
        let pager = Pager::new(file);
        let btree = BTree::new(pager).expect("Failed to create BTree");
        (btree, path)
    }

    #[test]
    fn test_cursor_seek_and_next() {
        let (mut btree, _path) = create_test_btree();

        // Insert some keys
        for i in 0..10 {
            let key = format!("key_{:02}", i);
            let value = format!("value_{}", i);
            btree.insert(&key, &value).unwrap();
        }

        // Seek to key_05
        let mut cursor = Cursor::new(&mut btree);
        assert!(cursor.seek("key_05").unwrap());

        let (key, value) = cursor.current().unwrap().unwrap();
        assert_eq!(key, "key_05");
        assert_eq!(value, "value_5");

        // Move to next
        assert!(cursor.next().unwrap());
        let (key, _) = cursor.current().unwrap().unwrap();
        assert_eq!(key, "key_06");
    }

    #[test]
    fn test_cursor_scan_range() {
        let (mut btree, _path) = create_test_btree();

        // Insert keys
        for i in 0..20 {
            let key = format!("key_{:02}", i);
            let value = format!("value_{}", i);
            btree.insert(&key, &value).unwrap();
        }

        // Scan range [key_05, key_10)
        let results = Cursor::scan_range(&mut btree, Some("key_05"), Some("key_10")).unwrap();
        assert_eq!(results.len(), 5);
        assert_eq!(results[0].0, "key_05");
        assert_eq!(results[4].0, "key_09");
    }

    #[test]
    fn test_cursor_scan_all() {
        let (mut btree, _path) = create_test_btree();

        // Insert keys
        for i in 0..10 {
            let key = format!("key_{:02}", i);
            let value = format!("value_{}", i);
            btree.insert(&key, &value).unwrap();
        }

        // Scan all
        let results = Cursor::scan_range(&mut btree, None, None).unwrap();
        assert_eq!(results.len(), 10);
    }
}
