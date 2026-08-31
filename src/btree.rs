use crate::node::Node;
use crate::pager::Pager;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Read, Write};
use std::path::Path;

const MAX_LEAF_KEYS: usize = 3; // Reduced to 3 to support 1KB values (1024 bytes) in 4KB pages
const MAX_INTERNAL_KEYS: usize = 10; // Maximum keys in an internal node
/// Minimum keys in a non-root leaf: ceil(MAX_LEAF_KEYS / 2).
const MIN_LEAF_KEYS: usize = MAX_LEAF_KEYS.div_ceil(2);
/// Minimum keys in a non-root internal node: ceil(MAX_INTERNAL_KEYS / 2).
const MIN_INTERNAL_KEYS: usize = MAX_INTERNAL_KEYS.div_ceil(2);
const HEADER_SIZE: usize = 100;
const MAGIC_BYTES: &[u8] = b"BTREEDB";
const MAGIC_BYTES_LEN: usize = 7;

/// Result of an insert operation that may cause a split.
enum InsertResult {
    /// No split occurred
    NoSplit,
    /// A split occurred, returning the separator key and new page ID
    Split {
        separator_key: String,
        new_page_id: u32,
    },
}

/// Result of a delete operation.
enum DeleteResult {
    /// Key was found and deleted; this node still meets occupancy (or is the root).
    Ok,
    /// Key was not found
    NotFound,
    /// Key was deleted and this non-root node is below minimum occupancy.
    Underflow,
}

/// Database header stored in the first 100 bytes of page 0.
struct DatabaseHeader {
    /// Magic bytes signature: "BTREEDB"
    magic: [u8; MAGIC_BYTES_LEN],
    /// Root page ID (u32, little-endian)
    root_page_id: u32,
    /// Reserved space for future use (100 - 7 - 4 = 89 bytes)
    _reserved: [u8; 89],
}

impl DatabaseHeader {
    /// Creates a new header with the given root page ID.
    fn new(root_page_id: u32) -> Self {
        let mut magic = [0u8; MAGIC_BYTES_LEN];
        magic.copy_from_slice(MAGIC_BYTES);
        DatabaseHeader {
            magic,
            root_page_id,
            _reserved: [0u8; 89],
        }
    }

    /// Serializes the header into a 100-byte buffer.
    fn serialize(&self) -> io::Result<[u8; HEADER_SIZE]> {
        let mut buffer = [0u8; HEADER_SIZE];
        let mut cursor = io::Cursor::new(&mut buffer[..]);

        // Write magic bytes
        cursor.write_all(&self.magic)?;

        // Write root_page_id (u32, little-endian)
        cursor.write_u32::<LittleEndian>(self.root_page_id)?;

        // Reserved space is already zero-padded
        Ok(buffer)
    }

    /// Deserializes a header from a 100-byte buffer.
    fn deserialize(buffer: &[u8; HEADER_SIZE]) -> io::Result<Self> {
        let mut cursor = io::Cursor::new(buffer);

        // Read magic bytes
        let mut magic = [0u8; MAGIC_BYTES_LEN];
        cursor.read_exact(&mut magic)?;

        // Verify magic bytes
        if magic != MAGIC_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Invalid magic bytes. Expected {:?}, got {:?}",
                    MAGIC_BYTES, magic
                ),
            ));
        }

        // Read root_page_id
        let root_page_id = cursor.read_u32::<LittleEndian>()?;

        Ok(DatabaseHeader {
            magic,
            root_page_id,
            _reserved: [0u8; 89],
        })
    }
}

/// B-Tree database structure that manages persistent storage via a Pager.
pub struct BTree {
    pager: Pager,
    root_page_id: u32,
    next_page_id: u32,
}

/// Database statistics returned by `BTree::stats()`.
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    /// Total number of keys in the database
    pub key_count: u64,
    /// Height of the B-Tree (1 = just root leaf)
    pub tree_height: u32,
    /// Total number of pages in the database file
    pub page_count: u32,
    /// Number of leaf nodes
    pub leaf_count: u32,
    /// Number of internal nodes
    pub internal_count: u32,
}

