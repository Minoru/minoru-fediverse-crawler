# Minoru's Fediverse Crawler - Agent Guidelines

## Build & Test Commands

**Build:**
- `cargo check` - quickly check for compilation errors
- `cargo build --release` - Build with optimizations
- `cargo clippy --all-features --all-targets` - Run clippy linter (used in Makefile)

**Note:** The project uses a Docker-based build environment (see `docker/buildhost.dockerfile`). The Makefile runs clippy and build via Docker for consistency.

**Testing:**
- `cargo test` - Run all tests
- Tests are inline with `#[cfg(test)]` modules (e.g., `src/checker/mod.rs:363`)

**Makefile targets:**
- `make` - Build everything (binary, HTML docs, SVG)
- `make clean` - Clean build artifacts
- `make deploy` - Deploy via Ansible

## Code Style Guidelines

Run `cargo fmt` at the end of the turn to enforce as much of this as possible.

**Imports:**
- Use `use crate::` for internal module imports
- Use `use anyhow::{Context, anyhow, bail}` for error handling
- Group std imports with braces: `use std::sync::{Arc, atomic::{AtomicBool, Ordering}}`
- Use `use lexopt::prelude::*` for lexopt parser

**Formatting:**
- 4-space indentation
- No comments unless explaining non-obvious code
- Use `#[deny(clippy::expect_used, clippy::unwrap_used, ...)]` at crate root

**Error Handling:**
- Use `anyhow::Result` for function return types
- Chain errors with `.context(with_loc!("message"))?`
- Use `bail!("message")` for early returns
- Use `.map_err(|err| err.into())` for type conversion
- Log errors with `error!(logger, "message");` before returning

**Naming Conventions:**
- snake_case for functions and variables
- PascalCase for types, enums, and structs
- Module files: `mod.rs` or `module_name.rs`
- Submodules in `mod/` directories (e.g., `src/checker/mod.rs`)

**Types:**
- Use `enum` for state machines (e.g., `InstanceState`)
- Implement `ToSql`/`FromSql` for SQLite custom types
- Use `#[derive(PartialEq, Eq, Debug, Clone, Copy)]` for simple enums
- Use `#[serde(untagged)]` for flexible JSON deserialization

**SQLite:**
- Use `params![]` for parameterized queries
- Wrap transactions with `conn.transaction()?`
- Handle `SQLITE_BUSY` with retry helpers (`on_sqlite_busy_retry`)
- Use `prepare_cached()` for repeated queries

**Logging:**
- Use `slog` with `Logger::root()` and `Logger::new()`
- Add context with `o!("key" => "value")`
- Log at appropriate levels: `info!`, `error!`

**Concurrency:**
- Use `rusty_pool::ThreadPool` for worker pools
- Wrap tasks with `std::panic::catch_unwind()`
- Use `AtomicBool` for graceful shutdown signals

**Testing:**
- Tests go in `#[cfg(test)] mod test { ... }`
- Suppress clippy in tests with `#[allow(clippy::expect_used)]`
- Use `.unwrap()` in test assertions

**Database:**
- Location: `minoru-fediverse-crawler.db`
- Enable WAL mode on connection
- Use migrations via `db::init()`
