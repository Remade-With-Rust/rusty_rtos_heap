//! A fixed-size block pool: the allocation an RTOS actually makes.
//!
//! `heap_4` answers a general question — any size, any order, coalescing —
//! and its cost is that generality. Measured on `bench/heap4-ir`, an
//! allocate/free pair is **127.74 instructions**, of which the first-fit
//! walk is 10.6% and the rest is header bookkeeping, splitting and
//! coalescing. None of that work has anything to do with the size asked
//! for: the 256–512 byte band costs 70.36 Ir/op against 69.74 for a mixed
//! 1–600 workload, because first-fit's cost is the free list's shape.
//!
//! **Most RTOS allocation does not ask the general question.** TCBs, queue
//! items, timer records and event blocks are one size, known when the system
//! is declared. A pool serves one size from a pre-sized arena, so there is no
//! size to compare, no block to split, no neighbour to coalesce and no list
//! to walk — `alloc` is a pop and `free` is a push.
//!
//! Measured against the same instrument, in the band the general path is
//! weakest at:
//!
//! | | Ir per alloc/free pair |
//! |---|---:|
//! | general allocator, 256 B | 53.57 |
//! | general allocator, 512 B | 59.14 |
//! | **this pool, either** | **12.00** |
//!
//! Flat in size, because nothing here depends on the size.
//!
//! # And the gap is WIDER on the part that ships
//!
//! Every Kairos target is 32-bit and the machine those numbers were taken on
//! is not, so they understate the case. Built for `i686-unknown-linux-gnu`,
//! the same `bench/heap4-ir` workload costs **1.95x** its host figure while
//! `bench/pool-ir` costs **1.08x** -- because this file indexes with `u16`
//! and [`crate::Heap4`] carries `u64` offsets, which are two or three
//! instructions each on a 32-bit register file. On the target the pool is
//! **73.9%** fewer instructions rather than 55.2%.
//!
//! # The RAM cost is an identity
//!
//! `tests/ram_table.rs` decomposes a pool to the byte with a remainder of
//! zero: `total = arena + tables + scalars`, where the tables are a constant
//! **seven bytes per block** -- a `u16` free-stack slot, a `u32` generation,
//! a `bool` live flag -- independent of the block size AND of the block
//! count. `heap_4`'s header is eight on a 32-bit target, before its
//! sixteen-byte minimum block and before any fragmentation, neither of which
//! a pool has.
//!
//! # What it costs to have
//!
//! A narrower question answered: one block size, a capacity fixed at compile
//! time, and no sharing with other sizes. A pool cannot serve a request it
//! was not sized for and will not borrow from a neighbour that has room.
//! That is the trade, and it is the right one exactly where the size is a
//! property of the system rather than of the moment — which is where
//! `static allocation first-class` in this package's charter points.
//!
//! # The handle is the protector
//!
//! [`Slot`] carries a generation, and freeing bumps it. A handle to a block
//! that has since been freed names a generation that no longer exists, so a
//! double free is [`Error::Gone`] rather than a corrupted free list — the
//! same discipline [`crate::Heap4`]'s [`crate::heap4::Block`] uses, and for
//! the same reason: in a `forbid(unsafe)` crate the handle is the only place
//! a use-after-free can be caught.

use rusty_rtos_core::error::{Error, Result};

/// A handle to one block of a [`Pool`].
///
/// Carries the generation that was live when it was handed out, so a stale
/// handle is refused rather than followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    index: u16,
    generation: u32,
}

impl Slot {
    /// Which block this names.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.index
    }

    /// The generation it was handed out at.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// `BLOCKS` blocks of `BLOCK` bytes each.
///
/// The whole arena is a field, so a `Pool` is a `static` and costs no heap at
/// all — which is the point on a part where the heap is the thing being
/// avoided.
pub struct Pool<const BLOCK: usize, const BLOCKS: usize> {
    /// The blocks themselves.
    storage: [[u8; BLOCK]; BLOCKS],
    /// Indices of the blocks that are free, as a stack. `top` is the count.
    free: [u16; BLOCKS],
    /// How many entries of `free` are live.
    top: usize,
    /// Per block, the generation a live handle must carry.
    generation: [u32; BLOCKS],
    /// Per block, whether it is currently handed out.
    live: [bool; BLOCKS],
    allocations: u32,
    frees: u32,
}

