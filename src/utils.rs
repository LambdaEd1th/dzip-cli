use crate::constants::ChunkFlags; // [Refactor] Import ChunkFlags
use anyhow::{Result, anyhow};
use std::borrow::Cow;
use std::io::BufRead;
use std::path::{Component, Path, PathBuf};

// [Optimization] Return Vec<Cow<'static, str>> to avoid String allocations.
// [Refactor] Use ChunkFlags to parse the u16 bits.
pub fn decode_flags(bits: u16) -> Vec<Cow<'static, str>> {
    let flags = ChunkFlags::from_bits_truncate(bits);
    let mut list = Vec::new();

    if flags.is_empty() {
        list.push(Cow::Borrowed("COPY"));
        return list;
    }

    // [Refactor] Use .contains() for readability and safety
    if flags.contains(ChunkFlags::COMBUF) {
        list.push(Cow::Borrowed("COMBUF"));
    }
    if flags.contains(ChunkFlags::DZ_RANGE) {
        list.push(Cow::Borrowed("DZ_RANGE"));
    }
    if flags.contains(ChunkFlags::ZLIB) {
        list.push(Cow::Borrowed("ZLIB"));
    }
    if flags.contains(ChunkFlags::BZIP) {
        list.push(Cow::Borrowed("BZIP"));
    }
    if flags.contains(ChunkFlags::MP3) {
        list.push(Cow::Borrowed("MP3"));
    }
    if flags.contains(ChunkFlags::JPEG) {
        list.push(Cow::Borrowed("JPEG"));
    }
    if flags.contains(ChunkFlags::ZERO) {
        list.push(Cow::Borrowed("ZERO"));
    }
    if flags.contains(ChunkFlags::COPYCOMP) {
        list.push(Cow::Borrowed("COPY"));
    }
    if flags.contains(ChunkFlags::LZMA) {
        list.push(Cow::Borrowed("LZMA"));
    }
    if flags.contains(ChunkFlags::RANDOMACCESS) {
        list.push(Cow::Borrowed("RANDOM_ACCESS"));
    }

    list
}

// [Refactor] Construct ChunkFlags from strings and return the raw u16 bits.
pub fn encode_flags<S: AsRef<str>>(flags_vec: &[S]) -> u16 {
    let mut res = ChunkFlags::empty();

    if flags_vec.is_empty() {
        return res.bits();
    }

    for f in flags_vec {
        match f.as_ref() {
            "COMBUF" => res.insert(ChunkFlags::COMBUF),
            "DZ_RANGE" => res.insert(ChunkFlags::DZ_RANGE),
            "ZLIB" => res.insert(ChunkFlags::ZLIB),
            "BZIP" => res.insert(ChunkFlags::BZIP),
            "MP3" => res.insert(ChunkFlags::MP3),
            "JPEG" => res.insert(ChunkFlags::JPEG),
            "ZERO" => res.insert(ChunkFlags::ZERO),
            "COPY" => res.insert(ChunkFlags::COPYCOMP),
            "LZMA" => res.insert(ChunkFlags::LZMA),
            "RANDOM_ACCESS" => res.insert(ChunkFlags::RANDOMACCESS),
            _ => {}
        }
    }

    // Explicitly handle "COPY" string which maps to COPYCOMP or empty
    if res.is_empty() && flags_vec.iter().any(|f| f.as_ref() == "COPY") {
        res.insert(ChunkFlags::COPYCOMP);
    }

    res.bits()
}

pub fn read_null_term_string<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut bytes = Vec::new();
    reader.read_until(0, &mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub fn sanitize_path(base: &Path, rel_path_str: &str) -> Result<PathBuf> {
    let normalized = rel_path_str.replace('\\', "/");
    let rel_path = Path::new(&normalized);
    
    let mut safe_path = PathBuf::new();

    for component in rel_path.components() {
        match component {
            Component::Normal(os_str) => safe_path.push(os_str),
            Component::ParentDir => {
                return Err(anyhow!(
                    "Security Error: Directory traversal (..) detected in path: {}",
                    rel_path_str
                ));
            }
            Component::RootDir => continue,
            Component::Prefix(_) => {
                return Err(anyhow!(
                    "Security Error: Absolute path or drive letter detected: {}",
                    rel_path_str
                ));
            }
            Component::CurDir => continue,
        }
    }

    if safe_path.as_os_str().is_empty() {
        return Err(anyhow!("Invalid empty path resolution: {}", rel_path_str));
    }

    Ok(base.join(safe_path))
}