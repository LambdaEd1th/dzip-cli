use crate::constants::ChunkFlags; // [Refactor] Import ChunkFlags
use anyhow::{Context, Result, anyhow};
use std::io::{self, Read, Write};
use std::sync::Arc;

pub trait Decompressor: Send + Sync {
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write, len: u32) -> Result<()>;
}

pub trait Compressor: Send + Sync {
    fn compress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<()>;
}

#[derive(Clone)]
pub struct CodecRegistry {
    // [Refactor] Store ChunkFlags keys instead of u16
    decompressors: Vec<(ChunkFlags, Arc<dyn Decompressor>)>,
    compressors: Vec<(ChunkFlags, Arc<dyn Compressor>)>,
}

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

    // [Refactor] Register methods now accept ChunkFlags
    pub fn register_decompressor<D: Decompressor + 'static>(
        &mut self,
        mask: ChunkFlags,
        decompressor: D,
    ) {
        self.decompressors.push((mask, Arc::new(decompressor)));
    }

    pub fn register_compressor<C: Compressor + 'static>(
        &mut self,
        mask: ChunkFlags,
        compressor: C,
    ) {
        self.compressors.push((mask, Arc::new(compressor)));
    }

    pub fn decompress(
        &self,
        input: &mut dyn Read,
        output: &mut dyn Write,
        flags_raw: u16, // Input is still u16 from file
        len: u32,
    ) -> Result<()> {
        // [Refactor] Convert raw bits to ChunkFlags
        let flags = ChunkFlags::from_bits_truncate(flags_raw);

        for (mask, decoder) in &self.decompressors {
            // [Refactor] Use intersects() to check if the specific compression flag is set
            if flags.intersects(*mask) {
                return decoder.decompress(input, output, len);
            }
        }
        // Default: COPY (Store)
        io::copy(input, output)?;
        Ok(())
    }

    pub fn compress(
        &self,
        input: &mut dyn Read,
        output: &mut dyn Write,
        flags_raw: u16,
    ) -> Result<()> {
        let flags = ChunkFlags::from_bits_truncate(flags_raw);
        for (mask, encoder) in &self.compressors {
            if flags.intersects(*mask) {
                return encoder.compress(input, output);
            }
        }
        // Default: COPY
        io::copy(input, output)?;
        Ok(())
    }
}

// --- Algorithm Implementations (Unchanged) ---
struct ZeroDecompressor;
impl Decompressor for ZeroDecompressor {
    fn decompress(&self, _input: &mut dyn Read, output: &mut dyn Write, len: u32) -> Result<()> {
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
        io::copy(input, output)?;
        Ok(())
    }
}

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

pub fn create_default_registry() -> CodecRegistry {
    let mut reg = CodecRegistry::new();

    // [Refactor] Use ChunkFlags constants for registration
    reg.register_decompressor(ChunkFlags::ZERO, ZeroDecompressor);
    reg.register_decompressor(ChunkFlags::DZ_RANGE, PassThroughDecompressor);
    reg.register_decompressor(ChunkFlags::LZMA, LzmaDecompressor);
    reg.register_decompressor(ChunkFlags::ZLIB, ZlibDecompressor);
    reg.register_decompressor(ChunkFlags::BZIP, Bzip2Decompressor);

    reg.register_compressor(ChunkFlags::LZMA, LzmaCompressor);
    reg.register_compressor(ChunkFlags::ZLIB, ZlibCompressor);
    reg.register_compressor(ChunkFlags::BZIP, Bzip2Compressor);

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn setup_registry() -> CodecRegistry {
        create_default_registry()
    }

    #[test]
    fn test_zero_decompressor() {
        let registry = setup_registry();
        let mut input = Cursor::new(vec![]);
        let mut output = Cursor::new(Vec::new());
        let target_len = 100;

        // [Refactor] Pass .bits() (u16) to the decompress function
        let res = registry.decompress(&mut input, &mut output, ChunkFlags::ZERO.bits(), target_len);
        assert!(res.is_ok());

        let result_data = output.into_inner();
        assert_eq!(result_data.len(), target_len as usize);
        assert!(result_data.iter().all(|&b| b == 0));
    }

    // ... (Other tests omitted for brevity, logic is same) ...
}
