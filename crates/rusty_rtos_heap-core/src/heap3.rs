//! `heap_3.c`: route to the platform's allocator.
//!
//! # What `heap_3.c` actually is
//!
//! Thirty lines, and all of them are `malloc` / `free` inside
//! `vTaskSuspendAll()` / `xTaskResumeAll()`. **Its entire content is the lock,
//! not the allocation.** It contributes no policy: whatever the platform's
//! allocator does about fragmentation, alignment and failure is what a
//! `heap_3` system gets.
//!
//! # Why this routes to the GLOBAL allocator and not to a named one
//!
//! In Rust the platform's allocator is the global allocator, which the
//! *deliverable* declares — the system one on a host, `rusty_rtos_alloc` on a
//! firmware that registered a region, anything else a consumer picks. A seam
//! that named one would be choosing for them, and choosing badly for much of
//! the hardware FreeRTOS exists to serve: `rusty_alloc`'s `MIN_REGION` is
//! **65,536 bytes**, the entire SRAM of a classic FreeRTOS QEMU target, while
//! [`crate::heap4`] runs in whatever arena you declare and its differential
//! runs in 8 KiB. Those are different floor FUNCTIONS, not one tuned
//! differently.
//!
//! # Why a handle and not a pointer
//!
//! The first draft of this returned `*mut u8` and took it back, which is what
//! the C does — and it needed `unsafe` for the raw allocation, the header
//! arithmetic and the `Layout` reconstruction. This crate is
//! `#![forbid(unsafe_code)]` and the family keeps its `unsafe` in
//! `rusty_rtos_port-<arch>`, so the lint refused it.
//!
//! **The lint was right.** A handle is what the rest of this family hands out
//! — [`crate::heap4::Block`] is an offset plus a generation, and a task, queue
//! and timer handle are all generational indices — and it removes the same bug
//! class here that it removes there. `Box<[u8]>` owns the bytes, the slot owns
//! the `Box`, and freeing twice is a refused lookup rather than a double
//! `dealloc`.
//!
//! It also removes the mismatch the pointer version had to solve:
//! `free( void * )` takes an address, `dealloc` wants the `Layout` back, and
//! the C is only spared that because libc keeps the bookkeeping itself. A
//! `Box<[u8]>` knows its own length, so there is no header to invent.
//!
//! # What a caller does NOT get
//!
//! The bytes come from the global allocator, so `heap_3` cannot say how much
//! is left, cannot coalesce, and cannot tell you what it did — exactly as
//! `heap_3.c` cannot. Prefer [`crate::heap4`] unless something specific is
//! wanted from the platform.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use rusty_rtos_core::error::{Error, Result};

/// What [`Heap3::alloc`] answers with: which slot, and which allocation.
///
/// The same shape as [`crate::heap4::Block`] and for the same reason — an
/// index alone cannot distinguish the allocation you were given from the one
/// living in that slot now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    slot: u32,
    generation: u32,
}

impl Handle {
    /// Which slot this allocation lives in.
    #[must_use]
    pub const fn slot(self) -> u32 {
        self.slot
    }

