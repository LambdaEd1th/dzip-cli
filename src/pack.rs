use anyhow::{Result, anyhow};
use byteorder::{LittleEndian, WriteBytesExt};
use log::{debug, info};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use crate::compression::compress_data;
use crate::constants::{CHUNK_DZ, MAGIC};
use crate::types::{ChunkDef, Config};
use crate::utils::encode_flags;

pub fn do_pack(config_path: &PathBuf) -> Result<()> {
    let toml_content = fs::read_to_string(config_path)?;
    let config: Config = toml::from_str(&toml_content)?;

    let base_dir = config_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let base_path = config_path.parent().unwrap().join(&base_dir);

    info!("Packing from directory: {:?}", base_path);

    let mut chunk_map_def: HashMap<u16, &ChunkDef> = HashMap::new();
    let mut has_dz_chunk = false;
    for c in &config.chunks {
        chunk_map_def.insert(c.id, c);
        if encode_flags(&c.flags) & CHUNK_DZ != 0 {
            has_dz_chunk = true;
        }
    }

    // Step 1: Index Source Files
    info!("Indexing source files...");
    let mut chunk_source_map: HashMap<u16, (PathBuf, u64, usize)> = HashMap::new();
    for f_entry in &config.files {
        let mut clean_rel_path = PathBuf::new();
        for part in f_entry.path.split(['/', '\\']) {
            if part == "." || part.is_empty() {
                continue;
            }
            if part == ".." {
                clean_rel_path.pop();
            } else {
                clean_rel_path.push(part);
            }
        }

        let full_path = base_path.join(clean_rel_path);

        if !full_path.exists() {
            return Err(anyhow!("Missing source: {:?}", full_path));
        }

        let mut current_offset: u64 = 0;
        for cid in &f_entry.chunks {
            let c_def = chunk_map_def.get(cid).unwrap();
            let flags = encode_flags(&c_def.flags);
            let read_len = if flags & CHUNK_DZ != 0 {
                c_def.size_compressed
            } else {
                c_def.size_decompressed
            } as usize;

            chunk_source_map.insert(*cid, (full_path.clone(), current_offset, read_len));
            current_offset += read_len as u64;
        }
    }

    // Step 2: Build Preliminary Header
    let mut unique_dirs = HashSet::new();
    for f in &config.files {
        let d = f.directory.trim();
        unique_dirs.insert(if d.is_empty() {
            "".to_string()
        } else {
            d.replace('\\', "/")
        });
    }
    if !unique_dirs.contains("") {
        unique_dirs.insert("".to_string());
    }
    let mut sorted_dirs: Vec<String> = unique_dirs.into_iter().collect();
    sorted_dirs.sort();

    if !sorted_dirs[0].is_empty() {
        sorted_dirs.insert(0, "".to_string());
    }

    let dir_map: HashMap<String, usize> = sorted_dirs
        .iter()
        .enumerate()
        .map(|(i, d)| (d.clone(), i))
        .collect();

    let mut header_buffer = Cursor::new(Vec::new());
    header_buffer.write_u32::<LittleEndian>(MAGIC)?;
    header_buffer.write_u16::<LittleEndian>(config.files.len() as u16)?;
    header_buffer.write_u16::<LittleEndian>(sorted_dirs.len() as u16)?;
    header_buffer.write_u8(0)?;

    for f in &config.files {
        header_buffer.write_all(f.filename.as_bytes())?;
        header_buffer.write_u8(0)?;
    }
    for d in sorted_dirs.iter().skip(1) {
        header_buffer.write_all(d.replace('/', "\\").as_bytes())?;
        header_buffer.write_u8(0)?;
    }
    for f in &config.files {
        let d_key = f.directory.replace('\\', "/");
        let d_id = *dir_map.get(&d_key).unwrap_or(&0) as u16;
        header_buffer.write_u16::<LittleEndian>(d_id)?;
        for cid in &f.chunks {
            header_buffer.write_u16::<LittleEndian>(*cid)?;
        }
        header_buffer.write_u16::<LittleEndian>(0xFFFF)?;
    }

    header_buffer.write_u16::<LittleEndian>((1 + config.archive_files.len()) as u16)?;
    header_buffer.write_u16::<LittleEndian>(config.chunks.len() as u16)?;

    let chunk_table_start = header_buffer.position();
    for _ in 0..config.chunks.len() {
        for _ in 0..16 {
            header_buffer.write_u8(0)?;
        }
    }

    if !config.archive_files.is_empty() {
        for fname in &config.archive_files {
            header_buffer.write_all(fname.as_bytes())?;
            header_buffer.write_u8(0)?;
        }
    }

    if has_dz_chunk {
        if let Some(rs) = &config.range_settings {
            header_buffer.write_u8(rs.win_size)?;
            header_buffer.write_u8(rs.flags)?;
            header_buffer.write_u8(rs.offset_table_size)?;
            header_buffer.write_u8(rs.offset_tables)?;
            header_buffer.write_u8(rs.offset_contexts)?;
            header_buffer.write_u8(rs.ref_length_table_size)?;
            header_buffer.write_u8(rs.ref_length_tables)?;
            header_buffer.write_u8(rs.ref_offset_table_size)?;
            header_buffer.write_u8(rs.ref_offset_tables)?;
            header_buffer.write_u8(rs.big_min_match)?;
        } else {
            for _ in 0..10 {
                header_buffer.write_u8(0)?;
            }
        }
    }

    // Step 3: Write Files
    let out_filename_0 = format!("{}_packed.dz", base_dir);
    let mut current_offset_0 = header_buffer.position() as u32;

    info!("Writing main archive: {}", out_filename_0);
    let f0 = File::create(&out_filename_0)?;
    let mut writer0 = BufWriter::new(f0);
    writer0.write_all(header_buffer.get_ref())?;

    let mut split_writers: HashMap<u16, BufWriter<File>> = HashMap::new();
    let mut split_offsets: HashMap<u16, u32> = HashMap::new();

    for (i, fname) in config.archive_files.iter().enumerate() {
        let idx = (i + 1) as u16;
        let path = config_path.parent().unwrap().join(fname);
        info!("Creating split archive: {:?}", path);
        debug!("Split file index: {}, Path: {:?}", idx, path);
        let f = File::create(&path)?;
        split_writers.insert(idx, BufWriter::new(f));
        split_offsets.insert(idx, 0);
    }

    // Step 4: Stream Data
    info!("Processing chunks (streaming)...");
    let mut sorted_chunks_def = config.chunks.clone();
    sorted_chunks_def.sort_by_key(|c| c.id);

    for c_def in &mut sorted_chunks_def {
        let (source_path, src_offset, read_len) = chunk_source_map.get(&c_def.id).unwrap();

        let mut f_in = File::open(source_path)?;
        f_in.seek(SeekFrom::Start(*src_offset))?;
        let mut buffer = vec![0u8; *read_len];
        f_in.read_exact(&mut buffer)?;

        let flags_int = encode_flags(&c_def.flags);
        let comp_data = compress_data(&buffer, flags_int)?;
        let comp_len = comp_data.len() as u32;

        c_def.offset = if c_def.archive_file_index == 0 {
            current_offset_0
        } else {
            *split_offsets.get(&c_def.archive_file_index).unwrap()
        };
        c_def.size_compressed = comp_len;

        if c_def.archive_file_index == 0 {
            writer0.write_all(&comp_data)?;
            current_offset_0 += comp_len;
        } else {
            let w = split_writers.get_mut(&c_def.archive_file_index).unwrap();
            w.write_all(&comp_data)?;
            *split_offsets.get_mut(&c_def.archive_file_index).unwrap() += comp_len;
        }
    }

    writer0.flush()?;
    for w in split_writers.values_mut() {
        w.flush()?;
    }

    // Step 5: Finalize Header
    info!("Finalizing header...");
    let mut table_writer = Cursor::new(Vec::new());
    for c in &sorted_chunks_def {
        table_writer.write_u32::<LittleEndian>(c.offset)?;
        table_writer.write_u32::<LittleEndian>(c.size_compressed)?;
        table_writer.write_u32::<LittleEndian>(c.size_decompressed)?;
        table_writer.write_u16::<LittleEndian>(encode_flags(&c.flags))?;
        table_writer.write_u16::<LittleEndian>(c.archive_file_index)?;
    }

    writer0.seek(SeekFrom::Start(chunk_table_start))?;
    writer0.write_all(table_writer.get_ref())?;
    writer0.flush()?;

    info!("All files packed successfully.");
    Ok(())
}
