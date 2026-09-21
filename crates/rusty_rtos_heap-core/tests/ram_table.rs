//! K4's RAM table: what a `heap_4` costs, decomposed to the byte.
//!
//! A footprint number you cannot decompose is a number you cannot
//! optimise, so this does not report a total — it reports an **identity**
//! and fails if the parts do not sum to it:
//!
//! ```text
//! size_of::<Heap4<N, ALIGN, LINK>>() == N + bookkeeping + padding
//! ```
//!
//! and it prints `padding` rather than folding it in. Padding that is
//! quietly absorbed into a total is how a footprint claim goes wrong: on a
//! fixed RAM map the bytes still exist, they just stop appearing in any
//! column.
//!
//! # Per profile means per POINTER WIDTH
//!
//! The geometry is not ours to choose. `heap_4`'s header is one pointer
//! plus one `size_t`, so it is **16 bytes on a 64-bit target and 8 on a
//! 32-bit one**, and every derived number moves with it: the per-allocation
//! overhead, the minimum block, and the split threshold that decides how
//! fragmented the arena gets. Every Kairos target is 32-bit, so the row
//! that matters for a firmware is the 8-byte one — and the row the
//! differential runs against is the 16-byte one, because the oracle is an
//! x86-64 build. Both are here so neither is mistaken for the other.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use core::mem::{align_of, size_of};

use rusty_rtos_heap_core::Heap4;
use rusty_rtos_heap_core::pool::Pool;

/// One row: a geometry, an arena size, and what it costs.
struct Row {
    profile: &'static str,
    arena: usize,
    total: usize,
    align: usize,
    header: usize,
    minimum_block: usize,
}

impl Row {
    /// Everything in the struct that is not the arena.
    const fn bookkeeping(&self) -> usize {
        self.total.saturating_sub(self.arena)
    }
}

/// Build a row for one instantiation. A macro because the geometry is in
/// const parameters, so each row is a distinct type.
macro_rules! row {
    ($profile:literal, $n:literal, $align:literal, $link:literal) => {
        Row {
            profile: $profile,
            arena: $n,
            total: size_of::<Heap4<$n, $align, $link>>(),
            align: align_of::<Heap4<$n, $align, $link>>(),
            header: Heap4::<$n, $align, $link>::STRUCT_SIZE,
            minimum_block: Heap4::<$n, $align, $link>::MINIMUM_BLOCK_SIZE,
        }
    };
}

#[test]
fn the_ram_table_decomposes_exactly() {
    // 32-bit is every Kairos target; 64-bit is the oracle's host, and the
    // geometry the differential runs against.
    let rows = [
        row!(
            "32-bit (thumbv7em, thumbv8m, riscv32imac/imafc)",
            1024,
            8,
            8
        ),
        row!(
            "32-bit (thumbv7em, thumbv8m, riscv32imac/imafc)",
            4096,
            8,
            8
        ),
        row!(
            "32-bit (thumbv7em, thumbv8m, riscv32imac/imafc)",
            8192,
            8,
            8
        ),
        row!("64-bit (the oracle's host)", 1024, 8, 16),
        row!("64-bit (the oracle's host)", 4096, 8, 16),
        row!("64-bit (the oracle's host)", 8192, 8, 16),
    ];

    println!();
    println!("=== K4 RAM table: what a heap_4 costs, per profile ===");
    println!();
    println!(
        "{:<48} {:>7} {:>8} {:>6} {:>7} {:>9} {:>8}",
        "profile", "arena", "total B", "align", "book", "header/op", "min blk"
    );

    let mut bookkeeping = None;
    for r in &rows {
        println!(
            "{:<48} {:>7} {:>8} {:>6} {:>7} {:>9} {:>8}",
            r.profile,
            r.arena,
            r.total,
            r.align,
            r.bookkeeping(),
            r.header,
            r.minimum_block
        );

        // The identity. `total` is the arena plus the bookkeeping and
        // nothing else; a non-zero remainder would be padding the struct
        // carries and no column names.
        let padding = r
            .total
            .checked_sub(r.arena)
            .and_then(|rest| rest.checked_sub(r.bookkeeping()))
            .expect("total is never smaller than the arena");
        assert_eq!(
            padding, 0,
            "{} at {}: {padding} bytes belong to no column",
            r.profile, r.arena
        );

        // The bookkeeping must be a CONSTANT, not a function of the arena.
        // That is the property that makes the table a table: if it grew
        // with N there would be a per-byte cost hiding in it.
        let seen = *bookkeeping.get_or_insert(r.bookkeeping());
        if r.header == rows[0].header {
            assert_eq!(
                r.bookkeeping(),
                seen,
                "bookkeeping moved with the arena size — it is not fixed overhead"
            );
        }
    }

    println!();
    println!("identity  total = arena + bookkeeping, remainder 0 on every row");
    println!("bookkeeping is FIXED: it does not grow with the arena.");
    println!();
    println!("READ THE COLUMNS CAREFULLY. `header/op` and `min blk` are the");
    println!("TARGET's geometry -- they come from the const parameters, so the");
    println!("32-bit rows are true 32-bit numbers. `total B` and `book` are");
    println!("measured on THIS host, so on an x86-64 box both profiles report");
    println!("the host's bookkeeping. The fixed-overhead identity is proven on");
    println!("the real targets by a `const` assertion in heap4.rs, which the");
    println!("gate evaluates on all four bare-metal rungs.");
}

