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

use rusty_rtos_core::error::{Error, Result};

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

/// What [`Heap4::alloc`] answers with: where the block is, and **which**
/// allocation it was.
///
/// # Why this is not just an offset
///
/// `heap_4.c` hands back a `void *` and takes it back in `vPortFree`, and it
/// protects that round trip with three checks: the block must carry the
/// allocated bit, its link must be null, and — with
/// `configENABLE_HEAP_PROTECTOR` — free-list pointers are XORed with a random
/// canary so a corrupted one is not a usable address.
///
/// **Two of those three do not transfer, and the third does not need to.**
/// This heap holds bounds-checked `u64` offsets in a crate that is
/// `#![forbid(unsafe_code)]`: a corrupted offset is a wrong answer, never a
/// write to an address an attacker chose. Transcribing the canary would be
/// carrying a mitigation across to a bug class the representation already
/// removed.
///
/// What does survive the change of representation is the failure the canary
/// was never aimed at: **freeing a block that has since been reallocated**.
/// The allocated bit cannot see it — the block IS allocated, just not to you —
/// and an offset alone cannot distinguish the allocation you were given from
/// the one living there now. So the offset carries a generation, which is the
/// same answer `rusty_rtos_core::Handle` already gives for the same question
/// everywhere else in this family.
///
/// Eight bytes, `Copy`, and the offset is still available for anyone who needs
/// the raw number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    offset: u32,
    generation: u32,
}

impl Block {
    /// The user offset into the arena, which is what `heap_4.c`'s `void *`
    /// stands for here.
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset as u64
    }

    /// Which allocation this was. Monotonic per heap.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// One region of a `heap_5` heap: where it starts in the arena, and how big.
///
/// `HeapRegion_t` in the C, over offsets rather than a pointer. The space
/// between two regions is a real gap that no allocation crosses — which is
/// enforced by arithmetic rather than by a check, since coalescing is an
/// address comparison and a gap makes it false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// Where the region starts, as an offset into the arena.
    pub start: usize,
    /// How many bytes it covers.
    pub size: usize,
}

impl Region {
    /// A region at `start` covering `size` bytes.
    #[must_use]
    pub const fn new(start: usize, size: usize) -> Self {
        Self { start, size }
    }
}

