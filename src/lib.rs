//! tenetui library crate: everything except the terminal event loop lives here
//! so benches and integration tests can exercise the internals. The binary
//! (`main.rs`) is a thin driver that wires these modules to crossterm/ratatui.
//!
//! Module boundaries (see docs/architecture.md): git2 access is confined to
//! `repo`, all color to `theme`, rendering is pure in `ui`, and state mutation
//! is centralized in `app`.

pub mod app;
pub mod diff;
pub mod input;
pub mod repo;
pub mod syntax;
pub mod theme;
pub mod ui;
