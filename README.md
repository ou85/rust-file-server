# Rust File Server

A simple encrypted file server written in Rust.

This project is being developed as a learning exercise to explore:

- Rust programming language
- File storage
- Encryption
- Embedded databases
- HTTP APIs
- System design

## Current Features

- Create storage directories automatically
- Save files to disk
- Configuration management
- Modular project structure

## Project Structure

```text
data/
├── db/
└── files/
src/
├── auth/          # password verification and roles
├── blob_store/    # encrypted blob files and temporary uploads
├── crypto/        # file encryption format
├── domain/        # file metadata and identifiers
├── metadata/      # Redb persistence
├── web/           # Axum routes
├── tools/
└── main.rs
```

## Roadmap

### Storage

- [x] Create storage directories
- [x] Save files to disk
- [x] Generate unique file IDs (UUID)
- [x] File metadata management

### Database

- [x] Integrate Redb
- [x] Store file metadata
- [ ] Store user information

### Security

- [ ] Password hashing with Argon2
- [x] File encryption using AES-GCM
- [x] User authentication

### API

- [x] HTTP server with Axum
- [x] File upload endpoint
- [x] File download endpoint
- [x] File listing endpoint

### Future Ideas

- [x] Web interface
- [ ] File sharing links
- [ ] Multi-user support
- [ ] Docker deployment

## Running

```bash
cargo run
```

By default, the server listens on `0.0.0.0:3000` and stores all state in a `data/`
directory next to the executable. Use `--local` to listen only on `127.0.0.1`.
Use `--data-dir /path/to/data` to choose a state directory (or set `RFS_DATA_DIR`).

For systemd or Alpine/OpenRC installations, set `--data-dir` to a persistent writable
directory such as `/var/lib/rust-file-server`; no paths relative to the service working
directory are required.

## Building

```bash
cargo build --release   

```

## Strip binary

```bash
strip target/release/rust-file-server

```

## Utilities

### Key Generation (keygen)

Generate a cryptographically secure encryption key for the RFS_ENCRYPTION_KEY environment variable.

Usage:
```bash
cargo run keygen
```
Output:

The utility generates a random 256-bit (32-byte) key in both Base64 and Hex formats:
```
=== Encryption Key Generator ===
Generated Key (Base64): a3f9x2k8mL9pQwErT5yUiOp2sD4fGhJkLmNoPqRs==
Generated Key (Hex):    6b7f371a7cec8ac8c53d4144b4a79c8b5c9e2f0a3d6c7e8f9a0b1c2d3e4f5a6b
```

Add the Base64 key to your .env file:
```env

RFS_ENCRYPTION_KEY=a3f9x2k8mL9pQwErT5yUiOp2sD4fGhJkLmNoPqRs==
```

### Password Hash Generation (hashgen)

Generate bcrypt password hashes for user and admin authentication.

Usage:
```bash

cargo run hashgen
```

Prompt:
```
Enter password to hash: 
```

Output:
```
Bcrypt hash: $2b$12$5QU3Tl1gcmEyFZL/ahdBHOU14UMQYRDkwvrVZufE8.QolJEmMva0e
```

Add the hash to your .env file, wrapped in double quotes or single quotes '...HASH...' to prevent shell interpolation of the $ symbol:
```env

RFS_USER_PASSWORD_HASH='$2b$12$5QU3Tl1gcmEyFZL/ahdBHOU14UMQYRDkwvrVZufE8.QolJEmMva0e'
RFS_ADMIN_PASSWORD_HASH=\$2b\$12\$...
```
⚠️ Important: Always enclose bcrypt hashes in double quotes in .env files to avoid truncation caused by $ symbol expansion.  ( in some cases single quotes works better )


## Portable Static Build (Linux)

To create a fully self-contained binary that runs on any 64-bit Linux distribution (including Alpine Linux) without external `glibc` dependencies, compile it statically using `musl`:

1. Add the musl target
```bash
rustup target add x86_64-unknown-linux-musl
```

2. Build the static release binary
- for x86-64

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

(Optional) Strip debug symbols to reduce binary size

```bash
strip target/x86_64-unknown-linux-musl/release/rust-file-server
```

- for aarch64

```bash
cargo zigbuild --release --target aarch64-unknown-linux-musl
```


## License

GNU General Public License v3.0

***
