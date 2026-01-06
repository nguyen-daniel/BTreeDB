use crate::node::Node;
use crate::pager::Pager;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Read, Write};

const MAX_LEAF_KEYS: usize = 3; // Reduced to 3 to support 1KB values (1024 bytes) in 4KB pages
const MAX_INTERNAL_KEYS: usize = 10; // Maximum keys in an internal node
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

    /// Creates a new BTree with the given Pager.
    /// Reads the header from page 0 to find the root page ID.
    /// If the header doesn't exist or is invalid, creates a new database.
    pub fn new(mut pager: Pager) -> io::Result<Self> {
        // Try to read the header
        match Self::read_header(&mut pager) {
            Ok(header) => {
                // Existing database, use the root from header
                // Derive next_page_id from actual file size to prevent page overwrites
                let page_count = pager.page_count()?;
                let next_page_id = page_count.max(2); // At minimum, page 0 (header) and page 1 (root) exist

                Ok(BTree {
                    pager,
                    root_page_id: header.root_page_id,
                    next_page_id,
                })
            }
            Err(_) => {
                // New database, create header and initial root
                let root_page_id = 1; // Root starts at page 1 (page 0 is for header)
                let next_page_id = 2;

                // Create empty root leaf at page 1
                let empty_leaf = Node::new_leaf(Vec::new());
                let buffer = empty_leaf.serialize()?;
                pager.write_page(root_page_id, &buffer)?;

                // Write the header
                Self::write_header(&mut pager, root_page_id)?;

                Ok(BTree {
                    pager,
                    root_page_id,
                    next_page_id,
                })
            }
        }
    }

    /// Gets the root page ID.
    pub fn root_page_id(&self) -> u32 {
        self.root_page_id
    }

    /// Syncs all data to disk by flushing the underlying file.
    pub fn sync(&mut self) -> io::Result<()> {
        self.pager.file_mut().sync_all()
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
}