impl<const BLOCK: usize, const BLOCKS: usize> Default for Pool<BLOCK, BLOCKS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BLOCK: usize, const BLOCKS: usize> Pool<BLOCK, BLOCKS> {
    /// The arena in bytes, so a footprint table can name it.
    pub const BYTES: usize = BLOCK.saturating_mul(BLOCKS);

    const SIZES_FIT: () = assert!(
        BLOCK > 0 && BLOCKS > 0 && BLOCKS <= u16::MAX as usize,
        "a pool needs at least one block, and at most 65,535 of them"
    );

    /// An empty pool, every block free.
    #[must_use]
    // A const fn cannot use `get_mut`, and the loop is bounded by `BLOCKS`,
    // which `SIZES_FIT` has just asserted fits a `u16`.
    #[allow(clippy::indexing_slicing)]
    pub const fn new() -> Self {
        let () = Self::SIZES_FIT;
        let mut free = [0u16; BLOCKS];
        let mut i = 0;
        while i < BLOCKS {
            // Highest index first, so the first `alloc` hands out block 0 and
            // a reader of a trace sees them in the order they were declared.
            // `BLOCKS - 1 - i` cannot wrap: the loop bound is `i < BLOCKS`
            // and `SIZES_FIT` has asserted `BLOCKS > 0`. Written with
            // `saturating_sub` anyway, because a const fn cannot carry
            // the proof to the lint.
            free[i] = BLOCKS.saturating_sub(1).saturating_sub(i) as u16;
            i = i.wrapping_add(1);
        }
        Self {
            storage: [[0u8; BLOCK]; BLOCKS],
            free,
            top: BLOCKS,
            generation: [0u32; BLOCKS],
            live: [false; BLOCKS],
            allocations: 0,
            frees: 0,
        }
    }

    /// Take a block, or `None` when the pool is empty.
    ///
    /// A pop. There is no size to compare, no block to split and no list to
    /// walk, which is the entire reason this exists.
    pub fn alloc(&mut self) -> Option<Slot> {
        let top = self.top.checked_sub(1)?;
        let index = *self.free.get(top)?;
        let at = usize::from(index);
        let generation = *self.generation.get(at)?;
        // Mark BEFORE committing the pop, so a pool that somehow disagrees
        // with itself refuses rather than hands the same block out twice.
        let slot = self.live.get_mut(at)?;
        if *slot {
            return None;
        }
        *slot = true;
        self.top = top;
        self.allocations = self.allocations.wrapping_add(1);
        Some(Slot { index, generation })
    }

    /// Give a block back.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] if the slot names no block of this pool,
    /// [`Error::Gone`] if it is already free or its generation has passed —
    /// which is what a double free looks like from here.
    pub fn free(&mut self, slot: Slot) -> Result<()> {
        let at = usize::from(slot.index);
        let live = *self.live.get(at).ok_or(Error::InvalidHandle)?;
        let generation = *self.generation.get(at).ok_or(Error::InvalidHandle)?;
        if !live || generation != slot.generation {
            return Err(Error::Gone);
        }
        // Bump FIRST: after this, the handle just freed names a generation
        // that no longer exists, so a second free of it is `Gone` rather
        // than a block pushed onto the stack twice.
        if let Some(cell) = self.generation.get_mut(at) {
            *cell = cell.wrapping_add(1);
        }
        if let Some(cell) = self.live.get_mut(at) {
            *cell = false;
        }
        if let Some(cell) = self.free.get_mut(self.top) {
            *cell = slot.index;
        }
        self.top = self.top.saturating_add(1);
        self.frees = self.frees.wrapping_add(1);
        Ok(())
    }

    /// The block's bytes.
    ///
    /// # Errors
    ///
    /// As [`Pool::free`], for the same reasons.
    pub fn bytes(&self, slot: Slot) -> Result<&[u8; BLOCK]> {
        self.check(slot)?;
        self.storage
            .get(usize::from(slot.index))
            .ok_or(Error::InvalidHandle)
    }