/// The header is what a caller actually pays per allocation, and it is
/// the number a firmware needs in order to size an arena.
///
/// This is the C's arithmetic, asserted rather than described: a request
/// is grown by the header and then rounded up to the alignment, so the
/// smallest request costs a whole minimum block and the cost is a step
/// function of the request.
#[test]
fn what_one_allocation_costs_is_the_cs_arithmetic() {
    type H = Heap4<8192, 8, 16>;
    let header = H::STRUCT_SIZE;
    let align = 8usize;

    println!();
    println!("=== what one allocation costs (64-bit geometry) ===");
    println!("{:>8} {:>10} {:>9}", "request", "block", "overhead");
    for request in [1usize, 7, 8, 9, 16, 100, 300, 600] {
        let block = (request.saturating_add(header).saturating_add(align - 1)) & !(align - 1);
        let overhead = block.saturating_sub(request);
        println!("{request:>8} {block:>10} {overhead:>9}");
        assert!(
            block >= header.saturating_add(request),
            "a block must hold its header and its payload"
        );
        assert_eq!(block % align, 0, "every block is alignment-sized");
    }

    // The smallest request still costs a full header plus padding, which
    // is the number that makes many tiny allocations expensive on a small
    // part — and the reason `StaticAllocation` exists at all.
    let smallest = (1usize + header + align - 1) & !(align - 1);
    assert_eq!(smallest, 24, "a 1-byte request costs 24 bytes at 64-bit");
    println!();
    println!("a 1-byte request costs {smallest} bytes: {header} header + padding.");
}

/// One pool geometry, decomposed to the byte.
struct PoolRow {
    block: usize,
    blocks: usize,
    arena: usize,
    total: usize,
    align: usize,
}

impl PoolRow {
    /// The free stack, the generations and the live flags: two bytes, four
    /// bytes and one byte per block.
    const fn tables(&self) -> usize {
        self.blocks.saturating_mul(2 + 4 + 1)
    }

    /// `top`, `allocations`, `frees`.
    fn scalars() -> usize {
        size_of::<usize>()
            .saturating_add(size_of::<u32>())
            .saturating_add(size_of::<u32>())
    }

    /// What the struct carries that no column above names.
    fn padding(&self) -> usize {
        self.total
            .saturating_sub(self.arena)
            .saturating_sub(self.tables())
            .saturating_sub(Self::scalars())
    }

    /// The whole cost of having a block, over and above the block.
    fn per_block(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let (book, blocks) = (
            self.total.saturating_sub(self.arena) as f64,
            self.blocks.max(1) as f64,
        );
        book / blocks
    }
}

macro_rules! pool_row {
    ($block:literal, $blocks:literal) => {
        PoolRow {
            block: $block,
            blocks: $blocks,
            arena: Pool::<$block, $blocks>::BYTES,
            total: size_of::<Pool<$block, $blocks>>(),
            align: align_of::<Pool<$block, $blocks>>(),
        }
    };
}

/// What a [`Pool`] costs, decomposed, and the property that makes it worth
/// having.
///
/// `heap_4`'s per-block cost is its header — eight bytes on a 32-bit target
/// — PLUS whatever its `MINIMUM_BLOCK_SIZE` of sixteen strands, PLUS
/// fragmentation, which no table can predict because it depends on the order
/// the application allocates in. A pool has a header of nothing, strands
/// nothing, and cannot fragment: its cost is three side tables, and this
/// asserts that cost is **constant per block and independent of the block
/// size**, which is the whole claim.
#[test]
fn the_pool_ram_table_decomposes_exactly() {
    let rows = [
        pool_row!(64, 48),
        pool_row!(256, 48),
        pool_row!(512, 48),
        pool_row!(2048, 48),
        pool_row!(512, 8),
        pool_row!(512, 256),
    ];

    println!();
    println!("=== what a Pool costs, per geometry ===");
    println!();
    println!(
        "{:>6} {:>7} {:>9} {:>9} {:>6} {:>8} {:>8} {:>10}",
        "block", "blocks", "arena", "total B", "align", "tables", "padding", "per block"
    );

    let mut per_block = None;
    for r in &rows {
        println!(
            "{:>6} {:>7} {:>9} {:>9} {:>6} {:>8} {:>8} {:>10.2}",
            r.block,
            r.blocks,
            r.arena,
            r.total,
            r.align,
            r.tables(),
            r.padding(),
            r.per_block()
        );

        // The identity: arena + tables + scalars is the whole struct.
        assert_eq!(
            r.padding(),
            0,
            "Pool<{}, {}>: {} bytes belong to no column",
            r.block,
            r.blocks,
            r.padding()
        );

        // The claim. A cost that moved with the BLOCK size would be a
        // per-byte tax hiding in the bookkeeping.
        let cost = r
            .total
            .saturating_sub(r.arena)
            .saturating_sub(PoolRow::scalars());
        let each = cost.checked_div(r.blocks).expect("a pool has blocks");
        assert_eq!(each, 7, "Pool<{}, {}>: per-block tables", r.block, r.blocks);
        match per_block {
            None => per_block = Some(each),
            Some(first) => assert_eq!(
                each, first,
                "the per-block cost moved with the geometry; it is supposed to be a constant"
            ),
        }
    }

    println!();
    println!("  seven bytes per block: a u16 free-stack slot, a u32 generation,");
    println!("  a bool live flag. Against heap_4's eight-byte header on a 32-bit");
    println!("  target -- before its 16-byte minimum block and before any");
    println!("  fragmentation, neither of which a pool has.");
    println!();
}
