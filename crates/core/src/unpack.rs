use byteorder::{LittleEndian, ReadBytesExt};
use log::{info, warn};
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};

use crate::Result;
use crate::codecs::decompress;
use crate::error::DzipError;
use crate::format::{
    CHUNK_LIST_TERMINATOR, CURRENT_DIR_STR, ChunkFlags, DEFAULT_BUFFER_SIZE, MAGIC,
};
use crate::io::{ReadSeekSend, UnpackSink, UnpackSource};
use crate::model::{ArchiveMeta, ChunkDef, Config, FileEntry, RangeSettings};
use crate::utils::{decode_flags, read_null_term_string};

// --- Structures ---

#[derive(Debug)]
pub struct ArchiveMetadata {
    pub version: u8,
    pub user_files: Vec<String>,
    pub directories: Vec<String>,
    pub map_entries: Vec<FileMapEntry>,
    pub raw_chunks: Vec<RawChunk>,
    pub split_file_names: Vec<String>,
    pub range_settings: Option<RangeSettings>,
    pub main_file_len: u64,
}

pub struct UnpackPlan {
    pub metadata: ArchiveMetadata,
    pub processed_chunks: Vec<RawChunk>,
}

#[derive(Debug, Clone)]
pub struct FileMapEntry {
    pub id: usize,
    pub dir_idx: usize,
    pub chunk_ids: Vec<u16>,
}

#[derive(Clone, Debug)]
pub struct RawChunk {
    pub id: u16,
    pub offset: u32,
    pub _head_c_len: u32,
    pub d_len: u32,
    pub flags: u16,
    pub file_idx: u16,
    pub real_c_len: u32,
}

// --- Wrapper ---

/// Main entry point for unpacking.
/// Uses `&dyn UnpackSink` to allow parallel file creation (thread-safe).
pub fn do_unpack(
    source: &dyn UnpackSource,
    sink: &dyn UnpackSink,
    keep_raw: bool,
) -> Result<Config> {
    let meta = ArchiveMetadata::load(source)?;
    let plan = UnpackPlan::build(meta, source)?;
    plan.extract(sink, keep_raw, source)?;
    let config = plan.generate_config_struct()?;
    info!("Unpack complete. Config object generated.");
    Ok(config)
}

// --- Implementations ---

impl ArchiveMetadata {
    pub fn load(source: &dyn UnpackSource) -> Result<Self> {
        let mut main_file_raw = source.open_main()?;
        let main_file_len = main_file_raw
            .seek(SeekFrom::End(0))
            .map_err(DzipError::Io)?;
        main_file_raw
            .seek(SeekFrom::Start(0))
            .map_err(DzipError::Io)?;

        let mut reader = BufReader::with_capacity(DEFAULT_BUFFER_SIZE, main_file_raw);

        let magic = reader.read_u32::<LittleEndian>().map_err(DzipError::Io)?;
        if magic != MAGIC {
            return Err(DzipError::InvalidMagic(magic));
        }
        let num_files = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
        let num_dirs = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
        let version = reader.read_u8().map_err(DzipError::Io)?;

        info!(
            "Header: Ver {}, Files {}, Dirs {}",
            version, num_files, num_dirs
        );

        let mut user_files = Vec::with_capacity(num_files as usize);
        for _ in 0..num_files {
            user_files.push(read_null_term_string(&mut reader).map_err(DzipError::Io)?);
        }
        let mut directories = Vec::with_capacity(num_dirs as usize);
        directories.push(CURRENT_DIR_STR.to_string());
        for _ in 0..(num_dirs - 1) {
            // [Reverted Logic] We read the raw string as-is (e.g., "textures\ui").
            // We do NOT normalize to '/' here anymore. The CLI/Sink is responsible for OS adaptation.
            directories.push(read_null_term_string(&mut reader).map_err(DzipError::Io)?);
        }

        let mut map_entries = Vec::with_capacity(num_files as usize);
        for i in 0..num_files {
            let dir_id = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)? as usize;
            let mut chunk_ids = Vec::new();
            loop {
                let cid = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
                if cid == CHUNK_LIST_TERMINATOR {
                    break;
                }
                chunk_ids.push(cid);
            }
            map_entries.push(FileMapEntry {
                id: i as usize,
                dir_idx: dir_id,
                chunk_ids,
            });
        }

        let num_arch_files = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
        let num_chunks = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
        info!(
            "Chunk Settings: {} chunks in {} archive files",
            num_chunks, num_arch_files
        );

