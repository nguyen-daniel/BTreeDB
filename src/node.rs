use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Page size in bytes (4KB)
pub const PAGE_SIZE: usize = 4096;

/// Maximum allowed key length (prevents OOM from corrupted data)
/// Set to PAGE_SIZE - header overhead to be safe
const MAX_KEY_LEN: u32 = PAGE_SIZE as u32 - 16;

/// Maximum allowed value length (prevents OOM from corrupted data)
const MAX_VALUE_LEN: u32 = PAGE_SIZE as u32 - 16;

/// Maximum number of keys per node (prevents excessive allocations)
/// Internal nodes: MAX_INTERNAL_KEYS = 10, Leaf nodes: MAX_LEAF_KEYS = 3
/// We use a generous limit here for validation
const MAX_NUM_KEYS: u32 = 1000;

/// Node type identifier
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Leaf = 0,
    Internal = 1,
}

/// A B-Tree node that can be either an Internal node or a Leaf node.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// Internal node containing keys and child page IDs.
    /// For n keys, there are n+1 children (keys separate the children).
    Internal {
        /// Node type identifier
        node_type: NodeType,
        /// Number of keys in this node
        num_keys: u32,
        /// Keys stored in this node
        keys: Vec<String>,
        /// Page IDs of child nodes
        children: Vec<u32>,
    },
    /// Leaf node containing key-value pairs.
    Leaf {
        /// Node type identifier
        node_type: NodeType,
        /// Number of key-value pairs in this node
        num_keys: u32,
        /// Key-value pairs stored in this node
        pairs: Vec<(String, String)>,
    },
}

impl Node {
    /// Creates a new Internal node with the given keys and children.
    /// The number of children must be one more than the number of keys.
    pub fn new_internal(keys: Vec<String>, children: Vec<u32>) -> Self {
        assert_eq!(
            children.len(),
            keys.len() + 1,
            "Internal node must have exactly one more child than keys"
        );
        Node::Internal {
            node_type: NodeType::Internal,
            num_keys: keys.len() as u32,
            keys,
            children,
        }
    }

    /// Creates a new Leaf node with the given key-value pairs.
    pub fn new_leaf(pairs: Vec<(String, String)>) -> Self {
        Node::Leaf {
            node_type: NodeType::Leaf,
            num_keys: pairs.len() as u32,
            pairs,
        }
    }

    /// Returns the node type.
    pub fn node_type(&self) -> NodeType {
        match self {
            Node::Internal { node_type, .. } => *node_type,
            Node::Leaf { node_type, .. } => *node_type,
        }
    }

    /// Returns the number of keys in this node.
    pub fn num_keys(&self) -> u32 {
        match self {
            Node::Internal { num_keys, .. } => *num_keys,
            Node::Leaf { num_keys, .. } => *num_keys,
        }
    }

    /// Serializes the node into a 4096-byte buffer with zero-padding.
    /// Format:
    /// - Byte 0: node_type (0 = Leaf, 1 = Internal)
    /// - Bytes 1-4: num_keys (u32, little-endian)
    /// - For Leaf: key-value pairs (each: key_len, key_bytes, value_len, value_bytes)
    /// - For Internal: keys (each: key_len, key_bytes) followed by children (each: u32 page_id)
    /// - Rest: zero padding to PAGE_SIZE
    pub fn serialize(&self) -> Result<[u8; PAGE_SIZE], std::io::Error> {
        let mut buffer = [0u8; PAGE_SIZE];
        let mut cursor = std::io::Cursor::new(&mut buffer[..]);

        // Write node type (byte 0)
        cursor.write_u8(self.node_type() as u8)?;

        // Write num_keys (bytes 1-4)
        cursor.write_u32::<LittleEndian>(self.num_keys())?;

        match self {
            Node::Leaf { pairs, .. } => {
                // Serialize key-value pairs
                for (key, value) in pairs {
                    let key_bytes = key.as_bytes();
                    let value_bytes = value.as_bytes();

                    // Check if this pair would exceed page size
                    let pair_size = 4 + key_bytes.len() + 4 + value_bytes.len();
                    if cursor.position() as usize + pair_size > PAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Node data exceeds page size: {} bytes at position {}",
                                pair_size,
                                cursor.position()
                            ),
                        ));
                    }

                    // Write key length and key bytes
                    cursor.write_u32::<LittleEndian>(key_bytes.len() as u32)?;
                    cursor.write_all(key_bytes)?;

