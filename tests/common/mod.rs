// The canonical R3 stub exposes its full contract surface (`start_stub`,
// `count`, `first_segment`, and every captured field) for reuse by later
// milestones' integration tests (M6/M7/M8). Each test crate that includes
// `mod common;` exercises only a subset, so unused items are expected here.
#![allow(dead_code)]

pub mod fixtures;
pub mod stub;
