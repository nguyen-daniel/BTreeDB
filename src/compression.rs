//! Compression module for reducing storage overhead.
//!
//! Provides simple compression utilities for large values.
//! Uses a simple run-length encoding (RLE) scheme for educational purposes.
//! In production, you would use libraries like lz4 or zstd.

use std::io::{self, Read, Write};

/// Minimum size for compression to be worthwhile.
pub const COMPRESSION_THRESHOLD: usize = 64;

/// Compression flag for serialized data.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    /// No compression applied
    None = 0,
    /// Run-length encoding
    RLE = 1,
}

impl TryFrom<u8> for CompressionType {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CompressionType::None),
            1 => Ok(CompressionType::RLE),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid compression type: {}", value),
            )),
        }
    }
}

/// Compressed data container.
#[derive(Debug, Clone)]
pub struct CompressedData {
    /// Compression type used
    pub compression_type: CompressionType,
    /// Original uncompressed size
    pub original_size: u32,
    /// Compressed data bytes
    pub data: Vec<u8>,
}

impl CompressedData {
    /// Creates uncompressed data.
    pub fn uncompressed(data: Vec<u8>) -> Self {
        let size = data.len() as u32;
        CompressedData {
            compression_type: CompressionType::None,
            original_size: size,
            data,
        }
    }

    /// Returns true if the data is compressed.
    pub fn is_compressed(&self) -> bool {
        self.compression_type != CompressionType::None
    }

    /// Returns the compression ratio (compressed size / original size).
    pub fn compression_ratio(&self) -> f64 {
        if self.original_size == 0 {
            1.0
        } else {
            self.data.len() as f64 / self.original_size as f64
        }
    }

    /// Serializes the compressed data.
    pub fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&[self.compression_type as u8])?;
        writer.write_all(&self.original_size.to_le_bytes())?;
        writer.write_all(&(self.data.len() as u32).to_le_bytes())?;
        writer.write_all(&self.data)?;
        Ok(())
    }

    /// Deserializes compressed data.
    pub fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        let mut type_byte = [0u8; 1];
        reader.read_exact(&mut type_byte)?;
        let compression_type = CompressionType::try_from(type_byte[0])?;

        let mut size_bytes = [0u8; 4];
        reader.read_exact(&mut size_bytes)?;
        let original_size = u32::from_le_bytes(size_bytes);

        reader.read_exact(&mut size_bytes)?;
        let compressed_size = u32::from_le_bytes(size_bytes) as usize;

        let mut data = vec![0u8; compressed_size];
        reader.read_exact(&mut data)?;

        Ok(CompressedData {
            compression_type,
            original_size,
            data,
        })
    }
}

/// Compresses data using run-length encoding if beneficial.
/// Returns the original data if compression doesn't help.
pub fn compress(data: &[u8]) -> CompressedData {
    if data.len() < COMPRESSION_THRESHOLD {
        return CompressedData::uncompressed(data.to_vec());
    }

    let compressed = rle_compress(data);

    // Only use compression if it actually reduces size
    if compressed.len() < data.len() {
        CompressedData {
            compression_type: CompressionType::RLE,
            original_size: data.len() as u32,
            data: compressed,
        }
    } else {
        CompressedData::uncompressed(data.to_vec())
    }
}

/// Decompresses data.
pub fn decompress(compressed: &CompressedData) -> io::Result<Vec<u8>> {
    match compressed.compression_type {
        CompressionType::None => Ok(compressed.data.clone()),
        CompressionType::RLE => {
            let decompressed = rle_decompress(&compressed.data)?;
            if decompressed.len() != compressed.original_size as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Decompressed size mismatch: expected {}, got {}",
                        compressed.original_size,
                        decompressed.len()
                    ),
                ));
            }
            Ok(decompressed)
        }
    }
}

/// Simple run-length encoding compression.
/// Format: [count, byte] pairs where count is 1-255.
fn rle_compress(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < data.len() {
        let byte = data[i];
        let mut count = 1u8;

        // Count consecutive identical bytes (up to 255)
        while i + (count as usize) < data.len() && data[i + (count as usize)] == byte && count < 255
        {
            count += 1;
        }

        result.push(count);
        result.push(byte);
        i += count as usize;
    }

    result
}

