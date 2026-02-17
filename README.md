# vx - A Fast Version Control System

Data-oriented Git alternative built in Rust with BLAKE3 hashing and SoA layouts.

## Build

```bash
cargo build --release
```

## Usage

```bash
# Initialize repository
./target/release/vx init

# Hash a file (without writing)
./target/release/vx hash-object test.txt

# Hash and write to object store
./target/release/vx hash-object -w test.txt

# Write tree from current directory
./target/release/vx write-tree

# Create a commit
./target/release/vx commit -m "first commit" --author "Your Name"

# View an object
./target/release/vx cat-file <hash>
```

## Example

```bash
./target/release/vx init
echo "hello world" > test.txt
./target/release/vx hash-object -w test.txt
./target/release/vx write-tree
./target/release/vx commit -m "initial commit"
```

## Format

- Uses BLAKE3 (32-byte hashes) instead of SHA-1
- Binary object format with SoA (Structure of Arrays) layout
- Magic: `VX01`
- Object types: Blob (0), Tree (1), Commit (2)

## Architecture

- `hash.rs` - BLAKE3 hashing utilities
- `object.rs` - Core object types (Blob, Tree, Commit)
- `tree_builder.rs` - Convenient tree construction
- `storage.rs` - Object storage layer
- `repository.rs` - Repository abstraction
- `*.rs` - Command implementations (hash-object, cat-file, write-tree, commit)