/// `heap_4.c` over an `N`-byte arena — and, initialised with
/// [`Heap4::define_regions`], `heap_5.c` over several.
///
/// * `ALIGN` is `portBYTE_ALIGNMENT`.
/// * `LINK` is `sizeof( BlockLink_t )` — one pointer plus one `size_t`.
///
/// The two C files share `pvPortMalloc`, `vPortFree` and
/// `prvInsertBlockIntoFreeList` almost line for line and differ only in
/// initialisation, so there is one free list here and two ways to lay it out.
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
    /// The generation the next allocation is stamped with.
    ///
    /// Monotonic and never reused, so a [`Block`] from an earlier allocation
    /// at the same offset cannot match the one living there now. It wraps
    /// after 2^32 allocations, which is the one case a stale handle could
    /// collide; at the 20,000 operations the differential runs that is not a
    /// near thing, and a heap that reaches it has other problems.
    next_generation: u32,
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
            next_generation: 1,
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

    // ---- vPortDefineHeapRegions ----------------------------------------

    /// `vPortDefineHeapRegions`, which is `heap_5.c`'s initialiser.
    ///
    /// Each region becomes a free block covering it, plus an end marker at its
    /// top; the previous region's marker is then pointed at the next region's
    /// block, so one address-ordered free list threads all of them. That is
    /// exactly what the C does with pointers, over offsets instead.
    ///
    /// # The gaps have to be real
    ///
    /// Regions are sub-ranges of this arena, and the space BETWEEN them must
    /// be left alone. That is not bookkeeping: coalescing in
    /// [`Self::insert_into_free_list`] is an address comparison, and what
    /// stops two regions merging into one is that the first one's end is not
    /// the second one's start. Declare regions that abut and they will
    /// coalesce, correctly, because then they are one region.
    ///
    /// # Errors
    ///
    /// * [`Error::Busy`] — already initialised. The C is blunter:
    ///   `configASSERT( pxEnd == NULL )` under the comment "Can only call
    ///   once!".
    /// * [`Error::InvalidArgument`] — no regions, a region that does not fit
    ///   the arena, one too small to hold its own end marker, or regions
    ///   **out of address order**.
    ///
    /// That last one is the C's behaviour and not a convenience of ours:
    ///
    /// ```c
    /// /* Check blocks are passed in with increasing start addresses. */
    /// configASSERT( ( size_t ) xAddress > ( size_t ) pxEnd );
    /// ```
    ///
    /// It does **not** sort them. The plan for this work assumed it did, and
    /// assumed a transcription that expected sorted input would be the bug;
    /// the C says the opposite, so accepting unsorted regions would be the
    /// bug. Reading the source settled it, which is the whole reason the
    /// oracle is pinned.
    pub fn define_regions(&mut self, regions: &[Region]) -> Result<()> {
        if self.initialised {
            return Err(Error::Busy);
        }
        if regions.is_empty() {
            return Err(Error::InvalidArgument);
        }

        let mask = ALIGN.saturating_sub(1);
        let mut previous_end: Option<u64> = None;
        let mut total = 0usize;

        for region in regions {
            // `xAddress` aligned UP, with the loss taken out of the size --
            // the C adjusts `xTotalRegionSize` by exactly what alignment cost.
            let start = region.start.saturating_add(mask) & !mask;
            let lost = start.saturating_sub(region.start);
            let size = region.size.saturating_sub(lost);

            // The end marker sits at the top, aligned DOWN.
            let raw_end = start.saturating_add(size).saturating_sub(Self::STRUCT_SIZE);
            let end = raw_end & !mask;

            // A region must fit, and must have room for a block AND its marker.
            if start.saturating_add(size) > N
                || end <= start
                || end.saturating_add(Self::STRUCT_SIZE) > N
            {
                return Err(Error::InvalidArgument);
            }

            let start = start as u64;
            let end = end as u64;

            // Increasing start addresses, which the C asserts rather than sorts.
            if let Some(previous) = previous_end {
                if start <= previous {
                    return Err(Error::InvalidArgument);
                }
            }

            self.set_raw_size(end, 0);
            self.set_next(end, NONE);

            let block_size = end.saturating_sub(start);
            self.set_raw_size(start, block_size);
            self.set_next(start, end);

            match previous_end {
                // `xStart.pxNextFreeBlock = xAlignedHeap` for the first region.
                None => self.start_next = start,
                // Otherwise the PREVIOUS region's marker points at this block,
                // which is what threads the regions into one list.
                Some(previous) => self.set_next(previous, start),
            }

            total = total.saturating_add(block_size as usize);
            previous_end = Some(end);
            self.end = end;
        }

        // `configASSERT( xTotalHeapSize )`.
        if total == 0 {
            return Err(Error::InvalidArgument);
        }

        self.initialised = true;
        self.free_bytes = total;
        self.minimum_ever_free = total;
        Ok(())
    }

    // ---- pvPortMalloc --------------------------------------------------

    /// `pvPortMalloc`, answering the offset of the **user** bytes — the
    /// address the C returns, which is the block plus its header.
    ///
    /// `None` is the C's `NULL`.
    pub fn alloc(&mut self, wanted: usize) -> Option<Block> {
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

        // `heapALLOCATE_BLOCK`, and then the link word.
        //
        // The C writes NULL there and `vPortFree` asserts on it. We write the
        // GENERATION instead, which costs nothing: the store happens either
        // way, and an allocated block's link is dead space in both designs.
        // The allocated bit still distinguishes a live block from a free one,
        // so the free path checks that first and never mistakes a free
        // block's next pointer for a generation.
        self.set_raw_size(chosen, self.raw_size_of(chosen) | ALLOCATED_BIT);
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        self.set_next(chosen, u64::from(generation));
        self.allocations = self.allocations.saturating_add(1);
        Some(Block {
            offset: u32::try_from(user).unwrap_or(u32::MAX),
            generation,
        })
    }

    // ---- vPortFree -----------------------------------------------------

    /// `vPortFree`, taking the [`Block`] [`Heap4::alloc`] answered.
    ///
    /// # Why this reports instead of ignoring
    ///
    /// The C `configASSERT`s its checks and then does nothing, so a release
    /// build silently ignores a bad free. This returned `()` and did the same,
    /// which is worse than the C rather than equal to it: the C at least
    /// stops in a debug build, and a caller here had no way to learn anything
    /// at all. A silent no-op on a double free is a bug that erases its own
    /// evidence.
    ///
    /// Nothing is corrupted either way — that is what `forbid(unsafe_code)`
    /// and bounds-checked offsets buy. What changes is that the caller can
    /// now tell.
    ///
    /// # Errors
    ///
    /// * [`Error::InvalidArgument`] — an offset this allocator could not have
    ///   handed out: before the first block, past the arena, or not on an
    ///   alignment boundary. That last one catches an INTERIOR offset, which
    ///   `heapVALIDATE_BLOCK_POINTER` does not: the C only range-checks, so a
    ///   pointer into the middle of a live block passes it and then reads a
    ///   header out of user data.
    /// * [`Error::Gone`] — the block is not allocated (a double free, or an
    ///   offset that was never allocated), or it has been reallocated since
    ///   this [`Block`] was handed out.
    pub fn free(&mut self, block: Block) -> Result<()> {
        let user = block.offset();
        let Some(link) = user.checked_sub(Self::STRUCT_SIZE as u64) else {
            return Err(Error::InvalidArgument);
        };
        // `heapVALIDATE_BLOCK_POINTER`, and then the part it does not do.
        // A block start is always aligned, so an offset that is not cannot
        // name one -- which is the cheapest way to refuse an interior offset
        // before it is used to read a header out of somebody's data.
        // `checked_rem` rather than `%`: `ALIGN` is a const generic, so nothing
        // stops a caller instantiating it at 0, and a remainder by zero
        // panics. `None` then means "no alignment to be on", which is not an
        // offset this allocator could have produced either -- so both arms of
        // the comparison refuse, which is the answer wanted.
        if link.saturating_add(Self::STRUCT_SIZE as u64) > N as u64
            || link.checked_rem(ALIGN as u64) != Some(0)
        {
            return Err(Error::InvalidArgument);
        }
        // The allocated bit FIRST: a free block's link word holds a next
        // pointer, and reading that as a generation is how this check would
        // fool itself.
        if !self.is_allocated(link) {
            return Err(Error::Gone);
        }
        if self.next_of(link) != u64::from(block.generation()) {
            return Err(Error::Gone);
        }
        self.free_at(link);
        Ok(())
    }

    /// `vPortFree`, taking a bare offset — **the protection an address can
    /// carry, which is `heap_4.c`'s and no more.**
    ///
    /// [`Self::free`] refuses a stale handle because a [`Block`] carries the
    /// generation it was handed out under. A C caller has no [`Block`]:
    /// `vPortFree( void * )` takes an address and nothing else, which is
    /// exactly why `heap_4.c` can manage only the allocated-bit check itself.
    /// So the C ABI uses this, and gets what FreeRTOS gets.
    ///
    /// That is a real difference and worth naming rather than blurring: a
    /// double free is caught here, and a free of a REALLOCATED block is not,
    /// because an address alone cannot distinguish the allocation you were
    /// given from the one living there now. A Rust caller should use
    /// [`Self::free`] and be told; a C caller cannot be, and the limit is the
    /// C's type, not this heap's.
    ///
    /// # Errors
    ///
    /// * [`Error::InvalidArgument`] — an offset this allocator could not have
    ///   handed out: before the first block, past the arena, or not on an
    ///   alignment boundary.
    /// * [`Error::Gone`] — the block is not allocated, which is a double free
    ///   or an offset that was never allocated.
    pub fn free_raw(&mut self, offset: u64) -> Result<()> {
        let Some(link) = offset.checked_sub(Self::STRUCT_SIZE as u64) else {
            return Err(Error::InvalidArgument);
        };
        if link.saturating_add(Self::STRUCT_SIZE as u64) > N as u64
            || link.checked_rem(ALIGN as u64) != Some(0)
        {
            return Err(Error::InvalidArgument);
        }
        if !self.is_allocated(link) {
            return Err(Error::Gone);
        }
        self.free_at(link);
        Ok(())
    }

    /// The part of `vPortFree` after the checks, shared by both entry points.
    fn free_at(&mut self, link: u64) {
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

    // ---- the binding: offsets out, pointers in -------------------------

    /// The address of an offset this heap returned.
    ///
    /// The allocator speaks offsets because an offset is comparable across
    /// two programs and a pointer is not, which is what made the K4
    /// differential against `heap_4.c` possible at all. A C ABI speaks
    /// pointers. This is the one place the two meet, and it lives here
    /// because `store` is private and should stay so — the module docs
    /// call this "the binding" and put it in whatever owns the storage.
    ///
    /// Bounds-checked: an offset outside the arena answers `None` rather
    /// than forming a pointer that is not in it.
    pub fn address_of(&mut self, offset: u64) -> Option<core::ptr::NonNull<u8>> {
        let index = usize::try_from(offset).ok()?;
        if index >= N {
            return None;
        }
        core::ptr::NonNull::new(self.store.as_mut_ptr().wrapping_add(index))
    }

    /// The inverse, for `free`: the offset of a pointer this heap handed
    /// out, or `None` if it did not come from this arena.
    ///
    /// A pointer from somewhere else is the caller's bug, and answering
    /// `None` makes it a refusal rather than a corrupted free list.
    #[must_use]
    pub fn offset_of(&self, pointer: *const u8) -> Option<u64> {
        let base = self.store.as_ptr() as usize;
        let address = pointer as usize;
        if address < base || address >= base.saturating_add(N) {
            return None;
        }
        // The guard above already proves `address >= base`, so this
        // subtraction cannot wrap -- but a proof the compiler cannot see
        // is not one the lint will accept, and `checked_sub` says the
        // same thing in a form that survives a refactor of the guard.
        address
            .checked_sub(base)
            .and_then(|offset| u64::try_from(offset).ok())
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "a test asserts; the crate's deny-by-default is for library code"
)]
mod protector_tests {
    //! The refusals that need a FORGED offset.
    //!
    //! [`Block`] has no public constructor, deliberately: inventing one is the
    //! bug the generation exists to catch. These live here, where the fields
    //! are in scope, rather than widening the real API so an integration test
    //! can reach them. The cases a caller could reach with a real handle --
    //! double free, freeing a reallocated block -- are in `tests/protector.rs`.

