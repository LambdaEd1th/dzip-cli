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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Component, Path};

    #[test]
    fn test_sanitize_path_security() {
        let base = Path::new("/tmp/sandbox");

        // 1. Normal path: should succeed and append to base
        let res = sanitize_path(base, "textures/player.png");
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), base.join("textures").join("player.png"));

        // 2. Directory traversal attack (ParentDir): must return error
        let res = sanitize_path(base, "../etc/passwd");
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(
            err_msg.contains("Directory traversal"),
            "Should detect directory traversal"
        );

        // 3. Embedded directory traversal: must return error
        let res = sanitize_path(base, "images/../../secret.txt");
        assert!(res.is_err());

        // 4. Absolute path prefix (Windows): must return error
        // Simulate Windows drive letter path for testing
        let path_str = std::path::Path::new("C:\\Windows\\System32");
        if path_str
            .components()
            .any(|c| matches!(c, Component::Prefix(_)))
        {
            let res = sanitize_path(base, "C:\\Windows\\System32");
            assert!(res.is_err());
        }
    }

    #[test]
    fn test_sanitize_path_root_handling() {
        let base = Path::new("/app/data");

        // 5. Starts with root directory: Logic ignores RootDir, treating it as relative
        // Input "/var/log" -> should become "/app/data/var/log" (prevent escaping base)
        let res = sanitize_path(base, "/var/log");
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), base.join("var").join("log"));
    }

    #[test]
    fn test_flags_roundtrip() {
        // Test consistency of Flag encoding and decoding
        let flags = vec!["LZMA".to_string(), "ZLIB".to_string(), "MP3".to_string()];

        // Encode
        let encoded = encode_flags(&flags);
        assert_eq!(encoded & CHUNK_LZMA, CHUNK_LZMA);
        assert_eq!(encoded & CHUNK_ZLIB, CHUNK_ZLIB);
        assert_eq!(encoded & CHUNK_MP3, CHUNK_MP3);

        // Decode
        let decoded = decode_flags(encoded);
        assert!(decoded.contains(&"LZMA".to_string()));
        assert!(decoded.contains(&"ZLIB".to_string()));
        assert!(decoded.contains(&"MP3".to_string()));

        // Ensure no extra flags
        assert!(!decoded.contains(&"BZIP".to_string()));
    }

    #[test]
    fn test_empty_flags() {
        let empty: Vec<String> = Vec::new();
        let encoded = encode_flags(&empty);
        assert_eq!(encoded, 0); // 0 defaults to COPY

        let decoded = decode_flags(0);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0], "COPY");
    }
}
