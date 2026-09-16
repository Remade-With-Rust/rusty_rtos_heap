//! `Heap5` against `heap_5.c`, operation for operation.
//!
//! The C arm is `oracle/heap5_driver.c` driving `heap_5.c` compiled VERBATIM
//! from the pinned kernel, and its trace is checked in — so this runs in CI
//! with no C toolchain, the same arrangement the other two use.
//!
//! # What this adds over the heap_4 differential
//!
//! heap_5 IS heap_4 with a different initialiser: the allocate, free and
//! insert paths are the same code in both C files, and one implementation in
//! ours. So the workload is heap_4's, and the one thing it adds is the one
//! thing heap_5 has — **three regions with gaps between them**.
//!
//! The gaps are the whole point. Coalescing is an address comparison, so what
//! stops two regions merging into one is that the first one's end is not the
//! second one's start. A transcription that quietly treated the arena as
//! contiguous would hand out a block spanning a gap, and the offsets are what
//! catch it.

// A test asserts; the workspace's deny-by-default is written for library code
// where a panic is a defect.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusty_rtos_core::error::Error;
use rusty_rtos_heap_core::heap4::Block;
use rusty_rtos_heap_core::{Heap5, Region};

/// The C driver's arena and geometry.
const ARENA: usize = 12288;
const ALIGN: usize = 8;
const LINK: usize = 16;

/// The C driver's regions: different sizes, real gaps, increasing addresses.
const R0: Region = Region::new(0, 4096);
const R1: Region = Region::new(4608, 2048);
const R2: Region = Region::new(7168, 4096);

/// The C driver's workload shape.
const SLOTS: usize = 48;
const MAX_SIZE: u32 = 600;
const OPS: u32 = 20_000;

type Heap = Heap5<ARENA, ALIGN, LINK>;

/// The C driver's LCG, written out identically.
struct Lcg(u32);

