//! `heap_1.c` over an `N`-byte arena: allocate, never free.
//!
//! The simplest of FreeRTOS's schemes and the one most systems should use. A
//! bump index, no headers, no free list, and `vPortFree` is a no-op. It exists
//! because a great many embedded systems allocate every task, queue and timer
//! at start-up and then never allocate again — and for those, an allocator
//! that cannot fragment is worth more than one that can free.
//!
//! # Three things here are the C's quirks, not a design
//!
//! A reimplementation would get all three wrong, and the differential is what
//! would catch it:
//!
//! 1. **The alignment is applied to the REQUEST, before the bump** — not to
//!    the resulting offset. `heap_4` adds a header and then aligns; this
//!    rounds `xWantedSize` up and bumps by the rounded figure, so the
//!    allocator's stride is the aligned size and the offsets follow from it.
//! 2. **The bound is STRICTLY less than.** `( xNextFreeByte + xWantedSize ) <
//!    configADJUSTED_HEAP_SIZE`, so the last byte of the adjusted arena can
//!    never be handed out. Writing `<=` would accept one request the C
//!    refuses — exactly the one-byte divergence a differential exists to find.
//! 3. **The usable arena is `N - ALIGN`, not `N`.** `configADJUSTED_HEAP_SIZE`
//!    subtracts a whole alignment because the C aligns `ucHeap`'s START at run
//!    time and can lose up to `ALIGN - 1` bytes doing it. We align by
//!    construction and lose nothing — and subtract it anyway, because the
//!    number this allocator refuses on has to be the number the C refuses on.
//!
//! The third is worth sitting with: it is arithmetic we do not need, kept
//! because matching the oracle matters more than reclaiming a few bytes. If
//! that ever stops being true it should be a decision with a row in the
//! ledger, not a quiet improvement.

use rusty_rtos_core::error::{Error, Result};

/// `heap_1.c` over an `N`-byte arena.
///
/// * `ALIGN` is `portBYTE_ALIGNMENT`.
///
/// There is no `LINK` parameter because there are no block headers: the whole
/// scheme is one index.
#[derive(Debug)]
pub struct Heap1<const N: usize, const ALIGN: usize> {
    /// The arena. Unlike `heap_4` there are no headers in it — every byte is
    /// either handed out or not yet reached.
    store: [u8; N],
    /// `xNextFreeByte`.
    next_free: usize,
    /// How many allocations succeeded. The C does not keep this for `heap_1`;
    /// it is here because the differential compares it, and a counter is
    /// cheaper than inferring one.
    allocations: usize,
}

impl<const N: usize, const ALIGN: usize> Heap1<N, ALIGN> {
    /// `configADJUSTED_HEAP_SIZE`.
    ///
    /// The C subtracts a whole `portBYTE_ALIGNMENT` because it aligns the
    /// start of `ucHeap` at run time and can lose up to `ALIGN - 1` bytes.
    /// Ours is aligned by construction and loses none — and subtracts it
    /// anyway, so the size this allocator refuses on is the size the C refuses
    /// on. See the module docs.
    pub const ADJUSTED_SIZE: usize = N.saturating_sub(ALIGN);

    /// An empty arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            store: [0; N],
            next_free: 0,
            allocations: 0,
        }
    }

    /// `pvPortMalloc`, answering with an offset into the arena.
    ///
    /// `None` is the C's `NULL`: the request did not fit, or rounding it to
    /// the alignment would have overflowed.
    pub fn alloc(&mut self, wanted: usize) -> Option<u64> {
        // The C rounds the REQUEST up, before it looks at the room left.
        // `heapADD_WILL_OVERFLOW` guards that rounding and sets the size to
        // zero when it would wrap, which then fails the `> 0` test below — so
        // an overflowing request is refused rather than wrapping into a small
        // one that fits.
        let wanted = match wanted.checked_rem(ALIGN) {
            None | Some(0) => wanted,
            // `unwrap_or_default` rather than a match, and the default IS
            // the behaviour: `heapADD_WILL_OVERFLOW` sets the size to zero
            // when rounding would wrap, and zero then fails the `> 0` test
            // below. So an overflowing request is refused rather than
            // wrapping into a small one that happens to fit.
            Some(remainder) => wanted
                .checked_add(ALIGN.saturating_sub(remainder))
                .unwrap_or_default(),
        };

        // `( xWantedSize > 0 ) && !overflow && ( next + wanted < ADJUSTED )`.
        // STRICTLY less than, which is the C's and costs the last byte.
        if wanted == 0 {
            return None;
        }
        let end = self.next_free.checked_add(wanted)?;
        if end >= Self::ADJUSTED_SIZE {
            return None;
        }

        let offset = self.next_free;
        self.next_free = end;
        self.allocations = self.allocations.saturating_add(1);
        Some(offset as u64)
    }

    /// `vPortFree`, which in this scheme is **invalid to call**.
    ///
    /// Not a no-op — that was this function's first draft, and the C says
    /// otherwise in as many words:
    ///
    /// ```c
    /// /* Force an assert as it is invalid to call this function. */
    /// configASSERT( pv == NULL );
    /// ```
    ///
    /// So freeing a real allocation is a programming error in `heap_1`, and a
    /// version that answered `Ok` would let a caller do silently what the C
    /// stops them doing loudly. The differential found this before it ran a
    /// single operation: the driver called `vPortFree` on a live pointer to
    /// show it did nothing, and the C arm exited 2 with
    /// `configASSERT failed: pv == NULL`.
    ///
    /// # Errors
    ///
    /// Always [`Error::Unsupported`]. The signature matches
    /// [`crate::heap4::Heap4::free`] so a caller generic over the two needs
    /// one shape, and this is the answer that makes choosing `heap_1` for a
    /// system that frees fail at the first free instead of never.
    pub const fn free(&mut self, _offset: u64) -> Result<()> {
        Err(Error::Unsupported)
    }

    /// `xPortGetFreeHeapSize`.
    #[must_use]
    pub const fn free_bytes(&self) -> usize {
        Self::ADJUSTED_SIZE.saturating_sub(self.next_free)
    }

    /// `xNextFreeByte`, for the differential.
    #[must_use]
    pub const fn next_free_byte(&self) -> usize {
        self.next_free
    }

    /// How many allocations succeeded.
    #[must_use]
    pub const fn allocations(&self) -> usize {
        self.allocations
    }

    /// A real address for an offset, for a caller that needs one.
    pub fn address_of(&mut self, offset: u64) -> Option<core::ptr::NonNull<u8>> {
        let at = usize::try_from(offset).ok()?;
        let byte = self.store.get_mut(at)?;
        core::ptr::NonNull::new(core::ptr::from_mut(byte))
    }

    /// The bytes of an allocation, for a caller that wants to use it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] when the range is not inside the arena.
    pub fn bytes_mut(&mut self, offset: u64, len: usize) -> Result<&mut [u8]> {
        let at = usize::try_from(offset).map_err(|_| Error::InvalidArgument)?;
        let end = at.checked_add(len).ok_or(Error::InvalidArgument)?;
        self.store.get_mut(at..end).ok_or(Error::InvalidArgument)
    }
}

impl<const N: usize, const ALIGN: usize> Default for Heap1<N, ALIGN> {
    fn default() -> Self {
        Self::new()
    }
}
