# dzip-core

**dzip-core** is the library crate powering `dzip-cli`, providing the fundamental logic for reading, writing, and analyzing **Marmalade SDK** resource archives (`.dz` / `.dzip`).

It is designed as a pure **mechanism** layer. It handles the binary format, compression algorithms, and parallel processing, while abstracting filesystem operations via traits (`Source` and `Sink`). This allows it to be used in various contexts (CLI, GUI, or network services) without being tied to `std::fs`.

## ✨ Features

* **🚀 Parallel Processing**: Heavy lifting (compression/decompression) is powered by `rayon`, utilizing a producer-consumer pipeline model for maximum throughput.
* **🔌 I/O Agnostic**: Built on `PackSource`/`PackSink` and `UnpackSource`/`UnpackSink` traits. The library doesn't enforce how files are read or written—you define the policy.
* **🛡️ Robust Parsing**: Contains logic to fix legacy header errors (e.g., incorrect ZSIZE fields) and handles multi-volume split archives transparently.
* **🗜️ Supported Algorithms**:
* LZMA (Legacy 13-byte header)
* ZLIB (RFC 1951)
* BZIP2
* Store (Raw copy)



## 📦 Installation

Add `dzip-core` to your `Cargo.toml`:

```toml
[dependencies]
dzip-core = { path = "crates/core" }

```

## 📖 Architecture & Usage

Unlike traditional libraries that operate directly on paths, `dzip-core` requires the consumer to implement IO traits. This separates the **logic** (parsing/compressing) from the **environment** (filesystem/memory).

### 1. The IO Traits

Before calling high-level functions, you interact with these traits defined in `dzip_core::io`:

* **`UnpackSource`**: Provides read access to the archive (main file and split volumes).
* **`UnpackSink`**: Handles the creation of extracted files and directories.
* **`PackSource`**: Provides read access to the raw files being packed.
* **`PackSink`**: Handles writing the final archive chunks to the destination.

### 2. Unpacking Example

To unpack an archive, implement `UnpackSource` and `UnpackSink`, then call `do_unpack`.

```rust
use dzip_core::{do_unpack, Result};
use dzip_core::io::{UnpackSource, UnpackSink, ReadSeekSend, WriteSend};

// 1. Implement your Source (e.g., reading from FS)
struct MyFileSource { /* ... */ }
impl UnpackSource for MyFileSource { /* ... */ }

// 2. Implement your Sink (e.g., writing to FS)
struct MyFileSink { /* ... */ }
impl UnpackSink for MyFileSink { /* ... */ }

fn main() -> Result<()> {
    let source = MyFileSource::new("assets/data.dz");
    let sink = MyFileSink::new("output_dir");
    let keep_raw = false; // Dump raw bytes if decompression fails?

    // 3. Execute logic
    // Returns a Config struct describing the extracted content
    // The last argument is a progress callback: |event| {}
    let config = do_unpack(&source, &sink, keep_raw, |_| {})?;
    
    println!("Unpacked {} files.", config.files.len());
    Ok(())
}

```

### 3. Packing Example

To pack files, create a `Config` object (usually generated from the unpack step) and call `do_pack`.

```rust
use dzip_core::{do_pack, Result, model::Config};
use dzip_core::io::{PackSource, PackSink};

struct MyPackSource { /* ... */ }
impl PackSource for MyPackSource { /* ... */ }

struct MyPackSink { /* ... */ }
impl PackSink for MyPackSink { /* ... */ }

fn main() -> Result<()> {
    // 1. Load configuration (e.g., from TOML)
    let config: Config = load_config("assets/data.toml");
    let base_name = "data".to_string();

    let source = MyPackSource::new("assets/data/");
    let mut sink = MyPackSink::new("output_dir/");

    // 2. Execute packing
    // Note: 'sink' is passed as a mutable reference
    // The last argument is a progress callback
    do_pack(config, base_name, &mut sink, &source, |_| {})?;
    
    println!("Archive created successfully.");
    Ok(())
}

```

## ⚙️ Core Modules

* **`codecs`**: Wrappers for underlying compression libraries (`lzma-rust2`, `flate2`, `bzip2`).
* **`format`**: Definitions of the `.dz` binary structures, magic numbers, and flags.
* **`model`**: Structs for the runtime representation of archives (`ArchiveMetadata`, `Config`, `ChunkDef`), fully serializable via Serde.
* **`io`**: Trait definitions for abstracting input/output operations.

## 📄 License

This project is licensed under the **GNU General Public License v3.0**. See the `LICENSE` file for details.