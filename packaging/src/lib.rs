//! Packaging-only crate.
//!
//! This crate ships no code. It exists so that `cargo deb` and
//! `cargo generate-rpm` — both of which are driven by `[package.metadata.*]` on
//! a *single* package — have one place to describe the whole RatClick
//! installation, rather than four crates each describing a slice of it.
//!
//! Everything interesting is in `packaging/Cargo.toml`. Build the packages with
//! `scripts/build-packages.sh`.
