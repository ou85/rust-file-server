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
src/
├── main.rs
├── config.rs
├── crypto.rs
├── database.rs
├── models.rs
└── storage.rs
tools/
├── keygen/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
└── hashgen/
    ├── Cargo.toml
    └── src/
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

## Building

```bash
cargo build
```

## Running

```bash
cargo run
```

## Utilities

### Key Generation (keygen)

Generate a cryptographically secure encryption key for the RFS_ENCRYPTION_KEY environment variable.

Usage:
```bash

cargo run -p keygen
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

cargo run -p hashgen
```

Prompt:
```
Enter password to hash: 
```

Output:
```
Bcrypt hash: $2b$12$5QU3Tl1gcmEyFZL/ahdBHOU14UMQYRDkwvrVZufE8.QolJEmMva0e
```

Add the hash to your .env file, wrapped in double quotes to prevent shell interpolation of the $ symbol:
```env

USER_PASSWORD_HASH='$2b$12$5QU3Tl1gcmEyFZL/ahdBHOU14UMQYRDkwvrVZufE8.QolJEmMva0e'
ADMIN_PASSWORD_HASH=\$2b\$12\$...
```
⚠️ Important: Always enclose bcrypt hashes in double quotes in .env files to avoid truncation caused by $ symbol expansion.

## License

GNU General Public License v3.0

***