                    // Write value length and value bytes
                    cursor.write_u32::<LittleEndian>(value_bytes.len() as u32)?;
                    cursor.write_all(value_bytes)?;
                }
            }
            Node::Internal { keys, children, .. } => {
                // Serialize keys
                for key in keys {
                    let key_bytes = key.as_bytes();
                    let key_size = 4 + key_bytes.len();
                    if cursor.position() as usize + key_size > PAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Node data exceeds page size: {} bytes at position {}",
                                key_size,
                                cursor.position()
                            ),
                        ));
                    }
                    cursor.write_u32::<LittleEndian>(key_bytes.len() as u32)?;
                    cursor.write_all(key_bytes)?;
                }

                // Serialize children (page IDs)
                for &child_id in children {
                    if cursor.position() as usize + 4 > PAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Node data exceeds page size when writing children",
                        ));
                    }
                    cursor.write_u32::<LittleEndian>(child_id)?;
                }
            }
        }

        // The rest of the buffer is already zero-padded (initialized with zeros)
        Ok(buffer)
    }

    /// Deserializes a node from a 4096-byte buffer.
    /// Includes bounds checking to prevent OOM attacks from corrupted data.
    pub fn deserialize(buffer: &[u8; PAGE_SIZE]) -> Result<Self, std::io::Error> {
        let mut cursor = std::io::Cursor::new(buffer);

        // Read node type (byte 0)
        let node_type_byte = cursor.read_u8()?;
        let node_type = match node_type_byte {
            0 => NodeType::Leaf,
            1 => NodeType::Internal,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Invalid node type: {}", node_type_byte),
                ));
            }
        };

        // Read num_keys (bytes 1-4)
        let num_keys = cursor.read_u32::<LittleEndian>()?;

        // Validate num_keys to prevent excessive allocations
        if num_keys > MAX_NUM_KEYS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "num_keys ({}) exceeds maximum allowed ({})",
                    num_keys, MAX_NUM_KEYS
                ),
            ));
        }

        match node_type {
            NodeType::Leaf => {
                let mut pairs = Vec::with_capacity(num_keys as usize);

                for i in 0..num_keys {
                    // Read key length and validate
                    let key_len = cursor.read_u32::<LittleEndian>()?;
                    if key_len > MAX_KEY_LEN {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Key {} length ({}) exceeds maximum allowed ({})",
                                i, key_len, MAX_KEY_LEN
                            ),
                        ));
                    }

                    // Check if key would read past buffer
                    if cursor.position() as usize + key_len as usize > PAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Key {} read would exceed page boundary (pos: {}, len: {})",
                                i,
                                cursor.position(),
                                key_len
                            ),
                        ));
                    }

                    let mut key_bytes = vec![0u8; key_len as usize];
                    cursor.read_exact(&mut key_bytes)?;
                    let key = String::from_utf8(key_bytes).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid UTF-8 in key {}: {}", i, e),
                        )
                    })?;

                    // Read value length and validate
                    let value_len = cursor.read_u32::<LittleEndian>()?;
                    if value_len > MAX_VALUE_LEN {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Value {} length ({}) exceeds maximum allowed ({})",
                                i, value_len, MAX_VALUE_LEN
                            ),
                        ));
                    }

                    // Check if value would read past buffer
                    if cursor.position() as usize + value_len as usize > PAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Value {} read would exceed page boundary (pos: {}, len: {})",
                                i,
                                cursor.position(),
                                value_len
                            ),
                        ));
                    }

                    let mut value_bytes = vec![0u8; value_len as usize];
                    cursor.read_exact(&mut value_bytes)?;
                    let value = String::from_utf8(value_bytes).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid UTF-8 in value {}: {}", i, e),
                        )
                    })?;

                    pairs.push((key, value));
                }

                Ok(Node::Leaf {
                    node_type: NodeType::Leaf,
                    num_keys,
                    pairs,
                })
            }
            NodeType::Internal => {
                let mut keys = Vec::with_capacity(num_keys as usize);
                let mut children = Vec::with_capacity(num_keys as usize + 1);

                // Read keys
                for i in 0..num_keys {
                    let key_len = cursor.read_u32::<LittleEndian>()?;
                    if key_len > MAX_KEY_LEN {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Internal key {} length ({}) exceeds maximum allowed ({})",
                                i, key_len, MAX_KEY_LEN
                            ),
                        ));
                    }

                    // Check if key would read past buffer
                    if cursor.position() as usize + key_len as usize > PAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Internal key {} read would exceed page boundary (pos: {}, len: {})",
                                i,
                                cursor.position(),
                                key_len
                            ),
                        ));
                    }

                    let mut key_bytes = vec![0u8; key_len as usize];
                    cursor.read_exact(&mut key_bytes)?;
                    let key = String::from_utf8(key_bytes).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid UTF-8 in internal key {}: {}", i, e),
                        )
                    })?;
                    keys.push(key);
                }

                // Read children (num_keys + 1 children)
                let num_children = num_keys + 1;
                let children_size = num_children as usize * 4;
                if cursor.position() as usize + children_size > PAGE_SIZE {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "Children read would exceed page boundary (pos: {}, need: {} bytes for {} children)",
                            cursor.position(),
                            children_size,
                            num_children
                        ),
                    ));
                }

                for _ in 0..num_children {
                    let child_id = cursor.read_u32::<LittleEndian>()?;
                    children.push(child_id);
                }

                Ok(Node::Internal {
                    node_type: NodeType::Internal,
                    num_keys,
                    keys,
                    children,
                })
            }
        }
    }
}
