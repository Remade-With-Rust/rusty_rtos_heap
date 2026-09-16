//! `heap_5.c`: `heap_4`'s free list, laid out over several regions.
//!
//! # Why this is a wrapper and not a second allocator
//!
//! `heap_4.c` and `heap_5.c` are two files in the pinned kernel whose
//! `pvPortMalloc`, `vPortFree` and `prvInsertBlockIntoFreeList` are the same
//! code. Only initialisation differs: heap_4 lays out one arena lazily inside
//! the first allocation, heap_5 is handed a table of regions up front.
//!
//! Copying that duplication into Rust would copy its cost — two homes for one
//! bug, and a fix that has to be made twice. So the free list stays in
//! [`Heap4`] and this is the other initialiser with a type around it.
//!
//! # Why a type around it, rather than just calling `define_regions`
//!
//! Because [`Heap4::alloc`] lays the arena out lazily when nothing has laid it
//! out yet, which is right for heap_4 and wrong for heap_5 — the C's
//! `pvPortMalloc` opens with `configASSERT( pxEnd )`, meaning *you were
//! supposed to have defined the regions by now*. A `Heap5` cannot be built
//! without them, so that mistake is not reachable.

use rusty_rtos_core::error::Result;

use crate::heap4::{Block, Heap4, Region};

/// `heap_5.c` over an `N`-byte arena carved into regions.
///
/// * `ALIGN` is `portBYTE_ALIGNMENT`.
/// * `LINK` is `sizeof( BlockLink_t )`.
///
/// The regions are sub-ranges of one arena and the space between them is a
/// real gap. That is not bookkeeping: coalescing is an address comparison, so
/// what stops two regions merging is that the first one's end is not the
/// second one's start.
#[derive(Debug)]
pub struct Heap5<const N: usize, const ALIGN: usize, const LINK: usize> {
    inner: Heap4<N, ALIGN, LINK>,
}

impl<const N: usize, const ALIGN: usize, const LINK: usize> Heap5<N, ALIGN, LINK> {
    /// `xHeapStructSize`.
    pub const STRUCT_SIZE: usize = Heap4::<N, ALIGN, LINK>::STRUCT_SIZE;

    /// A heap over `regions`, which is `vPortDefineHeapRegions`.
    ///
    /// # Errors
    ///
    /// As [`Heap4::define_regions`]: no regions, a region that does not fit,
    /// one too small for its own end marker, or regions **out of address
    /// order**. The C asserts that order rather than sorting —
    /// `configASSERT( ( size_t ) xAddress > ( size_t ) pxEnd )` under the
    /// comment "Check blocks are passed in with increasing start addresses".
    pub fn new(regions: &[Region]) -> Result<Self> {
        let mut inner = Heap4::new();
        inner.define_regions(regions)?;
        Ok(Self { inner })
    }

    /// `pvPortMalloc`.
    pub fn alloc(&mut self, wanted: usize) -> Option<Block> {
        self.inner.alloc(wanted)
    }

    /// `vPortFree`.
    ///
    /// # Errors
    ///
    /// As [`Heap4::free`] — including the protector's refusals, which apply
    /// here unchanged.
    pub fn free(&mut self, block: Block) -> Result<()> {
        self.inner.free(block)
    }

    /// `xPortGetFreeHeapSize`.
    #[must_use]
    pub const fn free_bytes(&self) -> usize {
        self.inner.free_bytes()
    }

    /// `xPortGetMinimumEverFreeHeapSize`.
    #[must_use]
    pub const fn minimum_ever_free_bytes(&self) -> usize {
        self.inner.minimum_ever_free_bytes()
    }

    /// How many allocations succeeded.
    #[must_use]
    pub const fn allocations(&self) -> usize {
        self.inner.allocations()
    }

    /// How many frees succeeded.
    #[must_use]
    pub const fn frees(&self) -> usize {
        self.inner.frees()
    }

    /// How many free blocks there are, and the largest.
    #[must_use]
    pub fn free_list_shape(&self) -> (usize, usize) {
        self.inner.free_list_shape()
    }

    /// A real address for an offset, for a caller that needs one.
    pub fn address_of(&mut self, offset: u64) -> Option<core::ptr::NonNull<u8>> {
        self.inner.address_of(offset)
    }
}
