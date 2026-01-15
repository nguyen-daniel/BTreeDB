//! Value module for supporting multiple value types.
//!
//! Provides a `Value` enum that can represent different data types:
//! strings, integers, floats, binary data, and null values.

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::fmt;
use std::io::{self, Read, Write};

/// Type tag for serialized values.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// UTF-8 string value
    String = 0,
    /// 64-bit signed integer
    Integer = 1,
    /// 64-bit floating point
    Float = 2,
    /// Binary blob
    Binary = 3,
    /// Null value
    Null = 4,
}

impl TryFrom<u8> for ValueType {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ValueType::String),
            1 => Ok(ValueType::Integer),
            2 => Ok(ValueType::Float),
            3 => Ok(ValueType::Binary),
            4 => Ok(ValueType::Null),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid value type: {}", value),
            )),
        }
    }
}

/// A typed value that can be stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// UTF-8 string value
    String(String),
    /// 64-bit signed integer
    Integer(i64),
    /// 64-bit floating point
    Float(f64),
    /// Binary blob
    Binary(Vec<u8>),
    /// Null value
    Null,
}

impl Value {
    /// Returns the type of this value.
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::String(_) => ValueType::String,
            Value::Integer(_) => ValueType::Integer,
            Value::Float(_) => ValueType::Float,
            Value::Binary(_) => ValueType::Binary,
            Value::Null => ValueType::Null,
        }
    }

    /// Serializes the value to bytes.
    /// Format: [1 byte type tag] [value data]
    pub fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let mut bytes_written = 1; // type tag

        // Write type tag
        writer.write_u8(self.value_type() as u8)?;

        match self {
            Value::String(s) => {
                let bytes = s.as_bytes();
                writer.write_u32::<LittleEndian>(bytes.len() as u32)?;
                writer.write_all(bytes)?;
                bytes_written += 4 + bytes.len();
            }
            Value::Integer(i) => {
                writer.write_i64::<LittleEndian>(*i)?;
                bytes_written += 8;
            }
            Value::Float(f) => {
                writer.write_f64::<LittleEndian>(*f)?;
                bytes_written += 8;
            }
            Value::Binary(b) => {
                writer.write_u32::<LittleEndian>(b.len() as u32)?;
                writer.write_all(b)?;
                bytes_written += 4 + b.len();
            }
            Value::Null => {
                // No additional data for null
            }
        }

        Ok(bytes_written)
    }

    /// Deserializes a value from bytes.
    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        let type_tag = reader.read_u8()?;
        let value_type = ValueType::try_from(type_tag)?;

        match value_type {
            ValueType::String => {
                let len = reader.read_u32::<LittleEndian>()? as usize;
                let mut bytes = vec![0u8; len];
                reader.read_exact(&mut bytes)?;
                let s = String::from_utf8(bytes).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("Invalid UTF-8: {}", e))
                })?;
                Ok(Value::String(s))
            }
            ValueType::Integer => {
                let i = reader.read_i64::<LittleEndian>()?;
                Ok(Value::Integer(i))
            }
            ValueType::Float => {
                let f = reader.read_f64::<LittleEndian>()?;
                Ok(Value::Float(f))
            }
            ValueType::Binary => {
                let len = reader.read_u32::<LittleEndian>()? as usize;
                let mut bytes = vec![0u8; len];
                reader.read_exact(&mut bytes)?;
                Ok(Value::Binary(bytes))
            }
            ValueType::Null => Ok(Value::Null),
        }
    }

    /// Parses a value from a string with optional type prefix.
    /// Format: `[type:]value`
    /// Types: `s:` (string, default), `i:` (integer), `f:` (float), `b:` (binary hex), `null`
    pub fn parse(s: &str) -> Result<Self, String> {
        if s == "null" || s == "NULL" {
            return Ok(Value::Null);
        }

        if let Some(rest) = s.strip_prefix("i:") {
            let i: i64 = rest
                .parse()
                .map_err(|e| format!("Invalid integer: {}", e))?;
            return Ok(Value::Integer(i));
        }

        if let Some(rest) = s.strip_prefix("f:") {
            let f: f64 = rest.parse().map_err(|e| format!("Invalid float: {}", e))?;
            return Ok(Value::Float(f));
        }

        if let Some(rest) = s.strip_prefix("b:") {
            let bytes = hex_decode(rest).map_err(|e| format!("Invalid hex: {}", e))?;
            return Ok(Value::Binary(bytes));
        }

        if let Some(rest) = s.strip_prefix("s:") {
            return Ok(Value::String(rest.to_string()));
        }

        // Default to string
        Ok(Value::String(s.to_string()))
    }

    /// Converts the value to a display string.
    pub fn to_display_string(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            Value::Integer(i) => format!("(int) {}", i),
            Value::Float(f) => format!("(float) {}", f),
            Value::Binary(b) => format!("(binary) {}", hex_encode(b)),
            Value::Null => "(null)".to_string(),
        }
    }

    /// Returns true if this is a string value.
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    /// Returns the string value if this is a string.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => write!(f, "{}", s),
            Value::Integer(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Binary(b) => write!(f, "<binary {} bytes>", b.len()),
            Value::Null => write!(f, "null"),
        }
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Integer(i)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}

impl From<Vec<u8>> for Value {
    fn from(b: Vec<u8>) -> Self {
        Value::Binary(b)
    }
}

/// Encodes bytes as a hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Decodes a hex string to bytes.
fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("Hex string must have even length".to_string());
    }

    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| format!("Invalid hex at position {}: {}", i, e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_value_serialize_deserialize() {
        let values = vec![
            Value::String("hello".to_string()),
            Value::Integer(42),
            Value::Integer(-123456789),
            Value::Float(3.14159),
            Value::Binary(vec![0x00, 0x01, 0x02, 0xFF]),
            Value::Null,
        ];

        for value in values {
            let mut buffer = Vec::new();
            value.serialize(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let deserialized = Value::deserialize(&mut cursor).unwrap();

            assert_eq!(value, deserialized);
        }
    }

    #[test]
    fn test_value_parse() {
        assert_eq!(
            Value::parse("hello").unwrap(),
            Value::String("hello".to_string())
        );
        assert_eq!(
            Value::parse("s:hello").unwrap(),
            Value::String("hello".to_string())
        );
        assert_eq!(Value::parse("i:42").unwrap(), Value::Integer(42));
        assert_eq!(Value::parse("i:-100").unwrap(), Value::Integer(-100));
        assert_eq!(Value::parse("f:3.14").unwrap(), Value::Float(3.14));
        assert_eq!(
            Value::parse("b:00ff").unwrap(),
            Value::Binary(vec![0x00, 0xFF])
        );
        assert_eq!(Value::parse("null").unwrap(), Value::Null);
    }

    #[test]
    fn test_hex_encode_decode() {
        let original = vec![0x00, 0x01, 0x02, 0xAB, 0xCD, 0xEF];
        let encoded = hex_encode(&original);
        assert_eq!(encoded, "000102abcdef");

        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}
