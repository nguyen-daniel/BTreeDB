use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Page size in bytes (4KB)
pub const PAGE_SIZE: usize = 4096;

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
                    cursor.write_u32::<LittleEndian>(key_bytes.len() as u32)?;
                    cursor.write_all(key_bytes)?;
                }

                // Serialize children (page IDs)
                for &child_id in children {
                    cursor.write_u32::<LittleEndian>(child_id)?;
                }
            }
        }

        // The rest of the buffer is already zero-padded (initialized with zeros)
        Ok(buffer)
    }

    /// Deserializes a node from a 4096-byte buffer.
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

        match node_type {
            NodeType::Leaf => {
                let mut pairs = Vec::new();

                for _ in 0..num_keys {
                    // Read key
                    let key_len = cursor.read_u32::<LittleEndian>()?;
                    let mut key_bytes = vec![0u8; key_len as usize];
                    cursor.read_exact(&mut key_bytes)?;
                    let key = String::from_utf8(key_bytes).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid UTF-8 in key: {}", e),
                        )
                    })?;

                    // Read value
                    let value_len = cursor.read_u32::<LittleEndian>()?;
                    let mut value_bytes = vec![0u8; value_len as usize];
                    cursor.read_exact(&mut value_bytes)?;
                    let value = String::from_utf8(value_bytes).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid UTF-8 in value: {}", e),
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
                let mut keys = Vec::new();
                let mut children = Vec::new();

                // Read keys
                for _ in 0..num_keys {
                    let key_len = cursor.read_u32::<LittleEndian>()?;
                    let mut key_bytes = vec![0u8; key_len as usize];
                    cursor.read_exact(&mut key_bytes)?;
                    let key = String::from_utf8(key_bytes).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Invalid UTF-8 in key: {}", e),
                        )
                    })?;
                    keys.push(key);
                }

                // Read children (num_keys + 1 children)
                for _ in 0..=num_keys {
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
