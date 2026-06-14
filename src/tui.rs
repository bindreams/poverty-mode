//! Interactive proxy-selection TUI (ratatui + crossterm).
//!
//! The render loop here is intentionally thin: it owns terminal lifecycle and
//! key mapping only. Every decision is delegated to [`reducer::TuiState`].

pub mod reducer;