impl Lcg {
    const fn new() -> Self {
        Self(12345)
    }

    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

#[test]
fn our_heap5_matches_the_c_kernels_operation_for_operation() {
    let oracle = include_str!("../../../oracle/heap5.trace");
    let mut lines = oracle.lines();

    // The geometry and the regions are premises, not decoration: if the C is
    // modelling a different heap then every later comparison is meaningless.
    assert_eq!(
        lines.next().expect("a geometry line"),
        format!(
            "geometry arena={ARENA} align={ALIGN} struct={} regions=3",
            LINK
        ),
        "the two arms are not modelling the same heap"
    );
    for (n, region) in [R0, R1, R2].iter().enumerate() {
        assert_eq!(
            lines.next().expect("a region line"),
            format!("region {n} {} {}", region.start, region.size),
            "region {n} differs between the arms"
        );
    }

    let mut heap = Heap::new(&[R0, R1, R2]).expect("three regions, in order");

    assert_eq!(
        lines.next().expect("the defined line"),
        format!(
            "defined {} {}",
            heap.free_bytes(),
            heap.minimum_ever_free_bytes()
        ),
        "the regions did not add up to the same free space"
    );

    let mut slots: [Option<Block>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg::new();

    for op in 0..OPS {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;

        // `slot` is `r % SLOTS` so it is in range by construction — but the
        // workspace forbids indexing that may panic, and a differential that
        // panics instead of reporting a mismatch is a worse instrument.
        let held = slots.get(slot).copied().flatten();
        let ours = if let Some(block) = held {
            heap.free(block).expect("a live handle must free");
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
            format!(
                "free {slot} -1 {} {}",
                heap.free_bytes(),
                heap.minimum_ever_free_bytes()
            )
        } else {
            let size = 1 + ((r / SLOTS as u32) % MAX_SIZE);
            let got = heap.alloc(size as usize);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
            let offset = got.map_or(-1i64, |b| i64::try_from(b.offset()).unwrap_or(-1));
            format!(
                "alloc {slot} {size} {offset} {} {}",
                heap.free_bytes(),
                heap.minimum_ever_free_bytes()
            )
        };

        let theirs = lines
            .next()
            .unwrap_or_else(|| panic!("the C trace ran out at operation {op}; ours {ours:?}"));
        assert_eq!(
            ours, theirs,
            "operation {op} diverged — ours {ours:?}, the C {theirs:?}"
        );
    }

    assert_eq!(
        lines.next().expect("an end line"),
        format!(
            "end {} {}",
            heap.free_bytes(),
            heap.minimum_ever_free_bytes()
        ),
        "the arms ended in different states"
    );
    assert!(lines.next().is_none(), "the C trace has lines left over");
}

/// The guard: every region must actually be used.
///
/// heap_4's guard fails when the workload never refuses; heap_1's fails when
/// the arena is never exhausted. This one fails when the workload never
/// reaches a region, because a three-region differential that only ever
/// allocates in the first one is a one-region differential wearing a costume.
#[test]
fn the_workload_reaches_every_region() {
    let mut heap = Heap::new(&[R0, R1, R2]).expect("three regions");
    let mut slots: [Option<Block>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg::new();
    let mut hits = [0u32; 3];

    for _ in 0..OPS {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;
        let held = slots.get(slot).copied().flatten();
        if let Some(block) = held {
            heap.free(block).expect("a live handle frees");
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
        } else {
            let size = 1 + ((r / SLOTS as u32) % MAX_SIZE);
            let got = heap.alloc(size as usize);
            if let Some(block) = got {
                let at = block.offset();
                let which = if at < R1.start as u64 {
                    0
                } else if at < R2.start as u64 {
                    1
                } else {
                    2
                };
                if let Some(count) = hits.get_mut(which) {
                    *count = count.saturating_add(1);
                }
            }
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
        }
    }

    for (n, count) in hits.iter().enumerate() {
        assert!(
            *count > 100,
            "region {n} was allocated in only {count} times — this is not a \
             three-region test"
        );
    }
}

/// **No allocation may span a gap.**
///
/// The regions are 0..4096, 4608..6656 and 7168..11264, so the two gaps are
/// 4096..4608 and 6656..7168. An allocation that started before a gap and ran
/// past it would be handing out memory that belongs to no region — the exact
/// failure a contiguous-arena transcription produces.
#[test]
fn no_allocation_crosses_a_gap() {
    let mut heap = Heap::new(&[R0, R1, R2]).expect("three regions");
    let mut slots: [Option<Block>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg::new();

    let gaps = [
        (R0.start + R0.size, R1.start),
        (R1.start + R1.size, R2.start),
    ];

    for _ in 0..OPS {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;
        let held = slots.get(slot).copied().flatten();
        if let Some(block) = held {
            heap.free(block).expect("a live handle frees");
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
        } else {
            let size = 1 + ((r / SLOTS as u32) % MAX_SIZE);
            let got = heap.alloc(size as usize);
            if let Some(block) = got {
                let start = block.offset() as usize;
                let end = start.saturating_add(size as usize);
                for (gap_start, gap_end) in gaps {
                    assert!(
                        end <= gap_start || start >= gap_end,
                        "an allocation at {start}..{end} crosses the gap \
                         {gap_start}..{gap_end}"
                    );
                }
            }
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
        }
    }
}

/// Regions out of address order are REFUSED, because the C asserts that order
/// rather than sorting.
///
/// ```c
/// /* Check blocks are passed in with increasing start addresses. */
/// configASSERT( ( size_t ) xAddress > ( size_t ) pxEnd );
/// ```
///
/// The plan for this work assumed the opposite — that the C sorted them, and
/// that a transcription expecting sorted input would be the bug. Reading the
/// source settled it the other way, which is why the oracle is pinned and
/// read rather than remembered.
#[test]
fn regions_out_of_address_order_are_refused() {
    assert_eq!(
        Heap::new(&[R2, R0, R1]).err(),
        Some(Error::InvalidArgument),
        "descending regions must be refused, not sorted"
    );
    assert_eq!(
        Heap::new(&[R0, R2, R1]).err(),
        Some(Error::InvalidArgument),
        "one out-of-order region is enough"
    );
    assert!(
        Heap::new(&[R0, R1, R2]).is_ok(),
        "and the ascending order is accepted"
    );
}

/// A heap must have regions, and they must fit.
#[test]
fn a_heap_needs_regions_that_fit() {
    assert_eq!(Heap::new(&[]).err(), Some(Error::InvalidArgument));
    assert_eq!(
        Heap::new(&[Region::new(0, ARENA + 8)]).err(),
        Some(Error::InvalidArgument),
        "a region larger than the arena"
    );
    assert_eq!(
        Heap::new(&[Region::new(0, LINK)]).err(),
        Some(Error::InvalidArgument),
        "a region with no room for a block AND its end marker"
    );
}

/// **Even abutting regions stay separate**, and the reason is the end marker.
///
/// This test was written to assert the opposite — that two regions sharing a
/// boundary coalesce, "because then they are one region" — and it failed. The
/// reason is worth keeping: every region reserves its own end marker at its
/// top, so region 0's free block ends at its marker and NOT at region 1's
/// start. The addresses do not meet, so the coalescing comparison is false,
/// and there is a `STRUCT_SIZE` marker sitting between them for ever.
///
/// So `vPortDefineHeapRegions` is not a way to describe one arena in pieces:
/// each piece costs a marker and none of them merge. A caller wanting one big
/// region should pass one big region.
#[test]
fn even_abutting_regions_do_not_merge() {
    let half = 2048;
    let mut split =
        Heap5::<4096, ALIGN, LINK>::new(&[Region::new(0, half), Region::new(half, half)])
            .expect("two abutting regions");
    let mut whole =
        Heap5::<4096, ALIGN, LINK>::new(&[Region::new(0, 2 * half)]).expect("one region");

    // Measure BEFORE allocating from either: the pair pays for one more end
    // marker, and comparing after would be comparing one heap that served a
    // request against one that refused it.
    assert!(
        split.free_bytes() < whole.free_bytes(),
        "two regions cost one more end marker than one region of the same size"
    );

    // A request that only the whole arena can serve.
    let big = half + 256;
    assert!(
        split.alloc(big).is_none(),
        "abutting regions must NOT merge: each keeps its own end marker, so          the blocks never meet"
    );
    assert!(
        whole.alloc(big).is_some(),
        "the same bytes as one region serve it"
    );
}