    use super::{Block, Heap4};
    use rusty_rtos_core::error::Error;

    const TOTAL: usize = 8192;
    const ALIGN: usize = 8;
    const LINK: usize = 8;

    type Heap = Heap4<TOTAL, ALIGN, LINK>;

    /// A handle no honest caller could hold.
    const fn forged(offset: u64, generation: u32) -> Block {
        Block {
            offset: offset as u32,
            generation,
        }
    }

    /// An interior offset is refused -- and this is the case `heap_4.c` does
    /// NOT catch.
    ///
    /// `heapVALIDATE_BLOCK_POINTER` only range-checks, so a pointer into the
    /// middle of a live block is inside the heap, passes, and is then used to
    /// read a header out of user data. A block start is always aligned, so an
    /// offset that is not cannot name one -- the cheapest possible refusal.
    #[test]
    fn an_interior_offset_is_refused() {
        let mut heap = Heap::new();
        let block = heap.alloc(128).expect("room for 128 bytes");

        let interior = forged(block.offset().saturating_add(1), block.generation());
        assert_eq!(heap.free(interior), Err(Error::InvalidArgument));

        assert_eq!(heap.free(block), Ok(()), "the real one still frees");
    }

    /// An offset past the arena is refused rather than silently ignored.
    #[test]
    fn an_offset_past_the_arena_is_refused() {
        let mut heap = Heap::new();
        let _ = heap.alloc(64).expect("lay the arena out");
        let bogus = forged(TOTAL as u64 + ALIGN as u64, 1);
        assert_eq!(heap.free(bogus), Err(Error::InvalidArgument));
    }

