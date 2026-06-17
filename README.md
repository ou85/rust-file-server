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

## License

GNU General Public License v3.0

***