use dzip_core::Result;
use log::error;
use rayon::prelude::*;

pub fn verify_archive(input_path: &str) -> Result<()> {
    // use dzip_core::format::*; // don't import everything, be explicit if needed, but here symbols are used

    let mut reader = dzip_core::reader::DzipReader::new(
        std::fs::File::open(input_path).map_err(dzip_core::DzipError::Io)?,
    );

    let settings = reader.read_archive_settings()?;

    // Read strings (filenames + dirnames)
    // Formula: num_user_files + num_directories - 1
    let strings_count = (settings.num_user_files + settings.num_directories - 1) as usize;
    let strings = reader.read_strings(strings_count)?;

    // Read FileChunkMap
    let map = reader.read_file_chunk_map(settings.num_user_files as usize)?;

    // We need chunk headers to get sizes
    let chunk_settings = reader.read_chunk_settings()?;
    let mut chunks = reader.read_chunks(chunk_settings.num_chunks as usize)?;

    // Read Auxiliary Files (Volumes)
    let num_volumes_expected = chunk_settings.num_archive_files.saturating_sub(1);
    let volume_files = if num_volumes_expected > 0 {
        reader.read_strings(num_volumes_expected as usize)?
    } else {
        Vec::new()
    };

    // Prepare shared data for VolumeManager
    let input_base_dir = std::path::Path::new(input_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let input_base_dir_shared = input_base_dir.to_path_buf();
    let volume_files_shared = volume_files.clone();

    // Use FileSystemVolumeManager
    // Note: Creating one here just to read file sizes.
    // Actually we don't need VolumeManager logic for size calc, can just iterate strings.
    // But we need it for verification loop later.

    // --- Chunk Size Correction ---
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

    println!("Verifying archive integrity...");

    println!(
        "{:<5} | {:<7} | {:<10} | {:<10} | {:<8} | Path",
        "Idx", "Status", "Size", "Packed", "Method"
    );
    println!(
        "{:-<5}-+-{:-<7}-+-{:-<10}-+-{:-<10}-+-{:-<8}-+-{:-<20}",
        "", "", "", "", "", ""
    );

    // Use parallel iterator to verify
    // We need to collect results to print them in order (or we could print as we go if we didn't care about order, but table looks best ordered)
    // Order is important for "Idx".

    let results: Vec<String> = map
        .par_iter()
        .enumerate()
        .map(|(i, (dir_id, chunk_ids))| -> Result<String> {
            let file_name = &strings[i];

            // Reconstruct path
            let mut full_path = String::new();
            if *dir_id > 0 {
                let dir_index = settings.num_user_files as usize + (*dir_id as usize) - 1;
                if let Some(dir_name) = strings.get(dir_index) {
                    full_path.push_str(dir_name);
                    if !full_path.ends_with('/') && !full_path.ends_with('\\') {
                        full_path.push('/');
                    }
                }
            }
            full_path.push_str(file_name);

            // Calculate sizes
            let mut size = 0;
            let mut packed = 0;
            let mut method_str = "Unknown";

            use dzip_core::format::*;
            if let Some(&first_chunk_id) = chunk_ids.first() {
                let chunk = &chunks[first_chunk_id as usize];
                // Determine method from first chunk
                if (chunk.flags & CHUNK_ZLIB) != 0 {
                    method_str = "Zlib";
                } else if (chunk.flags & CHUNK_BZIP) != 0 {
                    method_str = "Bzip";
                } else if (chunk.flags & CHUNK_LZMA) != 0 {
                    method_str = "LZMA";
                } else if (chunk.flags & CHUNK_COPYCOMP) != 0 {
                    method_str = "Copy";
                } else if (chunk.flags & CHUNK_ZERO) != 0 {
                    method_str = "Zero";
                } else if (chunk.flags & CHUNK_DZ) != 0 {
                    method_str = "Dz";
                }
            }

            // Verify integrity
            // We need a local DzipReader and VolumeManager
            let main_file = std::fs::File::open(input_path).map_err(dzip_core::DzipError::Io)?;
            let mut local_reader = dzip_core::reader::DzipReader::new(main_file);

            let mut volume_manager = dzip_core::volume::FileSystemVolumeManager::new(
                input_base_dir_shared.clone(),
                volume_files_shared.clone(),
            );

            let mut chunk_status = "OK";
            for &chunk_id in chunk_ids {
                if let Some(chunk) = chunks.get(chunk_id as usize) {
                    if let Err(_e) =
                        local_reader.read_chunk_data_with_volumes(chunk, &mut volume_manager)
                    {
                        // Log error but return FAIL string
                        error!("Chunk {} failed verification: {}", chunk_id, _e);
                        chunk_status = "FAIL";
                    }
                } else {
                    chunk_status = "FAIL";
                }
            }
            let status = chunk_status;

            for &cid in chunk_ids {
                let chunk = &chunks[cid as usize];
                size += chunk.decompressed_length;
                packed += chunk.compressed_length;
            }

            Ok(format!(
                "{:<5} | {:<7} | {:<10} | {:<10} | {:<8} | {}",
                i, status, size, packed, method_str, full_path
            ))
        })
        .collect::<Result<Vec<String>>>()?;

    for line in results {
        println!("{}", line);
    }

    Ok(())
}
