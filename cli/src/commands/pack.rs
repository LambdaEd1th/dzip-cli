use crate::config;
use dzip_core::format::{ArchiveSettings, CHUNK_DZ, Chunk, ChunkSettings, RangeSettings};
use dzip_core::{Result, compress_data};
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, info};
use rayon::prelude::*;
use std::io::{Seek, SeekFrom, Write};

pub fn pack_archive(input_path: &str, output_dir: &str) -> Result<()> {
    let config_path = std::path::Path::new(input_path);
    info!("Parsing config file: {}", config_path.display());
    let mut config = config::parse_config(config_path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // If base_dir is "." (default), make it relative to the config file's directory
    #[allow(clippy::collapsible_if)]
    if config.base_dir == std::path::Path::new(".") {
        if let Some(parent) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            config.base_dir = parent.to_path_buf();
        }
    }

    std::fs::create_dir_all(output_dir)?;

    // --- Prepare Metadata ---
    // 1. Strings: User Files + Unique Directories
    // Note: Dzip strings table contains filenames (basename) and directory paths.
    // The exact structure is: [List of User Filenames], [List of Directory Paths].
    // Wait, the format in unpacking:
    // strings = reader.read_strings(settings.num_user_files + settings.num_directories - 1)?
    // And map points to dir_index.
    // So strings table is: [file1_name, file2_name, ..., dir1_path, dir2_path, ...].
    // Note root dir is implicit/empty and usually not in strings table?
    // Unpacker: `dir_index = num_user_files + (dir_id - 1)`.
    // If dir_id=1, index = num_user_files.
    // So yes, strings list is [Files..., Dir1, Dir2...].

    // Collect File Names
    let mut file_names = Vec::new();
    for entry in &config.files {
        // Use filename component
        if let Some(name) = entry.path.file_name() {
            file_names.push(name.to_string_lossy().to_string());
        } else {
            return Err(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid file path").into(),
            );
        }
    }

    // Collect Unique Directories and assign IDs
    let mut directories = Vec::new();
    let mut dir_map = std::collections::HashMap::new(); // path -> dir_id (1-based)

    // Directory ID 0 is Root.
    // We need to map each file to a dir_id.
    let mut file_dir_ids = Vec::new();

    for entry in &config.files {
        let parent = entry.path.parent().unwrap_or(std::path::Path::new(""));
        // Force Windows-style backslashes as requested using core utility
        let parent_str = dzip_core::path::to_archive_format(parent);

        if parent_str.is_empty() || parent_str == "." {
            file_dir_ids.push(0u16);
        } else {
            // Check if known
            if let Some(&id) = dir_map.get(&parent_str) {
                file_dir_ids.push(id);
            } else {
                // New directory
                // Directories list stores paths.
                directories.push(parent_str.clone());
                let id = directories.len() as u16; // 1-based
                dir_map.insert(parent_str, id);
                file_dir_ids.push(id);
            }
        }
    }

    let num_user_files = file_names.len() as u16;
    let num_directories = (directories.len() + 1) as u16; // +1 for Root?
    // Unpacker: `strings_count = num_user_files + num_directories - 1`.
    // So strings count = files + dirs.
    // Strings array = [Files..., Dirs...].
    // Root dir is NOT in strings.

    let mut all_strings = file_names;
    all_strings.extend(directories);

    // --- Open Volumes ---
    if config.archives.is_empty() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "No archives specified").into(),
        );
    }

    let mut writers = std::collections::HashMap::new();
    for (i, name) in config.archives.iter().enumerate() {
        let path = std::path::Path::new(output_dir).join(name);
        info!("Opening volume {}: {}", i, path.display());
        let f = std::fs::File::create(&path)?;
        writers.insert(i as u16, f);
    }

    // --- Pre-calculate Header Size (Volume 0) ---
    // Header (ArchiveSettings) = 4+2+2+1 = 9
    // Strings = Sum(len+1)
    // FileMap (ChunkMap) = NumFiles * (2 + NumChunksInFile*2 + 2)
    // ChunkSettings = 2+2=4
    // ChunkTable = NumChunks * 16
    // Auxiliary File List = Sum(len+1) of archives[1..]

    // Assuming 1 chunk per file
    let num_chunks = num_user_files;

    let mut header_size = 9;
    for s in &all_strings {
        header_size += s.len() as u64 + 1;
    }
    let file_map_size = (num_user_files as u64) * 6; // DirID(2) + ChunkID(2) + Term(2)
    header_size += file_map_size;

    header_size += 4; // ChunkSettings
    let chunk_table_size = (num_chunks as u64) * 16;
    header_size += chunk_table_size;

    // Add Volume List Size
    if config.archives.len() > 1 {
        for name in &config.archives[1..] {
            header_size += name.len() as u64 + 1;
        }
    }

    // Should we add GlobalSettings size? Only if we use DZ compression.
    // Config options might specify usage. For now assume minimal header.
    // We will update this offset if needed.

    // Seek Volume 0
    if let Some(w) = writers.get_mut(&0) {
        w.seek(SeekFrom::Start(header_size))?;
    }

    // --- Process Files and Write Chunks ---
    let mut chunks = Vec::new();
    let mut chunk_map = Vec::new(); // (dir_id, vec![chunk_id])

    // Parallel Compression Phase
    info!("Compressing chunks in parallel...");
    let pb = ProgressBar::new(config.files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let processed_files: Vec<(u16, Vec<u8>, usize, u16)> = config
        .files
        .par_iter()
        .enumerate()
        .map(|(i, entry)| {
            let full_path = config.base_dir.join(&entry.path);
            debug!("Processing file {}: {}", i, full_path.display());
            pb.set_message(format!("Compressing {}", entry.path.display()));

            let raw_data = std::fs::read(&full_path).map_err(|e| {
                dzip_core::DzipError::Io(std::io::Error::other(format!(
                    "Failed to read {}: {}",
                    full_path.display(),
                    e
                )))
            })?;
            let original_len = raw_data.len();

            let method = entry.compression;
            let (flags, compressed_data) = compress_data(&raw_data, method)?;

            pb.inc(1);
            Ok((
                entry.archive_file_index,
                compressed_data,
                original_len,
                flags,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    pb.finish_with_message("Compression complete");

    // Sequential Write Phase
    info!("Writing compressed chunks to volumes...");
    for (i, (archive_id, compressed_data, original_len, flags)) in
        processed_files.into_iter().enumerate()
    {
        let chunk_id = chunks.len() as u16;

        let writer = writers.get_mut(&archive_id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Archive volume {} not found in config", archive_id),
            )
        })?;

        let offset = writer.stream_position()? as u32;
        writer.write_all(&compressed_data)?;

        chunks.push(Chunk {
            offset,
            compressed_length: compressed_data.len() as u32,
            decompressed_length: original_len as u32,
            flags,
            file: archive_id,
        });

        chunk_map.push((file_dir_ids[i], vec![chunk_id]));
    }

    // --- Write Header ---
    info!("Writing header to Volume 0...");
    let main_writer = writers
        .get_mut(&0)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "Volume 0 missing"))?;

    main_writer.seek(SeekFrom::Start(0))?;

    // We need DzipWriter
    struct SimpleWriter<'a, W: Write + Seek>(&'a mut W);
    impl<'a, W: Write + Seek> Write for SimpleWriter<'a, W> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    impl<'a, W: Write + Seek> Seek for SimpleWriter<'a, W> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(pos)
        }
    }

    let mut dzip_writer = dzip_core::writer::DzipWriter::new(SimpleWriter(main_writer));

    // ... rest of header writing ...

    dzip_writer.write_archive_settings(&ArchiveSettings {
        header: 0x5A525444, // DTRZ
        num_user_files,
        num_directories,
        version: 0,
    })?;

    // ...

    dzip_writer.write_strings(&all_strings)?;
    dzip_writer.write_file_chunk_map(&chunk_map)?;

    // ...

    let num_archive_files = config.archives.len() as u16;

    dzip_writer.write_chunk_settings(&ChunkSettings {
        num_archive_files,
        num_chunks: chunks.len() as u16,
    })?;

    dzip_writer.write_chunks(&chunks)?;

    // Write Auxiliary File List
    if config.archives.len() > 1 {
        let aux_files = &config.archives[1..];
        dzip_writer.write_strings(aux_files)?;
    }

    let has_dz = chunks.iter().any(|c| (c.flags & CHUNK_DZ) != 0);
    if has_dz {
        dzip_writer.write_global_settings(&RangeSettings {
            win_size: 0,
            flags: 0,
            offset_table_size: 0,
            offset_tables: 0,
            offset_contexts: 0,
            ref_length_table_size: 0,
            ref_length_tables: 0,
            ref_offset_table_size: 0,
            ref_offset_tables: 0,
            big_min_match: 0,
        })?;
    }

    info!("Pack complete.");
    Ok(())
}
