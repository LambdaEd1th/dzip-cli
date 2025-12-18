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
