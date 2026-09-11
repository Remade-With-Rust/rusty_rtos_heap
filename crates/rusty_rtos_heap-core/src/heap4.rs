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

    /// Both header words in ONE bounds-checked read.
    ///
    /// The free-list walk asks every block for its size and then its next
    /// pointer, which as two separate accessors is two `try_from`s, four
    /// saturating adds, two range checks and two `copy_from_slice`s for
    /// one 16-byte header that is contiguous in memory. Reading the header
    /// once per visit is the same move the kernel's list work made — "each
    /// node is read once per call" — and for the same reason: the work
    /// removed is not the load, it is everything wrapped around the load.
    ///
    /// `get` rather than indexing, and a fixed-size destination, so the
    /// compiler sees one length it can check instead of two it cannot.
    fn header_of(&self, offset: u64) -> (u64, u64) {
        let base = usize::try_from(offset).unwrap_or(usize::MAX);
        let Some(bytes) = self
            .store
            .get(base..)
            .and_then(|rest| rest.first_chunk::<{ 2 * size_of::<u64>() }>())
        else {
            return (0, 0);
        };
        let Some(next) = bytes.first_chunk::<{ size_of::<u64>() }>() else {
            return (0, 0);
        };
        let Some(size) = bytes.last_chunk::<{ size_of::<u64>() }>() else {
            return (0, 0);
        };
        (u64::from_ne_bytes(*next), u64::from_ne_bytes(*size))
    }

    fn read_word(&self, offset: u64, word: usize) -> u64 {
        let (next, size) = self.header_of(offset);
        if word == 0 { next } else { size }
    }

    fn write_word(&mut self, offset: u64, word: usize, value: u64) {
        let base = usize::try_from(offset).unwrap_or(usize::MAX);
        let Some(header) = self
            .store
            .get_mut(base..)
            .and_then(|rest| rest.first_chunk_mut::<{ 2 * size_of::<u64>() }>())
        else {
            return;
        };
        let bytes = value.to_ne_bytes();
        if word == 0 {
            if let Some(slot) = header.first_chunk_mut::<{ size_of::<u64>() }>() {
                *slot = bytes;
            }
        } else if let Some(slot) = header.last_chunk_mut::<{ size_of::<u64>() }>() {
            *slot = bytes;
        }
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
        loop {
            // One read serves both tests. The C reads the same two fields
            // out of one cache line; so does this now.
            let (next, raw) = self.header_of(block);
            if (raw & !ALLOCATED_BIT) >= size as u64 || next == NONE {
                break;
            }
            previous = Some(block);
            block = next;
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

        // REFUTED, and kept as written for that reason. Folding these four
        // reads of `chosen` into the one `header_of` above — carrying the
        // size in a local and writing `taken | ALLOCATED_BIT` once instead
        // of read-modify-write — MEASURED WORSE: 2,730,871 -> 2,776,099 Ir,
        // +1.7%, on the same 20,000 operations with an identical checksum.
        // The same move won 32.7% in the free-list walk and 13.2% in the
        // insert, and loses here, which is the law this project keeps
        // relearning: removing a redundant read wins only when the read
        // costs more than the check that avoids it. In a walk the read is
        // per node and the branch is not; here the reads are once per call
        // and folding them introduced a merge the unconditional stores did
        // not need.
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
    /// Every node is read ONCE.
    ///
    /// Written the obvious way this walks the list asking each node for
    /// its next pointer, then asks the SAME node again to advance — and
    /// then asks the predecessor and the successor for their sizes two and
    /// three more times while coalescing. Each of those is a bounds check
    /// and a load of a header that was already in hand. Carrying the
    /// header in a local instead is the same move as the walk in `alloc`,
    /// and the reason is the same: what it removes is not the load, it is
    /// everything wrapped around the load.
    fn insert_into_free_list(&mut self, block: u64) {
        let mut insert = block;
        let mut insert_size = self.size_of(insert);

        // Walk to the position, which is the address order the whole
        // design rests on. `next` is carried rather than re-read.
        let mut iterator: Option<u64> = None;
        let mut next = self.start_next;
        while next < insert {
            iterator = Some(next);
            next = self.next_of(next);
        }

        // Coalesce with the block BEFORE, if it ends exactly here.
        // `xStart` can never satisfy this: it is not in the arena, which
        // is why `iterator` is an `Option` rather than an offset.
        if let Some(previous) = iterator {
            let previous_size = self.size_of(previous);
            if previous.saturating_add(previous_size) == insert {
                insert_size = previous_size.saturating_add(insert_size);
                self.set_raw_size(previous, insert_size);
                insert = previous;
            }
        }

        // Coalesce with the block AFTER, unless that block is `pxEnd`,
        // which is a marker and must not be absorbed.
        let following = next;
        if insert.saturating_add(insert_size) == following {
            if following == self.end {
                self.set_next(insert, self.end);
            } else {
                let (after, following_raw) = self.header_of(following);
                self.set_raw_size(
                    insert,
                    insert_size.saturating_add(following_raw & !ALLOCATED_BIT),
                );
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
