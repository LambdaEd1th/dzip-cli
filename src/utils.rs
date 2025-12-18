use crate::constants::*;
use anyhow::{Result, anyhow};
use std::io::BufRead;
use std::path::{Component, Path, PathBuf};

pub fn decode_flags(flags: u16) -> Vec<String> {
    let mut list = Vec::new();
    if flags == 0 {
        list.push("COPY".to_string());
        return list;
    }
    if flags & CHUNK_COMBUF != 0 {
        list.push("COMBUF".to_string());
    }
    if flags & CHUNK_DZ != 0 {
        list.push("DZ_RANGE".to_string());
    }
    if flags & CHUNK_ZLIB != 0 {
        list.push("ZLIB".to_string());
    }
    if flags & CHUNK_BZIP != 0 {
        list.push("BZIP".to_string());
    }
    if flags & CHUNK_MP3 != 0 {
        list.push("MP3".to_string());
    }
    if flags & CHUNK_JPEG != 0 {
        list.push("JPEG".to_string());
    }
    if flags & CHUNK_ZERO != 0 {
        list.push("ZERO".to_string());
    }
    if flags & CHUNK_COPYCOMP != 0 {
        list.push("COPY".to_string());
    }
    if flags & CHUNK_LZMA != 0 {
        list.push("LZMA".to_string());
    }
    if flags & CHUNK_RANDOMACCESS != 0 {
        list.push("RANDOM_ACCESS".to_string());
    }
    list
}

pub fn encode_flags(flags_vec: &[String]) -> u16 {
    let mut res = 0;
    if flags_vec.is_empty() {
        return 0;
    }
    for f in flags_vec {
        match f.as_str() {
            "COMBUF" => res |= CHUNK_COMBUF,
            "DZ_RANGE" => res |= CHUNK_DZ,
            "ZLIB" => res |= CHUNK_ZLIB,
            "BZIP" => res |= CHUNK_BZIP,
            "MP3" => res |= CHUNK_MP3,
            "JPEG" => res |= CHUNK_JPEG,
            "ZERO" => res |= CHUNK_ZERO,
            "COPY" => res |= CHUNK_COPYCOMP,
            "LZMA" => res |= CHUNK_LZMA,
            "RANDOM_ACCESS" => res |= CHUNK_RANDOMACCESS,
            _ => {}
        }
    }
    if res == 0 && flags_vec.contains(&"COPY".to_string()) {
        res |= CHUNK_COPYCOMP;
    }
    res
}

pub fn read_null_term_string<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut bytes = Vec::new();
    reader.read_until(0, &mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

// Security enhanced version
pub fn sanitize_path(base: &Path, rel_path_str: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel_path_str);
    let mut safe_path = PathBuf::new();

    for component in rel_path.components() {
        match component {
            // 1. Normal filename: safe, append to path
            Component::Normal(os_str) => safe_path.push(os_str),
            // 2. Parent directory (".."): extremely dangerous, must be intercepted
            Component::ParentDir => {
                return Err(anyhow!(
                    "Security Error: Directory traversal (..) detected in path: {}",
                    rel_path_str
                ));
            }
            // 3. Root directory ("/"): ignore
            Component::RootDir => continue,
            // 4. Drive prefix (e.g. "C:"): absolute path or drive letter, forbidden
            Component::Prefix(_) => {
                return Err(anyhow!(
                    "Security Error: Absolute path or drive letter detected: {}",
                    rel_path_str
                ));
            }
            // 5. Current directory ("."): ignore
            Component::CurDir => continue,
        }
    }

    if safe_path.as_os_str().is_empty() {
        return Err(anyhow!("Invalid empty path resolution: {}", rel_path_str));
    }

    Ok(base.join(safe_path))
}
