# dzip-rs

**dzip-rs** is a modern, high-performance toolkit written in Rust for handling **Marmalade SDK** resource archives (`.dz` and `.dzip`).

This repository is organized as a **Rust Workspace** containing modular components for parsing, analyzing, extracting, and creating archive files. It aims to provide a safe and robust alternative to legacy tools, with a focus on correctness (fixing broken headers) and cross-platform compatibility.

## 📂 Project Structure

The project is divided into two main crates:

| Crate | Path | Description |
| --- | --- | --- |
| **dzip-core** | [`crates/core`](https://www.google.com/search?q=crates/core) | The backend library. It handles the binary format parsing, compression algorithms (LZMA, ZLIB, etc.), and parallel processing pipeline. It is I/O agnostic and can be embedded in other applications. |
| **dzip-cli** | [`crates/cli`](https://www.google.com/search?q=crates/cli) | The terminal frontend. A command-line tool that exposes the core functionality to end-users for unpacking, packing, and listing archive contents. |

## ✨ Key Features

* **⚡ Parallel Architecture**: Uses `rayon` to parallelize compression and decompression blocks, ensuring maximum throughput on multi-core systems.
* **🔧 Legacy Support**: Automatically detects and fixes common errors in old archive headers (e.g., incorrect `ZSIZE` fields) by analyzing chunk offsets.
* **📦 Split Archives**: Seamlessly handles multi-volume archives (e.g., `data.dz`, `data.d01`...) as a single logical unit.
* **🐧 Cross-Platform**:
* **Core**: Preserves raw path data for fidelity.
* **CLI**: Automatically normalizes path separators (Windows backslashes `\` vs. Unix forward slashes `/`) depending on the user's operating system.


* **📄 Configurable**: Uses TOML configuration files to allow precise control over chunk layout and compression methods during packing.

## 🚀 Getting Started

### Prerequisites

* [Rust](https://www.rust-lang.org/tools/install) (latest stable version)
* Git

### Building the Workspace

To build both the library and the CLI tool from the root directory:

```bash
# Clone the repository
git clone https://github.com/your-username/dzip-rs.git
cd dzip-rs

# Build all crates in release mode
cargo build --release

```

The compiled binary will be available at:

* `target/release/dzip-cli` (Linux/macOS)
* `target/release/dzip-cli.exe` (Windows)

### Running Tests

Run the test suite for the entire workspace to ensure integrity:

```bash
cargo test --workspace

```

## 📖 Usage Examples

Since most users interact with the project via the CLI, here are quick examples. For detailed documentation, please refer to the [CLI README](https://www.google.com/search?q=crates/cli/README.md).

```bash
# Unpack an archive
./target/release/dzip-cli unpack assets.dz

# List contents without extracting
./target/release/dzip-cli list assets.dz

# Repack from a config file
./target/release/dzip-cli pack assets.toml

```

## 🤝 Contributing

Contributions are welcome! Please follow these steps:

1. **Fork** the repository.
2. **Create** a feature branch (`git checkout -b feature/amazing-feature`).
3. **Commit** your changes.
4. **Push** to the branch.
5. **Open** a Pull Request.

Please ensure your code passes `cargo clippy` and `cargo fmt` before submitting.

## 📄 License

This project is licensed under the **GNU General Public License v3.0**.
See the [LICENSE](https://www.google.com/search?q=LICENSE) file for details.

---

*Marmalade SDK is a trademark of its respective owners.*