    /// An offset too small to carry a header behind it is refused.
    #[test]
    fn an_offset_below_the_first_header_is_refused() {
        let mut heap = Heap::new();
        let _ = heap.alloc(64).expect("lay the arena out");
        assert_eq!(heap.free(forged(0, 1)), Err(Error::InvalidArgument));
    }

    /// An aligned, in-range offset that was never allocated is `Gone`.
    ///
    /// It lands inside the one big free block, whose allocated bit is clear --
    /// which is the check that catches it, and the reason the allocated bit is
    /// tested BEFORE the generation: a free block's link word holds a next
    /// pointer, and reading that as a generation is how this check would fool
    /// itself.
    #[test]
    fn an_offset_that_was_never_allocated_is_refused() {
        let mut heap = Heap::new();
        let live = heap.alloc(64).expect("room for 64 bytes");
        let never = forged(live.offset().saturating_add(1024), 1);
        assert_eq!(heap.free(never), Err(Error::Gone));
        assert_eq!(heap.free(live), Ok(()));
    }

    /// A right offset with a wrong generation is refused.
    ///
    /// The narrowest case: everything about the handle is correct except which
    /// allocation it names.
    #[test]
    fn the_right_offset_with_the_wrong_generation_is_refused() {
        let mut heap = Heap::new();
        let block = heap.alloc(64).expect("room for 64 bytes");

        let wrong = forged(block.offset(), block.generation().wrapping_add(1));
        assert_eq!(heap.free(wrong), Err(Error::Gone));
        assert_eq!(heap.free(block), Ok(()));
    }
}
