#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
//! `rusty_rtos_heap-core` — the pure heart of `rusty_rtos_heap`.
//!
//! Rules this crate lives by (from the Kairos mission plan):
//!
//! 1. `no_std` by default; `alloc` is a feature, never an assumption.
//! 2. No CPU, no registers, no allocator, no operating system. Ports and
//!    backends are separate WRAP crates.
//! 3. Every type that crosses to another Kairos package comes from
//!    `rusty_rtos_core`, so packages compose without conversions.
//! 4. Handles are indices, never pointers; nothing on a hot path allocates.
//! 5. `forbid(unsafe)`. The C kernel's trace is the oracle; the scalar path
//!    is the oracle; any faster path is gated identical against it.

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod heap1;
/// `heap_3.c`: the platform's allocator behind the family's heap surface.
///
/// Needs `alloc`, which is the honest dependency: it IS the "use the
/// platform's allocator" heap, so a build without one has nothing to route
/// to.
#[cfg(feature = "alloc")]
pub mod heap3;
pub mod heap4;
pub mod heap5;
/// A fixed-size block pool: the allocation an RTOS actually makes.
///
/// Not a FreeRTOS file, and it does not pretend to be. `heap_4` answers a
/// general question and its cost is that generality; most RTOS allocation
/// -- TCBs, queue items, timer records, event blocks -- is one size known
/// when the system is declared, and that question is far cheaper to answer.
/// 55.2% fewer instructions than `heap_4` on the host, 73.9% at the 32-bit
/// width every Kairos target has.
pub mod pool;

pub use heap1::Heap1;
#[cfg(feature = "alloc")]
pub use heap3::Heap3;
pub use heap4::{Heap4, Region};
pub use heap5::Heap5;
pub use pool::{Pool, Slot};
pub use rusty_rtos_core as rtos_core;

/// The names a firmware wants in scope.
pub mod prelude {
    pub use rusty_rtos_core::prelude::*;
}

/// Crate version, for manifests and logs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