impl BTree {
    /// Reads the database header from page 0.
    fn read_header(pager: &mut Pager) -> io::Result<DatabaseHeader> {
        let page_buffer = pager.get_page(0)?;
        let header_buffer: [u8; HEADER_SIZE] = page_buffer[..HEADER_SIZE]
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Failed to extract header"))?;
        DatabaseHeader::deserialize(&header_buffer)
    }

    /// Writes the database header to page 0.
    fn write_header(pager: &mut Pager, root_page_id: u32) -> io::Result<()> {
        let header = DatabaseHeader::new(root_page_id);
        let header_buffer = header.serialize()?;

        // Read the current page 0
        let mut page_buffer = pager.get_page(0)?;

        // Write the header to the first 100 bytes
        page_buffer[..HEADER_SIZE].copy_from_slice(&header_buffer);

        // Write the entire page back
        pager.write_page(0, &page_buffer)
    }

    /// Opens a database file, recovers from the WAL if present, then logs writes.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::new(Pager::open(path)?)
    }

    /// Creates a new BTree with the given Pager.
    /// Reads the header from page 0 to find the root page ID.
    /// Empty or missing files are initialized as a new database.
    /// Existing files with invalid magic or a corrupt header fail to open
    /// and are not overwritten.
    pub fn new(mut pager: Pager) -> io::Result<Self> {
        if pager.page_count()? == 0 {
            return Self::initialize(pager);
        }

        let header = Self::read_header(&mut pager)?;
        // Derive next_page_id from actual file size to prevent page overwrites
        let page_count = pager.page_count()?;
        // At minimum, page 0 (header) and page 1 (root) exist
        let next_page_id = page_count.max(2);

        Ok(BTree {
            pager,
            root_page_id: header.root_page_id,
            next_page_id,
        })
    }

    /// Initializes an empty database: header on page 0, empty root leaf on page 1.
    fn initialize(mut pager: Pager) -> io::Result<Self> {
        let root_page_id = 1; // Root starts at page 1 (page 0 is for header)
        let next_page_id = 2;

        let empty_leaf = Node::new_leaf(Vec::new());
        let buffer = empty_leaf.serialize()?;
        pager.write_page(root_page_id, &buffer)?;

        Self::write_header(&mut pager, root_page_id)?;

        Ok(BTree {
            pager,
            root_page_id,
            next_page_id,
        })
    }

    /// Gets the root page ID.
    pub fn root_page_id(&self) -> u32 {
        self.root_page_id
    }

    /// Syncs all data to disk, then checkpoints the WAL.
    pub fn sync(&mut self) -> io::Result<()> {
        self.pager.file_mut().sync_all()?;
        self.pager.checkpoint()
    }

    /// Returns a mutable reference to the pager.
    /// Used by the cursor for tree traversal.
    pub fn pager(&mut self) -> &mut Pager {
        &mut self.pager
    }

    /// Computes and returns database statistics.
    pub fn stats(&mut self) -> io::Result<DatabaseStats> {
        let mut stats = DatabaseStats {
            key_count: 0,
            tree_height: 0,
            page_count: self.pager.page_count()?,
            leaf_count: 0,
            internal_count: 0,
        };

        self.collect_stats(self.root_page_id, 1, &mut stats)?;
        Ok(stats)
    }

    /// Recursively collects statistics from the tree.
    fn collect_stats(
        &mut self,
        page_id: u32,
        depth: u32,
        stats: &mut DatabaseStats,
    ) -> io::Result<()> {
        // Update tree height
        if depth > stats.tree_height {
            stats.tree_height = depth;
        }

        let page_buffer = self.pager.get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { pairs, .. } => {
                stats.leaf_count += 1;
                stats.key_count += pairs.len() as u64;
            }
            Node::Internal { children, .. } => {
                stats.internal_count += 1;
                for child_id in children {
                    self.collect_stats(child_id, depth + 1, stats)?;
                }
            }
        }

        Ok(())
    }

    /// Generates a text visualization of the tree structure.
    pub fn dump_tree(&mut self) -> io::Result<String> {
        let mut output = String::new();
        self.dump_node(self.root_page_id, 0, &mut output)?;
        Ok(output)
    }

    /// Recursively dumps a node and its children.
    fn dump_node(&mut self, page_id: u32, indent: usize, output: &mut String) -> io::Result<()> {
        let page_buffer = self.pager.get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        let prefix = "  ".repeat(indent);

        match node {
            Node::Leaf { pairs, .. } => {
                output.push_str(&format!(
                    "{}[Leaf@{}] {} keys: ",
                    prefix,
                    page_id,
                    pairs.len()
                ));
                let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
                if keys.len() <= 5 {
                    output.push_str(&keys.join(", "));
                } else {
                    output.push_str(&format!(
                        "{}, {}, ... {}",
                        keys[0],
                        keys[1],
                        keys[keys.len() - 1]
                    ));
                }
                output.push('\n');
            }
            Node::Internal { keys, children, .. } => {
                output.push_str(&format!(
                    "{}[Internal@{}] {} keys: ",
                    prefix,
                    page_id,
                    keys.len()
                ));
                if keys.len() <= 5 {
                    output.push_str(&keys.join(", "));
                } else {
                    output.push_str(&format!(
                        "{}, {}, ... {}",
                        keys[0],
                        keys[1],
                        keys[keys.len() - 1]
                    ));
                }
                output.push('\n');

                for child_id in children {
                    self.dump_node(child_id, indent + 1, output)?;
                }
            }
        }

        Ok(())
    }

    /// Retrieves a value by key from the B-Tree.
    /// Returns Some(value) if found, None if not found.
    pub fn get(&mut self, key: &str) -> io::Result<Option<String>> {
        self.search(self.root_page_id, key)
    }

    /// Recursively searches for a key starting from the given page_id.
    /// Returns Some(value) if found, None if not found.
    fn search(&mut self, page_id: u32, key: &str) -> io::Result<Option<String>> {
        // Fetch the page via pager
        let page_buffer = self.pager.get_page(page_id)?;

        // Deserialize the node
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { pairs, .. } => {
                // Search for the key in the leaf node
                for (k, v) in pairs {
                    if k == key {
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }
            Node::Internal { keys, children, .. } => {
                // Find the child page ID whose key range contains our target
                let child_index = Self::find_child_index(&keys, key);
                let child_page_id = children[child_index];

                // Recurse into the child
                self.search(child_page_id, key)
            }
        }
    }

    /// Finds the index of the child page that should contain the given key.
    /// For Internal nodes: keys[i] separates children[i] and children[i+1].
    /// - If key < keys[0], return 0 (go to children[0])
    /// - If key >= keys[i] and key < keys[i+1], return i+1
    /// - If key >= keys[n-1], return n (go to children[n])
    fn find_child_index(keys: &[String], key: &str) -> usize {
        for (i, k) in keys.iter().enumerate() {
            if key < k {
                return i;
            }
        }
        // Key is >= all keys, so go to the rightmost child
        keys.len()
    }

    /// Inserts a key-value pair into the B-Tree.
    pub fn insert(&mut self, key: &str, value: &str) -> io::Result<()> {
        let result = self.insert_recursive(self.root_page_id, key, value)?;

        match result {
            InsertResult::NoSplit => Ok(()),
            InsertResult::Split {
                separator_key,
                new_page_id,
            } => {
                // Root was split, create a new root
                self.create_new_root(self.root_page_id, separator_key, new_page_id)
            }
        }
    }

    /// Recursively inserts a key-value pair into the tree.
    /// Returns InsertResult indicating if a split occurred.
    fn insert_recursive(
        &mut self,
        page_id: u32,
        key: &str,
        value: &str,
    ) -> io::Result<InsertResult> {
        let page_buffer = self.pager.get_page(page_id)?;
        let node = Node::deserialize(&page_buffer)?;

        match node {
            Node::Leaf { mut pairs, .. } => {
                // Check if key already exists (update value)
                for (k, v) in pairs.iter_mut() {
                    if k == key {
                        *v = value.to_string();
                        let updated_node = Node::new_leaf(pairs);
                        let buffer = updated_node.serialize()?;
                        self.pager.write_page(page_id, &buffer)?;
                        return Ok(InsertResult::NoSplit);
                    }
                }

                // Insert the new key-value pair in sorted order
                let insert_pos = pairs
                    .binary_search_by(|(k, _)| k.as_str().cmp(key))
                    .unwrap_or_else(|pos| pos);
                pairs.insert(insert_pos, (key.to_string(), value.to_string()));

                // Check if we need to split
                if pairs.len() > MAX_LEAF_KEYS {
                    let split_result = self.split_leaf(page_id, pairs)?;
                    Ok(split_result)
                } else {
                    // Update the leaf node
                    let updated_node = Node::new_leaf(pairs);
                    let buffer = updated_node.serialize()?;
                    self.pager.write_page(page_id, &buffer)?;
                    Ok(InsertResult::NoSplit)
                }
            }
            Node::Internal {
                mut keys,
                mut children,
                ..
            } => {
                // Find the child to insert into
                let child_index = Self::find_child_index(&keys, key);
                let child_page_id = children[child_index];

                // Recursively insert into the child
                let result = self.insert_recursive(child_page_id, key, value)?;

                match result {
                    InsertResult::NoSplit => {
                        // No split, just update this node if needed
                        let updated_node = Node::new_internal(keys, children);
                        let buffer = updated_node.serialize()?;
                        self.pager.write_page(page_id, &buffer)?;
                        Ok(InsertResult::NoSplit)
                    }
                    InsertResult::Split {
                        separator_key,
                        new_page_id,
                    } => {
                        // Child was split, insert the separator key and new child
                        let insert_pos = keys
                            .binary_search_by(|k| k.as_str().cmp(separator_key.as_str()))
                            .unwrap_or_else(|pos| pos);
                        keys.insert(insert_pos, separator_key);
                        children.insert(insert_pos + 1, new_page_id);

                        // Check if we need to split the internal node
                        if keys.len() > MAX_INTERNAL_KEYS {
                            let split_result = self.split_internal(page_id, keys, children)?;
                            Ok(split_result)
                        } else {
                            // Update the internal node
                            let updated_node = Node::new_internal(keys, children);
                            let buffer = updated_node.serialize()?;
                            self.pager.write_page(page_id, &buffer)?;
                            Ok(InsertResult::NoSplit)
                        }
                    }
                }
            }
        }
    }

    /// Splits a leaf node that has exceeded MAX_LEAF_KEYS.
    /// Moves half the keys to a new leaf node.
    /// Returns the separator key (first key of the new node) and the new page ID.
    fn split_leaf(
        &mut self,
        page_id: u32,
        pairs: Vec<(String, String)>,
    ) -> io::Result<InsertResult> {
        let split_point = pairs.len() / 2;
        let (left_pairs, right_pairs) = pairs.split_at(split_point);

        // Create new leaf node with the right half
        let new_leaf = Node::new_leaf(right_pairs.to_vec());
        let new_page_id = self.next_page_id;
        self.next_page_id += 1;

        let new_buffer = new_leaf.serialize()?;
        self.pager.write_page(new_page_id, &new_buffer)?;

        // Update the original leaf with the left half
        let updated_leaf = Node::new_leaf(left_pairs.to_vec());
        let updated_buffer = updated_leaf.serialize()?;
        self.pager.write_page(page_id, &updated_buffer)?;

        // The separator key is the first key of the new (right) node
        let separator_key = right_pairs[0].0.clone();

        Ok(InsertResult::Split {
            separator_key,
            new_page_id,
        })
    }

    /// Splits an internal node that has exceeded MAX_INTERNAL_KEYS.
    /// Moves half the keys and children to a new internal node.
    /// Returns the separator key (middle key) and the new page ID.
    fn split_internal(
        &mut self,
        page_id: u32,
        keys: Vec<String>,
        children: Vec<u32>,
    ) -> io::Result<InsertResult> {
        let split_point = keys.len() / 2;
        let separator_key = keys[split_point].clone();

        // Split keys: left gets keys[0..split_point], right gets keys[split_point+1..]
        let (left_keys, right_keys_with_sep) = keys.split_at(split_point);
        let right_keys = right_keys_with_sep[1..].to_vec();

        // Split children: left gets children[0..split_point+1], right gets children[split_point+1..]
        let (left_children, right_children) = children.split_at(split_point + 1);

        // Create new internal node with the right half
        let new_internal = Node::new_internal(right_keys, right_children.to_vec());
        let new_page_id = self.next_page_id;
        self.next_page_id += 1;

        let new_buffer = new_internal.serialize()?;
        self.pager.write_page(new_page_id, &new_buffer)?;

        // Update the original internal node with the left half
        let updated_internal = Node::new_internal(left_keys.to_vec(), left_children.to_vec());
        let updated_buffer = updated_internal.serialize()?;
        self.pager.write_page(page_id, &updated_buffer)?;

        Ok(InsertResult::Split {
            separator_key,
            new_page_id,
        })
    }

    /// Creates a new root node when the old root is split.
    fn create_new_root(
        &mut self,
        left_child_id: u32,
        separator_key: String,
        right_child_id: u32,
    ) -> io::Result<()> {
        let new_root = Node::new_internal(vec![separator_key], vec![left_child_id, right_child_id]);

        let new_root_page_id = self.next_page_id;
        self.next_page_id += 1;

        let buffer = new_root.serialize()?;
        self.pager.write_page(new_root_page_id, &buffer)?;

        self.root_page_id = new_root_page_id;

        // Update the header with the new root page ID
        Self::write_header(&mut self.pager, new_root_page_id)
    }

    /// Deletes a key from the B-Tree.
    /// Returns true if the key was found and deleted, false if not found.
    /// After a non-root node drops below `ceil(MAX_*_KEYS / 2)`, borrows from a
    /// sibling or merges so the tree stays balanced.
    pub fn delete(&mut self, key: &str) -> io::Result<bool> {
        let result = self.delete_recursive(self.root_page_id, key)?;

        match result {
            DeleteResult::NotFound => Ok(false),
            DeleteResult::Ok | DeleteResult::Underflow => {
                self.handle_root_demotion()?;
                Ok(true)
            }
        }
    }

    /// Leaf occupancies in left-to-right order: `(page_id, key_count)`.
    pub fn leaf_occupancies(&mut self) -> io::Result<Vec<(u32, usize)>> {
        let mut out = Vec::new();
        self.collect_leaf_occupancies(self.root_page_id, &mut out)?;
        Ok(out)
    }

    fn collect_leaf_occupancies(
        &mut self,
        page_id: u32,
        out: &mut Vec<(u32, usize)>,
    ) -> io::Result<()> {
        let node = self.load_node(page_id)?;
        match node {
            Node::Leaf { pairs, .. } => {
                out.push((page_id, pairs.len()));
            }
            Node::Internal { children, .. } => {
                for child_id in children {
                    self.collect_leaf_occupancies(child_id, out)?;
                }
            }
        }
        Ok(())
    }

    fn load_node(&mut self, page_id: u32) -> io::Result<Node> {
        let page_buffer = self.pager.get_page(page_id)?;
        Node::deserialize(&page_buffer)
    }

    fn store_node(&mut self, page_id: u32, node: &Node) -> io::Result<()> {
        let buffer = node.serialize()?;
        self.pager.write_page(page_id, &buffer)
    }

    /// Handles root demotion when root becomes empty or has only one child.
    fn handle_root_demotion(&mut self) -> io::Result<()> {
        loop {
            let node = self.load_node(self.root_page_id)?;
            match node {
                Node::Internal { children, keys, .. } => {
                    if keys.is_empty() && children.len() == 1 {
                        self.root_page_id = children[0];
                        Self::write_header(&mut self.pager, self.root_page_id)?;
                    } else {
                        break;
                    }
                }
                Node::Leaf { .. } => break,
            }
        }
        Ok(())
    }

    /// Recursively deletes a key starting at `page_id`, then rebalances.
    fn delete_recursive(&mut self, page_id: u32, key: &str) -> io::Result<DeleteResult> {
        let node = self.load_node(page_id)?;

        match node {
            Node::Leaf { mut pairs, .. } => {
                let pos = pairs.iter().position(|(k, _)| k == key);
                match pos {
                    Some(idx) => {
                        pairs.remove(idx);
                        let underfull = page_id != self.root_page_id && pairs.len() < MIN_LEAF_KEYS;
                        self.store_node(page_id, &Node::new_leaf(pairs))?;
                        if underfull {
                            Ok(DeleteResult::Underflow)
                        } else {
                            Ok(DeleteResult::Ok)
                        }
                    }
                    None => Ok(DeleteResult::NotFound),
                }
            }
            Node::Internal {
                mut keys,
                mut children,
                ..
            } => {
                let child_index = Self::find_child_index(&keys, key);
                let child_page_id = children[child_index];
                let result = self.delete_recursive(child_page_id, key)?;
                match result {
                    DeleteResult::NotFound => Ok(DeleteResult::NotFound),
                    DeleteResult::Ok => Ok(DeleteResult::Ok),
                    DeleteResult::Underflow => {
                        self.rebalance_child(&mut keys, &mut children, child_index)?;
                        let underfull =
                            page_id != self.root_page_id && keys.len() < MIN_INTERNAL_KEYS;
                        self.store_node(page_id, &Node::new_internal(keys, children))?;
                        if underfull {
                            Ok(DeleteResult::Underflow)
                        } else {
                            Ok(DeleteResult::Ok)
                        }
                    }
                }
            }
        }
    }

    fn rebalance_child(
        &mut self,
        parent_keys: &mut Vec<String>,
        parent_children: &mut Vec<u32>,
        child_index: usize,
    ) -> io::Result<()> {
        if parent_children.len() < 2 {
            return Ok(());
        }

        let child = self.load_node(parent_children[child_index])?;
        match child {
            Node::Leaf { .. } => {
                self.rebalance_leaf_child(parent_keys, parent_children, child_index)
            }
            Node::Internal { .. } => {
                self.rebalance_internal_child(parent_keys, parent_children, child_index)
            }
        }
    }

    fn sibling_key_count(&mut self, page_id: u32) -> io::Result<usize> {
        Ok(match self.load_node(page_id)? {
            Node::Leaf { pairs, .. } => pairs.len(),
            Node::Internal { keys, .. } => keys.len(),
        })
    }

    fn rebalance_leaf_child(
        &mut self,
        parent_keys: &mut Vec<String>,
        parent_children: &mut Vec<u32>,
        child_index: usize,
    ) -> io::Result<()> {
        let child_id = parent_children[child_index];
        let has_left = child_index > 0;
        let has_right = child_index + 1 < parent_children.len();

        if has_left {
            let left_id = parent_children[child_index - 1];
            if self.sibling_key_count(left_id)? > MIN_LEAF_KEYS {
                return self.borrow_leaf_from_left(parent_keys, child_index, left_id, child_id);
            }
        }
        if has_right {
            let right_id = parent_children[child_index + 1];
            if self.sibling_key_count(right_id)? > MIN_LEAF_KEYS {
                return self.borrow_leaf_from_right(parent_keys, child_index, child_id, right_id);
            }
        }
        if has_left {
            let left_id = parent_children[child_index - 1];
            return self.merge_leaves(
                parent_keys,
                parent_children,
                child_index - 1,
                left_id,
                child_id,
            );
        }
        if has_right {
            let right_id = parent_children[child_index + 1];
            return self.merge_leaves(
                parent_keys,
                parent_children,
                child_index,
                child_id,
                right_id,
            );
        }
        Ok(())
    }

    fn borrow_leaf_from_left(
        &mut self,
        parent_keys: &mut [String],
        child_index: usize,
        left_id: u32,
        child_id: u32,
    ) -> io::Result<()> {
        let (mut left_pairs, mut child_pairs) =
            match (self.load_node(left_id)?, self.load_node(child_id)?) {
                (Node::Leaf { pairs: left, .. }, Node::Leaf { pairs: child, .. }) => (left, child),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected leaf siblings when borrowing from left",
                    ));
                }
            };
        let stolen = left_pairs.pop().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "left sibling has no key to lend",
            )
        })?;
        child_pairs.insert(0, stolen);
        parent_keys[child_index - 1] = child_pairs[0].0.clone();
        self.store_node(left_id, &Node::new_leaf(left_pairs))?;
        self.store_node(child_id, &Node::new_leaf(child_pairs))?;
        Ok(())
    }

    fn borrow_leaf_from_right(
        &mut self,
        parent_keys: &mut [String],
        child_index: usize,
        child_id: u32,
        right_id: u32,
    ) -> io::Result<()> {
        let (mut child_pairs, mut right_pairs) =
            match (self.load_node(child_id)?, self.load_node(right_id)?) {
                (Node::Leaf { pairs: child, .. }, Node::Leaf { pairs: right, .. }) => {
                    (child, right)
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected leaf siblings when borrowing from right",
                    ));
                }
            };
        if right_pairs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "right sibling has no key to lend",
            ));
        }
        let stolen = right_pairs.remove(0);
        child_pairs.push(stolen);
        parent_keys[child_index] = right_pairs[0].0.clone();
        self.store_node(child_id, &Node::new_leaf(child_pairs))?;
        self.store_node(right_id, &Node::new_leaf(right_pairs))?;
        Ok(())
    }

    fn merge_leaves(
        &mut self,
        parent_keys: &mut Vec<String>,
        parent_children: &mut Vec<u32>,
        left_index: usize,
        left_id: u32,
        right_id: u32,
    ) -> io::Result<()> {
        let (mut left_pairs, right_pairs) =
            match (self.load_node(left_id)?, self.load_node(right_id)?) {
                (Node::Leaf { pairs: left, .. }, Node::Leaf { pairs: right, .. }) => (left, right),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected leaf siblings when merging",
                    ));
                }
            };
        left_pairs.extend(right_pairs);
        self.store_node(left_id, &Node::new_leaf(left_pairs))?;
        parent_keys.remove(left_index);
        parent_children.remove(left_index + 1);
        Ok(())
    }

    fn rebalance_internal_child(
        &mut self,
        parent_keys: &mut Vec<String>,
        parent_children: &mut Vec<u32>,
        child_index: usize,
    ) -> io::Result<()> {
        let child_id = parent_children[child_index];
        let has_left = child_index > 0;
        let has_right = child_index + 1 < parent_children.len();

        if has_left {
            let left_id = parent_children[child_index - 1];
            if self.sibling_key_count(left_id)? > MIN_INTERNAL_KEYS {
                return self.borrow_internal_from_left(parent_keys, child_index, left_id, child_id);
            }
        }
        if has_right {
            let right_id = parent_children[child_index + 1];
            if self.sibling_key_count(right_id)? > MIN_INTERNAL_KEYS {
                return self.borrow_internal_from_right(
                    parent_keys,
                    child_index,
                    child_id,
                    right_id,
                );
            }
        }
        if has_left {
            let left_id = parent_children[child_index - 1];
            return self.merge_internals(
                parent_keys,
                parent_children,
                child_index - 1,
                left_id,
                child_id,
            );
        }
        if has_right {
            let right_id = parent_children[child_index + 1];
            return self.merge_internals(
                parent_keys,
                parent_children,
                child_index,
                child_id,
                right_id,
            );
        }
        Ok(())
    }

    fn borrow_internal_from_left(
        &mut self,
        parent_keys: &mut [String],
        child_index: usize,
        left_id: u32,
        child_id: u32,
    ) -> io::Result<()> {
        let (mut left_keys, mut left_children, mut child_keys, mut child_children) =
            match (self.load_node(left_id)?, self.load_node(child_id)?) {
                (
                    Node::Internal {
                        keys: lk,
                        children: lc,
                        ..
                    },
                    Node::Internal {
                        keys: ck,
                        children: cc,
                        ..
                    },
                ) => (lk, lc, ck, cc),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected internal siblings when borrowing from left",
                    ));
                }
            };
        let stolen_key = left_keys.pop().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "left sibling has no key to lend",
            )
        })?;
        let stolen_child = left_children.pop().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "left sibling has no child to lend",
            )
        })?;
        let sep = std::mem::replace(&mut parent_keys[child_index - 1], stolen_key);
        child_keys.insert(0, sep);
        child_children.insert(0, stolen_child);
        self.store_node(left_id, &Node::new_internal(left_keys, left_children))?;
        self.store_node(child_id, &Node::new_internal(child_keys, child_children))?;
        Ok(())
    }

    fn borrow_internal_from_right(
        &mut self,
        parent_keys: &mut [String],
        child_index: usize,
        child_id: u32,
        right_id: u32,
    ) -> io::Result<()> {
        let (mut child_keys, mut child_children, mut right_keys, mut right_children) =
            match (self.load_node(child_id)?, self.load_node(right_id)?) {
                (
                    Node::Internal {
                        keys: ck,
                        children: cc,
                        ..
                    },
                    Node::Internal {
                        keys: rk,
                        children: rc,
                        ..
                    },
                ) => (ck, cc, rk, rc),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected internal siblings when borrowing from right",
                    ));
                }
            };
        if right_keys.is_empty() || right_children.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "right sibling has no key/child to lend",
            ));
        }
        let stolen_key = right_keys.remove(0);
        let stolen_child = right_children.remove(0);
        let sep = std::mem::replace(&mut parent_keys[child_index], stolen_key);
        child_keys.push(sep);
        child_children.push(stolen_child);
        self.store_node(child_id, &Node::new_internal(child_keys, child_children))?;
        self.store_node(right_id, &Node::new_internal(right_keys, right_children))?;
        Ok(())
    }

    fn merge_internals(
        &mut self,
        parent_keys: &mut Vec<String>,
        parent_children: &mut Vec<u32>,
        left_index: usize,
        left_id: u32,
        right_id: u32,
    ) -> io::Result<()> {
        let (mut left_keys, mut left_children, right_keys, right_children) =
            match (self.load_node(left_id)?, self.load_node(right_id)?) {
                (
                    Node::Internal {
                        keys: lk,
                        children: lc,
                        ..
                    },
                    Node::Internal {
                        keys: rk,
                        children: rc,
                        ..
                    },
                ) => (lk, lc, rk, rc),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected internal siblings when merging",
                    ));
                }
            };
        let sep = parent_keys.remove(left_index);
        left_keys.push(sep);
        left_keys.extend(right_keys);
        left_children.extend(right_children);
        self.store_node(left_id, &Node::new_internal(left_keys, left_children))?;
        parent_children.remove(left_index + 1);
        Ok(())
    }
}
