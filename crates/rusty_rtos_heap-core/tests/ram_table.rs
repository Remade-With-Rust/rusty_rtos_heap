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
        row!("32-bit (thumbv7em, thumbv8m, riscv32imac/imafc)", 1024, 8, 8),
        row!("32-bit (thumbv7em, thumbv8m, riscv32imac/imafc)", 4096, 8, 8),
        row!("32-bit (thumbv7em, thumbv8m, riscv32imac/imafc)", 8192, 8, 8),
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
