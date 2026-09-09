#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
//! `rusty_rtos_heap` — FreeRTOS heap_1 / heap_4 / heap_5 remade in Rust behind the Heap seam, with the heap protector; heap_3 as the seam over rusty_alloc small-metal and esp-alloc; static allocation first-class.
//!
//! This is the facade: it re-exports the `no_std` core. Depend on this crate;
//! reach into the sub-crates only when you are building a port or a backend.
//!
//! Part of Kairos (Remade With Rust). Plan: `docs/plans/rusty_rtos_heap.md`.

pub use rusty_rtos_heap_core::*;

/// The names a firmware wants in scope.
pub mod prelude {
    pub use rusty_rtos_heap_core::prelude::*;
}
