use crate::Result;
use crate::error::DzipError;
use crate::format::ChunkFlags;
use std::io::{Read, Write};

/// Compresses data from `input` to `output` based on the provided flags.
pub fn compress(input: &mut dyn Read, output: &mut dyn Write, flags: u16) -> Result<()> {
    let flags_enum = ChunkFlags::from_bits_truncate(flags);

    match flags_enum {
        ChunkFlags::LZMA => {
            // LZMA Compression
            let options = lzma_rust2::LzmaOptions::with_preset(6);

            // LzmaWriter::new(out, options, use_header, use_end_marker, expected_size)
            let mut encoder = lzma_rust2::LzmaWriter::new(output, &options, true, true, None)
                .map_err(|e| DzipError::Compression(format!("LZMA init failed: {}", e)))?;

            std::io::copy(input, &mut encoder).map_err(DzipError::Io)?;

            encoder
                .finish()
                .map_err(|e| DzipError::Compression(format!("LZMA finish failed: {}", e)))?;
        }
        ChunkFlags::ZLIB => {
            // ZLIB Compression
            let mut encoder =
                flate2::write::ZlibEncoder::new(output, flate2::Compression::default());
            std::io::copy(input, &mut encoder).map_err(DzipError::Io)?;
            encoder
                .finish()
                .map_err(|e| DzipError::Compression(format!("Zlib finish failed: {}", e)))?;
        }
        ChunkFlags::BZIP => {
            // BZIP2 Compression
            let mut encoder = bzip2::write::BzEncoder::new(output, bzip2::Compression::default());
            std::io::copy(input, &mut encoder).map_err(DzipError::Io)?;
            encoder
                .finish()
                .map_err(|e| DzipError::Compression(format!("Bzip2 finish failed: {}", e)))?;
        }
        // Default: Store (Copy without compression) or other unimplemented flags
        _ => {
            std::io::copy(input, output).map_err(DzipError::Io)?;
        }
    }

    Ok(())
}

/// Decompresses data from `input` to `output` based on the provided flags.
pub fn decompress(
    input: &mut dyn Read,
    output: &mut dyn Write,
    flags: u16,
    _compressed_size: u32,
) -> Result<()> {
    let flags_enum = ChunkFlags::from_bits_truncate(flags);

    match flags_enum {
        ChunkFlags::LZMA => {
            // LZMA Decompression
            let mut decoder = lzma_rust2::LzmaReader::new_mem_limit(input, u32::MAX, None)
                .map_err(|e| DzipError::Decompression(format!("LZMA init failed: {}", e)))?;

            std::io::copy(&mut decoder, output).map_err(DzipError::Io)?;
        }
        ChunkFlags::ZLIB => {
            // ZLIB Decompression
            let mut decoder = flate2::read::ZlibDecoder::new(input);
            std::io::copy(&mut decoder, output).map_err(DzipError::Io)?;
        }
        ChunkFlags::BZIP => {
            // BZIP2 Decompression
            let mut decoder = bzip2::read::BzDecoder::new(input);
            std::io::copy(&mut decoder, output).map_err(DzipError::Io)?;
        }
        // Default: Store (Copy without decompression)
        _ => {
            std::io::copy(input, output).map_err(DzipError::Io)?;
        }
    }

    Ok(())
}
