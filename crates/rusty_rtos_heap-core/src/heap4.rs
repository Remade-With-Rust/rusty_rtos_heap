//! `heap_4` remade: first fit with coalescing, byte for byte.
//!
//! The C original is `FreeRTOS-Kernel/portable/MemMang/heap_4.c`, and this
//! is a transcription of its algorithm rather than a re-design. Every
//! decision it makes — which free block is chosen, whether that block is
//! split, where the remainder lands in the address-ordered free list,
//! which neighbours coalesce on a free — is reproduced here so that the
//! two can be **diffed against each other** on the same request sequence.
//!
//! # Offsets, not pointers
//!
//! The C stores each block's header *inside* the block and threads the
//! free list through raw pointers. This crate is `forbid(unsafe)` and the
//! family's rule is that handles are indices, so the arena is a byte array
//! and every link is an **offset into it**. The header is written into the
//! block exactly where the C writes it — two machine words at the block's
//! own address — so the arithmetic, the split points and the coalescing
//! tests are the same arithmetic, and the offsets this returns are the
//! offsets the C returns.
//!
//! That is what makes the differential possible: a pointer is not
//! comparable across two programs, and an offset is.
//!
//! # The geometry is the C's, and it is explicit
//!
//! `LINK` and `ALIGN` are the const parameters because `sizeof(
//! BlockLink_t )` and `portBYTE_ALIGNMENT` are what set the block
//! granularity, the minimum block size and the split threshold. The oracle
//! runs on x86-64 with `portBYTE_ALIGNMENT` 8, so it is `Heap4<N, 8, 16>`
//! that reproduces it. Naming them rather than hard-coding 16 is what lets
//! the same code be diffed against a 32-bit build later.
//!
//! # What it is not
//!
//! It does not hand out pointers, so it does not implement
//! `rusty_rtos_core::heap::Heap` on its own. That seam takes a
//! `NonNull<u8>`, and turning an offset into one is the job of the crate
//! that owns the storage. This is the allocator; that is the binding.

use core::mem::size_of;

/// The free-list terminator. The C uses a null pointer; an offset of all
/// ones cannot be a real offset into an arena that fits in memory.
const NONE: u64 = u64::MAX;

/// `heapBLOCK_ALLOCATED_BITMASK`: the top bit of the size word marks a
/// block as the application's rather than the free list's.
///
/// Modelled at **64 bits** because the oracle's `size_t` is 64 bits. The
/// bit's position is part of the geometry, not of the host.
const ALLOCATED_BIT: u64 = 1 << 63;

/// Round `value` up to a multiple of `align`, saturating.
const fn align_up(value: usize, align: usize) -> usize {
    let mask = align.saturating_sub(1);
    value.saturating_add(mask) & !mask
}

/// `heap_4.c` over an `N`-byte arena.
///
/// * `ALIGN` is `portBYTE_ALIGNMENT`.
/// * `LINK` is `sizeof( BlockLink_t )` — one pointer plus one `size_t`.
#[derive(Debug)]
pub struct Heap4<const N: usize, const ALIGN: usize, const LINK: usize> {
    /// The arena. Block headers live inside it, where the C puts them.
    store: [u8; N],
    /// `xStart.pxNextFreeBlock`. `xStart` itself is a static outside the
    /// arena in the C, so it is a field here and never an offset — which
    /// also makes its coalescing test false by construction, exactly as an
    /// arbitrary static address makes it false there.
    start_next: u64,
    /// The offset of `pxEnd`, which *is* in the arena.
    end: u64,
    /// `xFreeBytesRemaining`.
    free_bytes: usize,
    /// `xMinimumEverFreeBytesRemaining`.
    minimum_ever_free: usize,
    /// `xNumberOfSuccessfulAllocations`.
    allocations: usize,
    /// `xNumberOfSuccessfulFrees`.
    frees: usize,
    /// Whether `prvHeapInit` has run. The C tests `pxEnd == NULL`.
    initialised: bool,
}

impl<const N: usize, const ALIGN: usize, const LINK: usize> Heap4<N, ALIGN, LINK> {
    /// `xHeapStructSize`: the header, rounded up to the alignment.
    pub const STRUCT_SIZE: usize = align_up(LINK, ALIGN);
    /// `heapMINIMUM_BLOCK_SIZE`: twice the header. A remainder must be
    /// **strictly greater** than this to be worth splitting off.
    pub const MINIMUM_BLOCK_SIZE: usize = Self::STRUCT_SIZE.saturating_mul(2);

