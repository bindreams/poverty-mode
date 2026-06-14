//! Pure, headless reducer for the interactive proxy-selection TUI.
//!
//! All UI *meaning* lives here so it can be unit-tested without a terminal.
//! `src/tui.rs` is a thin render/event shell that translates key events into
//! [`TuiAction`]s and feeds them to [`TuiState::apply`].
