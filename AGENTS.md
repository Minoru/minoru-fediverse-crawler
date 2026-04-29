# Minoru's Fediverse Crawler - Agent Guidelines

## Build & Test Commands

Run these at the end of the turn to check that the project is in good shape.

**Build:**
- `cargo check` - quickly check for compilation errors
- `cargo clippy --all-features --all-targets` - Run clippy linter (used in Makefile)

**Testing:**
- `cargo test` - Run all tests
- Tests are inline with `#[cfg(test)]` modules (e.g., `src/checker/mod.rs:363`)

## Code Style Guidelines

Run `cargo fmt` at the end of the turn to enforce Rust code style.