    /// Which allocation it was.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// `heap_3.c`: the platform's allocator, behind the family's heap surface.
///
/// Owns no arena — that is the point. What it owns is the bookkeeping that
/// turns the global allocator into something with the same surface as
/// [`crate::heap4::Heap4`] and [`crate::heap5::Heap5`].
#[derive(Debug, Default)]
pub struct Heap3 {
    /// One entry per allocation ever made; a freed entry is `None` and its
    /// slot is reused, which is why the generation exists.
    slots: Vec<Option<Box<[u8]>>>,
    /// The generation each slot is currently on.
    generations: Vec<u32>,
    /// The generation the next allocation is stamped with.
    next_generation: u32,
    /// `xNumberOfSuccessfulAllocations`.
    allocations: usize,
    /// `xNumberOfSuccessfulFrees`.
    frees: usize,
    /// How many bytes are live, which the C cannot tell you and this can.
    live_bytes: usize,
}

impl Heap3 {
    /// A heap over the global allocator.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            generations: Vec::new(),
            next_generation: 1,
            allocations: 0,
            frees: 0,
            live_bytes: 0,
        }
    }

    /// `pvPortMalloc`.
    ///
    /// The C wraps `malloc` in `vTaskSuspendAll()` / `xTaskResumeAll()`
    /// because a C library allocator is not generally safe against the
    /// scheduler preempting it. Rust's `GlobalAlloc` contract already requires
    /// an implementation to be safe for concurrent use, so the lock the C adds
    /// is the allocator's own problem rather than this seam's — adding a
    /// second one would serialise something that does not need it.
    ///
    /// `None` is the C's `NULL`: a zero request, or the allocator refused.
    pub fn alloc(&mut self, wanted: usize) -> Option<Handle> {
        if wanted == 0 {
            return None;
        }
        // `try_reserve` rather than `vec![0; n]`: an allocation failure must
        // be `None` the way `malloc` returns NULL, not a panic. That is the
        // whole contract a heap is judged on.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.try_reserve_exact(wanted).ok()?;
        bytes.resize(wanted, 0);
        let bytes = bytes.into_boxed_slice();

        let slot = match self.slots.iter().position(Option::is_none) {
            Some(free) => free,
            None => {
                self.slots.try_reserve(1).ok()?;
                self.generations.try_reserve(1).ok()?;
                self.slots.push(None);
                self.generations.push(0);
                self.slots.len().checked_sub(1)?
            }
        };

        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);

        *self.slots.get_mut(slot)? = Some(bytes);
        *self.generations.get_mut(slot)? = generation;

        self.allocations = self.allocations.saturating_add(1);
        self.live_bytes = self.live_bytes.saturating_add(wanted);
        Some(Handle {
            slot: u32::try_from(slot).ok()?,
            generation,
        })
    }

    /// `vPortFree`.
    ///
    /// # Errors
    ///
    /// [`Error::Gone`] for a handle whose slot is empty (a double free) or
    /// whose generation no longer matches (a free of a slot that has since
    /// been reallocated). Both are caller bugs, and neither can corrupt
    /// anything — `Box` owns the bytes and dropping it is the only way they go
    /// back.
    pub fn free(&mut self, handle: Handle) -> Result<()> {
        let slot = usize::try_from(handle.slot()).map_err(|_| Error::Gone)?;
        if self.generations.get(slot).copied() != Some(handle.generation()) {
            return Err(Error::Gone);
        }
        let entry = self.slots.get_mut(slot).ok_or(Error::Gone)?;
        let bytes = entry.take().ok_or(Error::Gone)?;
        self.live_bytes = self.live_bytes.saturating_sub(bytes.len());
        drop(bytes);
        self.frees = self.frees.saturating_add(1);
        Ok(())
    }

    /// The bytes of an allocation.
    ///
    /// # Errors
    ///
    /// [`Error::Gone`] as [`Self::free`].
    pub fn bytes_mut(&mut self, handle: Handle) -> Result<&mut [u8]> {
        let slot = usize::try_from(handle.slot()).map_err(|_| Error::Gone)?;
        if self.generations.get(slot).copied() != Some(handle.generation()) {
            return Err(Error::Gone);
        }
        self.slots
            .get_mut(slot)
            .and_then(Option::as_deref_mut)
            .ok_or(Error::Gone)
    }

    /// `xPortGetFreeHeapSize`, which this scheme cannot answer.
    ///
    /// `heap_3.c` does not implement it either: the file defines
    /// `pvPortMalloc`, `vPortFree` and `vPortHeapResetState` and nothing else,
    /// because libc does not publish how much it has left. A made-up number
    /// would be worse than a refusal.
    ///
    /// # Errors
    ///
    /// Always [`Error::Unsupported`]. Use [`Self::live_bytes`] for the half
    /// this CAN answer.
    // No `expect(unused_self)`: clippy does not fire it here, and an
    // expectation that is never fulfilled is itself a warning. `&self` stays
    // because the signature matches the other heaps, which do have state.
    pub const fn free_bytes(&self) -> Result<usize> {
        Err(Error::Unsupported)
    }

    /// How many bytes are currently live.
    ///
    /// Not a FreeRTOS number — `heap_3.c` has nothing like it. It is kept
    /// because this seam is the one place that knows, and a consumer weighing
    /// `heap_3` against `heap_4` should be able to see what it is using.
    #[must_use]
    pub const fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    /// How many allocations succeeded.
    #[must_use]
    pub const fn allocations(&self) -> usize {
        self.allocations
    }

    /// How many frees succeeded.
    #[must_use]
    pub const fn frees(&self) -> usize {
        self.frees
    }

    /// How many slots are currently live.
    #[must_use]
    pub fn live_allocations(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}