/// Run-length encoding decompression.
fn rle_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    if !data.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "RLE data must have even length",
        ));
    }

    let mut result = Vec::new();

    for chunk in data.chunks(2) {
        let count = chunk[0] as usize;
        let byte = chunk[1];
        result.extend(std::iter::repeat_n(byte, count));
    }

    Ok(result)
}

/// Statistics about compression.
#[derive(Debug, Clone, Default)]
pub struct CompressionStats {
    /// Total bytes before compression
    pub total_original: u64,
    /// Total bytes after compression
    pub total_compressed: u64,
    /// Number of items compressed
    pub items_compressed: u64,
    /// Number of items not compressed (too small or no benefit)
    pub items_uncompressed: u64,
}

impl CompressionStats {
    /// Creates new empty stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a compression operation.
    pub fn record(&mut self, compressed: &CompressedData) {
        self.total_original += compressed.original_size as u64;
        self.total_compressed += compressed.data.len() as u64;

        if compressed.is_compressed() {
            self.items_compressed += 1;
        } else {
            self.items_uncompressed += 1;
        }
    }

    /// Returns the overall compression ratio.
    pub fn overall_ratio(&self) -> f64 {
        if self.total_original == 0 {
            1.0
        } else {
            self.total_compressed as f64 / self.total_original as f64
        }
    }

    /// Returns the space savings percentage.
    pub fn savings_percentage(&self) -> f64 {
        (1.0 - self.overall_ratio()) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rle_compress_decompress() {
        let data = b"AAAAAABBBCCCCCCCCDDDD";
        let compressed = rle_compress(data);
        let decompressed = rle_decompress(&compressed).unwrap();
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_rle_long_runs() {
        // Test with very long runs
        let data: Vec<u8> = vec![0xAA; 1000];
        let compressed = rle_compress(&data);
        let decompressed = rle_decompress(&compressed).unwrap();
        assert_eq!(data, decompressed);

        // Should be much smaller
        assert!(compressed.len() < data.len() / 2);
    }

    #[test]
    fn test_compress_small_data() {
        // Small data should not be compressed
        let data = b"hello";
        let compressed = compress(data);
        assert_eq!(compressed.compression_type, CompressionType::None);
        assert_eq!(compressed.data, data.to_vec());
    }

    #[test]
    fn test_compress_repetitive_data() {
        // Repetitive data should compress well
        let data: Vec<u8> = vec![0x42; 256];
        let compressed = compress(&data);
        assert_eq!(compressed.compression_type, CompressionType::RLE);
        assert!(compressed.data.len() < data.len());

        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_compress_random_data() {
        // Random data may not compress well
        let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
        let compressed = compress(&data);

        // Should either not compress or have same size
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_compression_stats() {
        let mut stats = CompressionStats::new();

        // Record some compressions
        let data1: Vec<u8> = vec![0x00; 1000];
        let compressed1 = compress(&data1);
        stats.record(&compressed1);

        let data2 = b"short".to_vec();
        let compressed2 = compress(&data2);
        stats.record(&compressed2);

        assert_eq!(stats.items_compressed, 1);
        assert_eq!(stats.items_uncompressed, 1);
        assert!(stats.savings_percentage() > 0.0);
    }

    #[test]
    fn test_compressed_data_serialization() {
        let data: Vec<u8> = vec![0x42; 256];
        let compressed = compress(&data);

        let mut buffer = Vec::new();
        compressed.serialize(&mut buffer).unwrap();

        let mut cursor = std::io::Cursor::new(buffer);
        let deserialized = CompressedData::deserialize(&mut cursor).unwrap();

        assert_eq!(compressed.compression_type, deserialized.compression_type);
        assert_eq!(compressed.original_size, deserialized.original_size);
        assert_eq!(compressed.data, deserialized.data);
    }
}