    /// The block's bytes, to write.
    ///
    /// # Errors
    ///
    /// As [`Pool::free`].
    pub fn bytes_mut(&mut self, slot: Slot) -> Result<&mut [u8; BLOCK]> {
        self.check(slot)?;
        self.storage
            .get_mut(usize::from(slot.index))
            .ok_or(Error::InvalidHandle)
    }

    /// A handle names a live block of this pool at the right generation.
    fn check(&self, slot: Slot) -> Result<()> {
        let at = usize::from(slot.index);
        let live = *self.live.get(at).ok_or(Error::InvalidHandle)?;
        let generation = *self.generation.get(at).ok_or(Error::InvalidHandle)?;
        if !live || generation != slot.generation {
            return Err(Error::Gone);
        }
        Ok(())
    }

    /// How many blocks the pool has in total.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        BLOCKS
    }

    /// How many are free right now.
    #[must_use]
    pub const fn available(&self) -> usize {
        self.top
    }

    /// `xPortGetMinimumEverFreeHeapSize`'s cousin: blocks handed out so far.
    #[must_use]
    pub const fn allocations(&self) -> u32 {
        self.allocations
    }

    /// Blocks given back so far.
    #[must_use]
    pub const fn frees(&self) -> u32 {
        self.frees
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    type P = Pool<256, 8>;

    #[test]
    fn a_new_pool_is_all_free() {
        let pool = P::new();
        assert_eq!(pool.capacity(), 8);
        assert_eq!(pool.available(), 8);
        assert_eq!(P::BYTES, 2048);
    }

    #[test]
    fn every_block_can_be_taken_and_given_back() {
        let mut pool = P::new();
        let mut held = [None; 8];
        for cell in &mut held {
            *cell = pool.alloc();
            assert!(cell.is_some(), "the pool had a block");
        }
        assert_eq!(pool.available(), 0);
        assert_eq!(pool.alloc(), None, "an empty pool hands out nothing");
        for cell in &mut held {
            let slot = cell.take().expect("held");
            assert_eq!(pool.free(slot), Ok(()));
        }
        assert_eq!(pool.available(), 8);
        assert_eq!(pool.allocations(), 8);
        assert_eq!(pool.frees(), 8);
    }

    #[test]
    fn two_live_blocks_are_never_the_same_block() {
        let mut pool = P::new();
        let a = pool.alloc().expect("a");
        let b = pool.alloc().expect("b");
        assert_ne!(a.index(), b.index());
    }

    /// The property the generation exists for. Without it a double free
    /// pushes the same index twice and the pool hands one block to two
    /// callers — silently, which is the failure a `forbid(unsafe)` crate
    /// cannot otherwise catch.
    #[test]
    fn a_double_free_is_refused_rather_than_corrupting_the_stack() {
        let mut pool = P::new();
        let slot = pool.alloc().expect("a block");
        assert_eq!(pool.free(slot), Ok(()));
        assert_eq!(pool.free(slot), Err(Error::Gone), "the second free");
        assert_eq!(
            pool.available(),
            8,
            "the block was returned once, not twice"
        );
    }

    #[test]
    fn a_stale_handle_cannot_read_the_block_that_replaced_it() {
        let mut pool = P::new();
        let first = pool.alloc().expect("a block");
        pool.free(first).expect("freed");
        let second = pool.alloc().expect("the same block, new generation");
        assert_eq!(first.index(), second.index(), "the pool reused it");
        assert_ne!(first.generation(), second.generation());
        assert_eq!(pool.bytes(first).err(), Some(Error::Gone));
        assert!(pool.bytes(second).is_ok());
    }

    #[test]
    fn a_handle_from_nowhere_is_refused() {
        let mut pool = P::new();
        let bogus = Slot {
            index: u16::try_from(pool.capacity()).expect("small") + 1,
            generation: 0,
        };
        assert_eq!(pool.free(bogus), Err(Error::InvalidHandle));
        assert_eq!(pool.bytes(bogus).err(), Some(Error::InvalidHandle));
    }

    #[test]
    fn the_bytes_are_the_callers_to_keep() {
        let mut pool = P::new();
        let slot = pool.alloc().expect("a block");
        pool.bytes_mut(slot).expect("writable")[0] = 0xa5;
        assert_eq!(pool.bytes(slot).expect("readable")[0], 0xa5);
    }
}
