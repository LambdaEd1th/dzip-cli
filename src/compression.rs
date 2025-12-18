use crate::constants::*;
use anyhow::{Context, Result, anyhow};
use std::io::{self, Read, Write};
use std::sync::Arc;

/// Define Decompressor trait
pub trait Decompressor: Send + Sync {
    /// Execute decompression
    /// input: Input stream (length limited)
    /// output: Output stream
    /// len: Expected length (some algorithms might need it)
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write, len: u32) -> Result<()>;
}

/// Define Compressor trait
pub trait Compressor: Send + Sync {
    fn compress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<()>;
}

/// Codec Registry
#[derive(Clone)]
pub struct CodecRegistry {
    decompressors: Vec<(u16, Arc<dyn Decompressor>)>,
    compressors: Vec<(u16, Arc<dyn Compressor>)>,
}

// Added Default implementation to fix clippy warning
impl Default for CodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CodecRegistry {
    pub fn new() -> Self {
        Self {
            decompressors: Vec::new(),
            compressors: Vec::new(),
        }
    }

    pub fn register_decompressor<D: Decompressor + 'static>(&mut self, mask: u16, decompressor: D) {
        self.decompressors.push((mask, Arc::new(decompressor)));
    }

    pub fn register_compressor<C: Compressor + 'static>(&mut self, mask: u16, compressor: C) {
        self.compressors.push((mask, Arc::new(compressor)));
    }

    pub fn decompress(
        &self,
        input: &mut dyn Read,
        output: &mut dyn Write,
        flags: u16,
        len: u32,
    ) -> Result<()> {
        for (mask, decoder) in &self.decompressors {
            if flags & *mask != 0 {
                return decoder.decompress(input, output, len);
            }
        }
        // Default: COPY (Store)
        io::copy(input, output)?;
        Ok(())
    }

    pub fn compress(&self, input: &mut dyn Read, output: &mut dyn Write, flags: u16) -> Result<()> {
        for (mask, encoder) in &self.compressors {
            if flags & *mask != 0 {
                return encoder.compress(input, output);
            }
        }
        // Default: COPY
        io::copy(input, output)?;
        Ok(())
    }
}

// --- Standard Algorithm Implementations ---

struct ZeroDecompressor;
impl Decompressor for ZeroDecompressor {
    fn decompress(&self, _input: &mut dyn Read, output: &mut dyn Write, len: u32) -> Result<()> {
        // Write `len` zeros
        let chunk_size = 4096;
        let zeros = vec![0u8; chunk_size];
        let mut remaining = len as usize;
        while remaining > 0 {
            let to_write = std::cmp::min(remaining, chunk_size);
            output.write_all(&zeros[..to_write])?;
            remaining -= to_write;
        }
        Ok(())
    }
}

struct LzmaDecompressor;
impl Decompressor for LzmaDecompressor {
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write, _len: u32) -> Result<()> {
        let mut lzma_reader = lzma_rust2::LzmaReader::new_mem_limit(input, u32::MAX, None)
            .map_err(|e| anyhow!("Failed to initialize LZMA reader: {}", e))?;
        io::copy(&mut lzma_reader, output).context("LZMA decompress failed")?;
        Ok(())
    }
}

struct ZlibDecompressor;
impl Decompressor for ZlibDecompressor {
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write, _len: u32) -> Result<()> {
        let mut d = flate2::read::ZlibDecoder::new(input);
        io::copy(&mut d, output).context("ZLIB decompress failed")?;
        Ok(())
    }
}

struct Bzip2Decompressor;
impl Decompressor for Bzip2Decompressor {
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write, _len: u32) -> Result<()> {
        let mut d = bzip2::read::BzDecoder::new(input);
        io::copy(&mut d, output).context("BZIP2 decompress failed")?;
        Ok(())
    }
}

struct PassThroughDecompressor;
impl Decompressor for PassThroughDecompressor {
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write, _len: u32) -> Result<()> {
        // Pass-through copy
        io::copy(input, output)?;
        Ok(())
    }
}

// --- Compressor Implementations ---

struct LzmaCompressor;
impl Compressor for LzmaCompressor {
    fn compress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<()> {
        let options = lzma_rust2::LzmaOptions::default();
        let mut w = lzma_rust2::LzmaWriter::new_use_header(output, &options, None)
            .context("Failed to initialize LZMA writer")?;
        io::copy(input, &mut w)?;
        w.finish()?;
        Ok(())
    }
}

