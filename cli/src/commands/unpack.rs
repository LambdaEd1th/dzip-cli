use crate::config;
use dzip_core::Result;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, error, info, warn};
use rayon::prelude::*;

pub fn unpack_archive(input_path: &str, output_dir: &str) -> Result<()> {
    let file = std::fs::File::open(input_path)?;
    let mut reader = dzip_core::reader::DzipReader::new(file);

    info!("Reading archive metadata...");
    let settings = reader.read_archive_settings()?;

    // Determine string count (handling implicit root directory)
    let strings_count = (settings.num_user_files + settings.num_directories - 1) as usize;
    let strings = reader.read_strings(strings_count)?;

    let map = reader.read_file_chunk_map(settings.num_user_files as usize)?;
    let chunk_settings = reader.read_chunk_settings()?;
    let mut chunks = reader.read_chunks(chunk_settings.num_chunks as usize)?;

    // Read file list (if multi-volume)
    let num_other_volumes = if chunk_settings.num_archive_files > 0 {
        chunk_settings.num_archive_files as usize - 1
    } else {
        0
    };
    let volume_files = reader.read_file_list(num_other_volumes)?;
    debug!(
        "Num archive files: {}, Volume List: {:?}",
        chunk_settings.num_archive_files, volume_files
    );

    info!(
        "Extracting {} files to '{}'...",
        settings.num_user_files, output_dir
    );
    std::fs::create_dir_all(output_dir)?;

    let mut archives_names = vec![
        std::path::Path::new(input_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
    ];
    archives_names.extend(volume_files.clone());

    use dzip_core::format::CHUNK_DZ;
    let has_dz_chunks = chunks.iter().any(|c| (c.flags & CHUNK_DZ) != 0);

    let global_options = if has_dz_chunks {
        let settings = reader.read_global_settings()?;
        Some(config::GlobalOptions {
            win_size: settings.win_size,
            offset_table_size: settings.offset_table_size,
            offset_tables: settings.offset_tables,
            offset_contexts: settings.offset_contexts,
            ref_length_table_size: settings.ref_length_table_size,
            ref_length_tables: settings.ref_length_tables,
            ref_offset_table_size: settings.ref_offset_table_size,
            ref_offset_tables: settings.ref_offset_tables,
            big_min_match: settings.big_min_match,
            ..config::GlobalOptions::default()
        })
    } else {
        None
    };

    let mut pack_config = config::DzipConfig {
        archives: archives_names,
        base_dir: std::path::PathBuf::from("."),
        files: Vec::new(),
        options: global_options,
    };

    // Prepare shared data for parallel execution
    let settings_num_user_files = settings.num_user_files;
    let volume_files_shared = volume_files.clone(); // Clone vec from manager
    let input_base_dir = std::path::Path::new(input_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let input_base_dir_shared = input_base_dir.to_path_buf();

    // --- Chunk Size Correction ---
    // Some archives (like testnew.dz) have incorrect compressed_length headers (listing uncompressed size).
    // Validity check: compressed_length cannot exceed distance to next chunk or EOF.
    let mut file_sizes = std::collections::HashMap::new();
    if let Ok(meta) = std::fs::metadata(input_path) {
        file_sizes.insert(0u16, meta.len());
    }
    for (i, vol_name) in volume_files.iter().enumerate() {
        let path = input_base_dir.join(vol_name);
        if let Ok(meta) = std::fs::metadata(&path) {
            file_sizes.insert((i + 1) as u16, meta.len());
        }
    }
    dzip_core::reader::correct_chunk_sizes(&mut chunks, &file_sizes);
    // -----------------------------

    info!("Extracting {} files to '{}'...", map.len(), output_dir);
    let pb = ProgressBar::new(map.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    // We need to collect file entries for config *after* parallel execution or use a mutex.
    // Collecting results is better.
    // Result type: (FileEntry, Vec<String>) where Vec<String> are log messages? No, just log directly or return errors.
    // Actually, we need to generate `pack_config.files`.

    let results: Vec<config::FileEntry> = map
        .par_iter()
        .enumerate()
        .map(|(i, (dir_id, chunk_ids))| -> Result<config::FileEntry> {
            pb.inc(1);
            let file_name = &strings[i];

            // Actually, we should construct the full path string first, then resolve it.
            // But we have `relative_path_buf` which is built using `push`.
            // If `dir_name` contains `\`, `push` treats it as a filename on Unix.
            // So `relative_path_buf` might be "dir\subdir/filename" on Unix.

            // We should append components carefully?
            // Or just use string builder for the "archive path" and then resolve.

            // Best approach:
            // 1. Reconstruct the full "archive path string" (using / or \ as per archive, likely mixed)
            // 2. Pass that string to `resolve_relative_path`

            let mut full_archive_path = String::new();
            if *dir_id > 0 {
                // dir_id 0 is root.
                let dir_index = settings_num_user_files as usize + (*dir_id as usize) - 1;
                if dir_index < strings.len() {
                    let dir_name = &strings[dir_index];
                    full_archive_path.push_str(dir_name);
                    // Ensure separator if missing
                    if !full_archive_path.ends_with('/') && !full_archive_path.ends_with('\\') {
                        full_archive_path.push('\\'); // Use archive default separator
                    }
                }
            }
            full_archive_path.push_str(file_name);

            // Normalize path using dzip-core path handling (Platform Aware)
            let sanitized_path = dzip_core::path::resolve_relative_path(&full_archive_path)?;
            let full_out_path = std::path::Path::new(output_dir).join(&sanitized_path);

            // Sanity check: ensure it is still within output_dir?
            // sanitize_path returns a relative path without `..` so joining it to output_dir is safe.

            // Relative path for config
            let relative_path = sanitized_path.clone();

            // Use sanitized path for creation
            if let Some(parent) = full_out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // info!("Extracting: {}", file_name); // Valid input, but too detailed for parallel log? PB shows progress.

            let mut out_file = std::fs::File::create(&full_out_path)?;

            // Thread-local VolumeManager
            let mut volume_manager = dzip_core::volume::FileSystemVolumeManager::new(
                input_base_dir_shared.clone(),
                volume_files_shared.clone(),
            );

            // Also need local DzipReader for Main Volume (ID 0)
            // But VolumeManager handles ID > 0.
            // ID 0 chunks must be read from MAIN file.
            // DzipReader::read_chunk_data_with_volumes handles this?
            // "if chunk.file == 0 { self.read_chunk_data(chunk) }"
            // So we need a DzipReader for `self`.
            let main_file = std::fs::File::open(input_path).map_err(dzip_core::DzipError::Io)?;
            let mut reader = dzip_core::reader::DzipReader::new(main_file);

            // Determine compression from the first chunk
            use dzip_core::CompressionMethod;
            let mut compression = CompressionMethod::Dz; // Default
            let mut archive_index = 0;
            if let Some(&first_chunk_id) = chunk_ids.first() {
                let chunk = &chunks[first_chunk_id as usize];
                archive_index = chunk.file;

                use dzip_core::format::*;
                if (chunk.flags & CHUNK_ZLIB) != 0 {
                    compression = CompressionMethod::Zlib;
                } else if (chunk.flags & CHUNK_BZIP) != 0 {
                    compression = CompressionMethod::Bzip;
                } else if (chunk.flags & CHUNK_COPYCOMP) != 0 {
                    compression = CompressionMethod::Copy;
                } else if (chunk.flags & CHUNK_ZERO) != 0 {
                    compression = CompressionMethod::Zero;
                } else if (chunk.flags & CHUNK_MP3) != 0 {
                    compression = CompressionMethod::Mp3;
                } else if (chunk.flags & CHUNK_JPEG) != 0 {
                    compression = CompressionMethod::Jpeg;
                } else if (chunk.flags & CHUNK_LZMA) != 0 {
                    compression = CompressionMethod::Lzma;
                } else if (chunk.flags & CHUNK_DZ) != 0 {
                    compression = CompressionMethod::Dz;
                } else if (chunk.flags & CHUNK_COMBUF) != 0 {
                    compression = CompressionMethod::Combuf;
                } else if (chunk.flags & CHUNK_RANDOMACCESS) != 0 {
                    compression = CompressionMethod::RandomAccess;
                }
            }

            for &chunk_id in chunk_ids {
                let chunk = &chunks[chunk_id as usize];
                /*
                debug!(
                    "Chunk {} - Offset: {}, CompLen: {}, DecompLen: {}, File: {}, Flags: {:#x}",
                    chunk_id,
                    chunk.offset,
                    chunk.compressed_length,
                    chunk.decompressed_length,
                    chunk.file,
                    chunk.flags
                );
                */
                match reader.read_chunk_data_with_volumes(chunk, &mut volume_manager) {
                    Ok(data) => {
                        use std::io::Write;
                        out_file.write_all(&data)?;
                    }
                    Err(dzip_core::DzipError::UnsupportedCompression(flags)) => {
                        warn!(
                            "Skipping chunk {} due to unsupported compression (flags: {:#x})",
                            chunk_id, flags
                        );
                    }
                    Err(_e) => {
                        error!("Error extracting chunk {}: {}", chunk_id, _e);
                        // Continue? Or fail? Currently continue.
                        continue;
                    }
                }
            }

            Ok(config::FileEntry {
                path: relative_path,
                archive_file_index: archive_index,
                compression,
                modifiers: String::new(),
            })
        })
        .collect::<Result<Vec<config::FileEntry>>>()?;

    pack_config.files = results;

    // Write config file
    let input_name = std::path::Path::new(input_path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let config_filename = format!("{}.toml", input_name);
    let config_path = std::path::Path::new(output_dir).join(config_filename);
    let toml_string = toml::to_string_pretty(&pack_config).expect("Failed to serialize config");
    std::fs::write(config_path, toml_string)?;

    pb.finish_with_message("Unpack complete");
    info!("Unpack complete.");
    Ok(())
}