    /// An uninitialised arena. `prvHeapInit` runs on the first allocation,
    /// as it does in the C.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            store: [0; N],
            start_next: NONE,
            end: NONE,
            free_bytes: 0,
            minimum_ever_free: 0,
            allocations: 0,
            frees: 0,
            initialised: false,
        }
    }

    // ---- the header, read and written where the C keeps it -------------

    fn read_word(&self, offset: u64, word: usize) -> u64 {
        let base = usize::try_from(offset).unwrap_or(usize::MAX);
        let at = base.saturating_add(word.saturating_mul(size_of::<u64>()));
        let end = at.saturating_add(size_of::<u64>());
        if end > N {
            return 0;
        }
        let mut bytes = [0u8; size_of::<u64>()];
        bytes.copy_from_slice(&self.store[at..end]);
        u64::from_ne_bytes(bytes)
    }

    fn write_word(&mut self, offset: u64, word: usize, value: u64) {
        let base = usize::try_from(offset).unwrap_or(usize::MAX);
        let at = base.saturating_add(word.saturating_mul(size_of::<u64>()));
        let end = at.saturating_add(size_of::<u64>());
        if end > N {
            return;
        }
        self.store[at..end].copy_from_slice(&value.to_ne_bytes());
    }

    /// `pxBlock->pxNextFreeBlock`.
    fn next_of(&self, offset: u64) -> u64 {
        self.read_word(offset, 0)
    }

    fn set_next(&mut self, offset: u64, value: u64) {
        self.write_word(offset, 0, value);
    }

    /// `pxBlock->xBlockSize`, allocated bit included.
    fn raw_size_of(&self, offset: u64) -> u64 {
        self.read_word(offset, 1)
    }

    fn set_raw_size(&mut self, offset: u64, value: u64) {
        self.write_word(offset, 1, value);
    }

    /// The block's size with the allocated bit masked off.
    fn size_of(&self, offset: u64) -> u64 {
        self.raw_size_of(offset) & !ALLOCATED_BIT
    }

    /// `heapBLOCK_IS_ALLOCATED`.
    fn is_allocated(&self, offset: u64) -> bool {
        (self.raw_size_of(offset) & ALLOCATED_BIT) != 0
    }

    /// `xStart.pxNextFreeBlock` or `pxIterator->pxNextFreeBlock`, with
    /// `None` standing for the `xStart` sentinel that lives outside the
    /// arena.
    fn next_from(&self, iterator: Option<u64>) -> u64 {
        match iterator {
            None => self.start_next,
            Some(offset) => self.next_of(offset),
        }
    }

    fn set_next_from(&mut self, iterator: Option<u64>, value: u64) {
        match iterator {
            None => self.start_next = value,
            Some(offset) => self.set_next(offset, value),
        }
    }

    // ---- prvHeapInit ---------------------------------------------------

    /// `prvHeapInit`. Idempotent, and called lazily exactly where the C
    /// calls it: inside the first `pvPortMalloc`.
    fn init(&mut self) {
        if self.initialised {
            return;
        }
        self.initialised = true;

        // The C aligns the start address and takes the loss out of the
        // total. Offset 0 of this arena stands for the aligned base, so
        // the alignment is the caller's to arrange and the loss is zero
        // here — which is why the differential's C side declares its own
        // `ucHeap` with `configAPPLICATION_ALLOCATED_HEAP` and aligns it.
        let total = N;
        let start_address: u64 = 0;

        // `uxEndAddress = uxStartAddress + xTotalHeapSize - xHeapStructSize`,
        // then aligned DOWN.
        let raw_end = total.saturating_sub(Self::STRUCT_SIZE);
        let mask = ALIGN.saturating_sub(1);
        let end_address = raw_end & !mask;
        let end = end_address as u64;

        self.end = end;
        self.set_raw_size(end, 0);
        self.set_next(end, NONE);

        // One free block covering everything up to `pxEnd`.
        let first_size = end_address.saturating_sub(start_address as usize);
        self.set_raw_size(start_address, first_size as u64);
        self.set_next(start_address, end);
        self.start_next = start_address;

        self.free_bytes = first_size;
        self.minimum_ever_free = first_size;
    }

    // ---- pvPortMalloc --------------------------------------------------

    /// `pvPortMalloc`, answering the offset of the **user** bytes — the
    /// address the C returns, which is the block plus its header.
    ///
    /// `None` is the C's `NULL`.
    pub fn alloc(&mut self, wanted: usize) -> Option<u64> {
        if wanted == 0 {
            return None;
        }
        // `xWantedSize += xHeapStructSize`, then round up to the alignment.
        let with_header = wanted.checked_add(Self::STRUCT_SIZE)?;
        let size = align_up(with_header, ALIGN);
        // `heapBLOCK_SIZE_IS_VALID`: the top bit is the allocated flag, so
        // a request that reaches it is refused rather than mis-tagged.
        if (size as u64 & ALLOCATED_BIT) != 0 {
            return None;
        }

        self.init();

        if size > self.free_bytes {
            return None;
        }

        // Walk the address-ordered free list for the first block that fits.
        let mut previous: Option<u64> = None;
        let mut block = self.start_next;
        while self.size_of(block) < size as u64 && self.next_of(block) != NONE {
            previous = Some(block);
            block = self.next_of(block);
        }
        // Reaching `pxEnd` means nothing was large enough.
        if block == self.end {
            return None;
        }

        // The C reads the returned block back out of `pxPreviousBlock`,
        // which is the same block it just walked to.
        let chosen = self.next_from(previous);
        let user = chosen.saturating_add(Self::STRUCT_SIZE as u64);

        // Unlink it.
        let after = self.next_of(chosen);
        self.set_next_from(previous, after);

        // Split only when the remainder is STRICTLY larger than the
        // minimum — an equal remainder is left attached, which is the line
        // that decides how fragmented the heap gets.
        let chosen_size = self.size_of(chosen);
        let remainder = chosen_size.saturating_sub(size as u64);
        if remainder > Self::MINIMUM_BLOCK_SIZE as u64 {
            let new_block = chosen.saturating_add(size as u64);
            self.set_raw_size(new_block, remainder);
            self.set_raw_size(chosen, size as u64);
            // The remainder takes the chosen block's place in the list.
            let previous_next = self.next_from(previous);
            self.set_next(new_block, previous_next);
            self.set_next_from(previous, new_block);
        }

        let taken = self.size_of(chosen);
        self.free_bytes = self.free_bytes.saturating_sub(taken as usize);
        if self.free_bytes < self.minimum_ever_free {
            self.minimum_ever_free = self.free_bytes;
        }

        // `heapALLOCATE_BLOCK` and the null link the free path asserts on.
        self.set_raw_size(chosen, self.raw_size_of(chosen) | ALLOCATED_BIT);
        self.set_next(chosen, NONE);
        self.allocations = self.allocations.saturating_add(1);
        Some(user)
    }

    // ---- vPortFree -----------------------------------------------------

    /// `vPortFree`, taking the offset [`Heap4::alloc`] answered.
    ///
    /// A block that is not allocated, or whose link is not null, is
    /// ignored — the C `configASSERT`s both and then does nothing, and
    /// doing nothing is what a release build does.
    pub fn free(&mut self, user: u64) {
        let Some(link) = user.checked_sub(Self::STRUCT_SIZE as u64) else {
            return;
        };
        if !self.is_allocated(link) || self.next_of(link) != NONE {
            return;
        }
        // `heapFREE_BLOCK`.
        self.set_raw_size(link, self.raw_size_of(link) & !ALLOCATED_BIT);
        self.free_bytes = self.free_bytes.saturating_add(self.size_of(link) as usize);
        self.insert_into_free_list(link);
        self.frees = self.frees.saturating_add(1);
    }

    /// `prvInsertBlockIntoFreeList`: address-ordered insert that coalesces
    /// with the block before and the block after.
    fn insert_into_free_list(&mut self, block: u64) {
        let mut insert = block;

        // Walk to the position, which is the address order the whole
        // design rests on.
        let mut iterator: Option<u64> = None;
        while self.next_from(iterator) < insert {
            iterator = Some(self.next_from(iterator));
        }

        // Coalesce with the block BEFORE, if it ends exactly here.
        // `xStart` can never satisfy this: it is not in the arena, which
        // is why `iterator` is an `Option` rather than an offset.
        if let Some(previous) = iterator {
            if previous.saturating_add(self.size_of(previous)) == insert {
                self.set_raw_size(
                    previous,
                    self.size_of(previous).saturating_add(self.size_of(insert)),
                );
                insert = previous;
            }
        }

        // Coalesce with the block AFTER, unless that block is `pxEnd`,
        // which is a marker and must not be absorbed.
        let following = self.next_from(iterator);
        if insert.saturating_add(self.size_of(insert)) == following {
            if following == self.end {
                self.set_next(insert, self.end);
            } else {
                self.set_raw_size(
                    insert,
                    self.size_of(insert).saturating_add(self.size_of(following)),
                );
                let after = self.next_of(following);
                self.set_next(insert, after);
            }
        } else {
            self.set_next(insert, following);
        }

        // Only relink when the block did not merge into its predecessor;
        // if it did, the predecessor is already in the list.
        if insert != iterator.unwrap_or(NONE) {
            self.set_next_from(iterator, insert);
        }
    }

    // ---- the reporting the C exposes -----------------------------------

    /// `xPortGetFreeHeapSize`.
    #[must_use]
    pub const fn free_bytes(&self) -> usize {
        self.free_bytes
    }

    /// `xPortGetMinimumEverFreeHeapSize`.
    #[must_use]
    pub const fn minimum_ever_free_bytes(&self) -> usize {
        self.minimum_ever_free
    }

    /// `xNumberOfSuccessfulAllocations`.
    #[must_use]
    pub const fn allocations(&self) -> usize {
        self.allocations
    }

    /// `xNumberOfSuccessfulFrees`.
    #[must_use]
    pub const fn frees(&self) -> usize {
        self.frees
    }

    /// How many blocks the free list holds, and the largest of them —
    /// `vPortGetHeapStats`' two interesting fields, and the pair that says
    /// how fragmented the arena is when `free_bytes` alone cannot.
    #[must_use]
    pub fn free_list_shape(&self) -> (usize, usize) {
        let mut count = 0usize;
        let mut largest = 0usize;
        let mut block = self.start_next;
        while block != NONE && block != self.end {
            count = count.saturating_add(1);
            let size = self.size_of(block) as usize;
            if size > largest {
                largest = size;
            }
            block = self.next_of(block);
        }
        (count, largest)
    }
}