        let mut raw_chunks = Vec::with_capacity(num_chunks as usize);
        let mut has_dz_chunk = false;
        for i in 0..num_chunks {
            let offset = reader.read_u32::<LittleEndian>().map_err(DzipError::Io)?;
            let c_len = reader.read_u32::<LittleEndian>().map_err(DzipError::Io)?;
            let d_len = reader.read_u32::<LittleEndian>().map_err(DzipError::Io)?;
            let flags_raw = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
            let file_idx = reader.read_u16::<LittleEndian>().map_err(DzipError::Io)?;
            let flags = ChunkFlags::from_bits_truncate(flags_raw);
            if flags.contains(ChunkFlags::DZ_RANGE) {
                has_dz_chunk = true;
            }
            raw_chunks.push(RawChunk {
                id: i,
                offset,
                _head_c_len: c_len,
                d_len,
                flags: flags_raw,
                file_idx,
                real_c_len: 0,
            });
        }

        let mut split_file_names = Vec::new();
        if num_arch_files > 1 {
            info!("Reading {} split archive filenames...", num_arch_files - 1);
            for _ in 0..(num_arch_files - 1) {
                split_file_names.push(read_null_term_string(&mut reader).map_err(DzipError::Io)?);
            }
        }

        let mut range_settings = None;
        if has_dz_chunk {
            info!("Detected CHUNK_DZ, reading RangeSettings...");
            range_settings = Some(RangeSettings {
                win_size: reader.read_u8().map_err(DzipError::Io)?,
                flags: reader.read_u8().map_err(DzipError::Io)?,
                offset_table_size: reader.read_u8().map_err(DzipError::Io)?,
                offset_tables: reader.read_u8().map_err(DzipError::Io)?,
                offset_contexts: reader.read_u8().map_err(DzipError::Io)?,
                ref_length_table_size: reader.read_u8().map_err(DzipError::Io)?,
                ref_length_tables: reader.read_u8().map_err(DzipError::Io)?,
                ref_offset_table_size: reader.read_u8().map_err(DzipError::Io)?,
                ref_offset_tables: reader.read_u8().map_err(DzipError::Io)?,
                big_min_match: reader.read_u8().map_err(DzipError::Io)?,
            });
        }

        Ok(Self {
            version,
            user_files,
            directories,
            map_entries,
            raw_chunks,
            split_file_names,
            range_settings,
            main_file_len,
        })
    }
}

impl UnpackPlan {
    pub fn build(metadata: ArchiveMetadata, source: &dyn UnpackSource) -> Result<Self> {
        let processed_chunks = Self::calculate_chunk_sizes(&metadata, source)?;
        Ok(Self {
            metadata,
            processed_chunks,
        })
    }

    fn calculate_chunk_sizes(
        meta: &ArchiveMetadata,
        source: &dyn UnpackSource,
    ) -> Result<Vec<RawChunk>> {
        let mut chunks = meta.raw_chunks.clone();
        let mut file_chunks_map: HashMap<u16, Vec<usize>> = HashMap::new();
        for (idx, c) in chunks.iter().enumerate() {
            file_chunks_map.entry(c.file_idx).or_default().push(idx);
        }

        for (f_idx, c_indices) in file_chunks_map.iter() {
            let mut sorted_indices = c_indices.clone();
            sorted_indices.sort_by_key(|&i| chunks[i].offset);
            let current_file_size = if *f_idx == 0 {
                meta.main_file_len
            } else {
                let idx = (*f_idx - 1) as usize;
                let split_name = meta.split_file_names.get(idx).ok_or_else(|| {
                    DzipError::Generic(format!("Invalid split file index {} in header", f_idx))
                })?;
                source.get_split_len(split_name)?
            };

            for k in 0..sorted_indices.len() {
                let idx = sorted_indices[k];
                let current_offset = chunks[idx].offset;
                let next_offset = if k == sorted_indices.len() - 1 {
                    current_file_size as u32
                } else {
                    chunks[sorted_indices[k + 1]].offset
                };
                if next_offset < current_offset {
                    chunks[idx].real_c_len = chunks[idx]._head_c_len;
                } else {
                    chunks[idx].real_c_len = next_offset - current_offset;
                }
            }
        }
        Ok(chunks)
    }