struct ZlibCompressor;
impl Compressor for ZlibCompressor {
    fn compress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<()> {
        let mut e = flate2::write::ZlibEncoder::new(output, flate2::Compression::default());
        io::copy(input, &mut e)?;
        e.finish()?;
        Ok(())
    }
}

struct Bzip2Compressor;
impl Compressor for Bzip2Compressor {
    fn compress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<()> {
        let mut e = bzip2::write::BzEncoder::new(output, bzip2::Compression::default());
        io::copy(input, &mut e)?;
        e.finish()?;
        Ok(())
    }
}

/// Default factory method
pub fn create_default_registry() -> CodecRegistry {
    let mut reg = CodecRegistry::new();

    // The registration order determines priority
    reg.register_decompressor(CHUNK_ZERO, ZeroDecompressor);
    reg.register_decompressor(CHUNK_DZ, PassThroughDecompressor);
    reg.register_decompressor(CHUNK_LZMA, LzmaDecompressor);
    reg.register_decompressor(CHUNK_ZLIB, ZlibDecompressor);
    reg.register_decompressor(CHUNK_BZIP, Bzip2Decompressor);

    reg.register_compressor(CHUNK_LZMA, LzmaCompressor);
    reg.register_compressor(CHUNK_ZLIB, ZlibCompressor);
    reg.register_compressor(CHUNK_BZIP, Bzip2Compressor);

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // Helper: create a memory buffer for testing
    fn setup_registry() -> CodecRegistry {
        create_default_registry()
    }

    #[test]
    fn test_zero_decompressor() {
        let registry = setup_registry();
        // ZERO algorithm needs no input data, only length
        let mut input = Cursor::new(vec![]);
        let mut output = Cursor::new(Vec::new());
        let target_len = 100;

        // Execute decompression: expect 100 zero bytes output
        let res = registry.decompress(&mut input, &mut output, CHUNK_ZERO, target_len);
        assert!(res.is_ok());

        let result_data = output.into_inner();
        assert_eq!(result_data.len(), target_len as usize);
        assert!(result_data.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_copy_fallback_decompress() {
        let registry = setup_registry();
        let raw_data = b"Hello, Marmalade SDK!";
        let mut input = Cursor::new(raw_data);
        let mut output = Cursor::new(Vec::new());

        // Use 0 (COPY) or unknown Flag
        // Expected behavior: directly copy input to output
        let res = registry.decompress(&mut input, &mut output, 0, raw_data.len() as u32);
        assert!(res.is_ok());

        let result_data = output.into_inner();
        assert_eq!(result_data, raw_data);
    }

    #[test]
    fn test_unknown_flag_fallback() {
        let registry = setup_registry();
        let raw_data = b"Data with unknown flag";
        let mut input = Cursor::new(raw_data);
        let mut output = Cursor::new(Vec::new());

        // Use an unregistered Flag, e.g., 0x8000
        let unknown_flag = 0x8000;

        // Expected: registry.decompress defaults to COPY when no decoder is found
        let res = registry.decompress(&mut input, &mut output, unknown_flag, raw_data.len() as u32);
        assert!(res.is_ok());
        assert_eq!(output.into_inner(), raw_data);
    }

    #[test]
    fn test_lzma_roundtrip() {
        // Integration test: verify LZMA compression and decompression roundtrip
        let registry = setup_registry();
        let original_data = b"Repeat Repeat Repeat Repeat Repeat";

        // 1. Compress
        let mut input_compress = Cursor::new(original_data);
        let mut compressed_output = Cursor::new(Vec::new());
        let compress_res =
            registry.compress(&mut input_compress, &mut compressed_output, CHUNK_LZMA);
        assert!(compress_res.is_ok());

        let compressed_bytes = compressed_output.into_inner();
        // Compressed data should differ from original (usually smaller, or has LZMA header)
        assert_ne!(compressed_bytes, original_data);

        // 2. Decompress
        let mut input_decompress = Cursor::new(compressed_bytes);
        let mut restored_output = Cursor::new(Vec::new());
        // LZMA decompression usually doesn't need pre-known length, but the interface requires it
        let decompress_res = registry.decompress(
            &mut input_decompress,
            &mut restored_output,
            CHUNK_LZMA,
            original_data.len() as u32,
        );
        assert!(decompress_res.is_ok());

        assert_eq!(restored_output.into_inner(), original_data);
    }
}