/// The footprint identity, checked ON EVERY TARGET the gate builds.
///
/// A `Heap4` costs its arena plus a FIXED bookkeeping block — the free
/// counters, the two sentinels and the init flag — and nothing that grows
/// with `N`. A host test can print the bytes but cannot prove this for a
/// 32-bit target, because `size_of` there answers about the host's
/// pointers. A `const` assertion can: it is evaluated by the compiler for
/// whichever target is being built, so `kairos check`'s four bare-metal
/// rungs verify it on ARMv7-M, ARMv8-M and both RISC-V profiles.
///
/// Expressed as an identity rather than a literal because the literal
/// differs by pointer width, and a number that has to be edited per target
/// is a number that stops being checked.
const _: () = {
    let small = size_of::<Heap4<1024, 8, 8>>() - 1024;
    let large = size_of::<Heap4<8192, 8, 8>>() - 8192;
    assert!(
        small == large,
        "heap_4's bookkeeping grew with the arena: it is not fixed overhead"
    );
};

const _: () = {
    let thirty_two = Heap4::<1024, 8, 8>::STRUCT_SIZE;
    let sixty_four = Heap4::<1024, 8, 16>::STRUCT_SIZE;
    // The header is one pointer plus one `size_t`, so the 64-bit geometry
    // costs exactly twice the 32-bit one — and the minimum block, which is
    // twice the header, moves with it. This is the line a firmware sizing
    // an arena needs, and it is asserted rather than described.
    assert!(sixty_four == thirty_two * 2);
    assert!(Heap4::<1024, 8, 8>::MINIMUM_BLOCK_SIZE == thirty_two * 2);
    assert!(Heap4::<1024, 8, 16>::MINIMUM_BLOCK_SIZE == sixty_four * 2);
};

impl<const N: usize, const ALIGN: usize, const LINK: usize> Default for Heap4<N, ALIGN, LINK> {
    fn default() -> Self {
        Self::new()
    }
}