    pub fn extract(
        &self,
        sink: &dyn UnpackSink,
        keep_raw: bool,
        source: &dyn UnpackSource,
    ) -> Result<()> {
        info!("Extracting {} files...", self.metadata.map_entries.len());
        let chunk_indices: HashMap<u16, usize> = self
            .processed_chunks
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i))
            .collect();

        self.metadata.map_entries.par_iter().try_for_each_init(
            HashMap::new,
            |file_cache: &mut HashMap<u16, Box<dyn ReadSeekSend>>, entry| -> Result<()> {
                let fname = &self.metadata.user_files[entry.id];
                let raw_dir = if entry.dir_idx < self.metadata.directories.len() {
                    &self.metadata.directories[entry.dir_idx]
                } else {
                    CURRENT_DIR_STR
                };
                // Construct the relative path. Note: `raw_dir` might contain '\' or '/'.
                // We pass this raw path to the Sink, which must handle normalization.
                let rel_path = if raw_dir == CURRENT_DIR_STR || raw_dir.is_empty() {
                    fname.clone()
                } else {
                    format!("{}/{}", raw_dir, fname)
                };

                // Create directory structure via sink
                // Note: We scan for both separators to find the parent directory
                if let Some(last_slash) = rel_path.rfind('/').or_else(|| rel_path.rfind('\\')) {
                    sink.create_dir_all(&rel_path[..last_slash])?;
                }

                let out_file = sink.create_file(&rel_path)?;
                let mut writer = BufWriter::with_capacity(DEFAULT_BUFFER_SIZE, out_file);

                for cid in &entry.chunk_ids {
                    if let Some(&idx) = chunk_indices.get(cid) {
                        let chunk = &self.processed_chunks[idx];
                        let source_file = match file_cache.entry(chunk.file_idx) {
                            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                            std::collections::hash_map::Entry::Vacant(e) => {
                                let f = if chunk.file_idx == 0 {
                                    source.open_main()?
                                } else {
                                    let split_idx = (chunk.file_idx - 1) as usize;
                                    let split_name =
                                        self.metadata.split_file_names.get(split_idx).ok_or_else(
                                            || {
                                                DzipError::Generic(format!(
                                                    "Invalid archive file index {} for chunk {}",
                                                    chunk.file_idx, chunk.id
                                                ))
                                            },
                                        )?;
                                    source.open_split(split_name)?
                                };
                                e.insert(f)
                            }
                        };
                        source_file
                            .seek(SeekFrom::Start(chunk.offset as u64))
                            .map_err(DzipError::Io)?;
                        let mut source_reader =
                            BufReader::with_capacity(DEFAULT_BUFFER_SIZE, source_file)
                                .take(chunk.real_c_len as u64);
                        if let Err(e) =
                            decompress(&mut source_reader, &mut writer, chunk.flags, chunk.d_len)
                        {
                            if keep_raw {
                                let err_msg = e.to_string();
                                let mut raw_buf_reader = source_reader.into_inner();
                                raw_buf_reader
                                    .seek(SeekFrom::Start(chunk.offset as u64))
                                    .map_err(DzipError::Io)?;
                                let mut raw_take = raw_buf_reader.take(chunk.real_c_len as u64);
                                warn!(
                                    "Failed to decompress chunk {}: {}. Writing raw data.",
                                    chunk.id, err_msg
                                );
                                std::io::copy(&mut raw_take, &mut writer).map_err(DzipError::Io)?;
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
                writer.flush().map_err(DzipError::Io)?;
                Ok(())
            },
        )?;
        Ok(())
    }

    pub fn generate_config_struct(&self) -> Result<Config> {
        let mut toml_files = Vec::new();

        for entry in &self.metadata.map_entries {
            let fname = &self.metadata.user_files[entry.id];
            let raw_dir = if entry.dir_idx < self.metadata.directories.len() {
                &self.metadata.directories[entry.dir_idx]
            } else {
                CURRENT_DIR_STR
            };

            let full_raw_path = if raw_dir == CURRENT_DIR_STR || raw_dir.is_empty() {
                fname.clone()
            } else {
                format!("{}/{}", raw_dir, fname)
            };

            toml_files.push(FileEntry {
                path: full_raw_path,
                directory: raw_dir.to_string(),
                filename: fname.clone(),
                chunks: entry.chunk_ids.clone(),
            });
        }

        let mut toml_chunks = Vec::new();
        let mut sorted_chunks = self.processed_chunks.clone();
        sorted_chunks.sort_by_key(|c| c.id);

        for c in sorted_chunks {
            let flags_list = decode_flags(c.flags)
                .into_iter()
                .map(|s| s.into_owned())
                .collect();
            toml_chunks.push(ChunkDef {
                id: c.id,
                offset: c.offset,
                size_compressed: c.real_c_len,
                size_decompressed: c.d_len,
                flags: flags_list,
                archive_file_index: c.file_idx,
            });
        }

        Ok(Config {
            archive: ArchiveMeta {
                version: self.metadata.version,
                total_files: self.metadata.map_entries.len() as u16,
                total_directories: self.metadata.directories.len() as u16,
                total_chunks: self.processed_chunks.len() as u16,
            },
            archive_files: self.metadata.split_file_names.clone(),
            range_settings: self.metadata.range_settings.clone(),
            files: toml_files,
            chunks: toml_chunks,
        })
    }
}
