//! `Heap1` against `heap_1.c`, operation for operation.
//!
//! The C arm is `oracle/heap1_driver.c` driving `heap_1.c` compiled VERBATIM
//! from the pinned kernel, and its trace is checked in — so this runs in CI
//! with no C toolchain, the same arrangement `heap4_differential.rs` uses.
//!
//! # Why this workload is not the heap_4 workload
//!
//! heap_4's exercises fragmentation: allocate, free, coalesce, refuse when no
//! free block is big enough. **heap_1 cannot free**, so every one of those
//! branches is absent and a workload leaning on them would prove nothing. The
//! only interesting branch a bump allocator has is REFUSAL, so this one runs
//! the arena to exhaustion and keeps asking: 1,973 of its 2,000 operations are
//! refused.
//!
//! That is the heap_4 lesson applied rather than relearned. Its first workload
//! never refused a single request, so what agreed was the easy half of an
//! allocator; `the_workload_reaches_the_branches_that_matter` exists there to
//! fail when that is true. Here refusal IS the hard half, so the guard is
//! inverted: this one checks that the arena was actually exhausted.

// A test asserts; the workspace's deny-by-default is written for library code
// where a panic is a defect.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusty_rtos_core::error::Error;
use rusty_rtos_heap_core::Heap1;

/// The C driver's arena and alignment.
const TOTAL: usize = 8192;
const ALIGN: usize = 8;

/// The C driver's workload shape.
const MAX_SIZE: u32 = 600;
const OPS: u32 = 2000;

type Heap = Heap1<TOTAL, ALIGN>;

/// The C driver's LCG, written out identically. `rand()` is
/// implementation-defined and would make the two arms incomparable by
/// construction.
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
fn our_heap1_matches_the_c_kernels_operation_for_operation() {
    let oracle = include_str!("../../../oracle/heap1.trace");
    let mut lines = oracle.lines();

    // The geometry line is a premise, not decoration: if the C is modelling a
    // different heap then every later comparison is meaningless. `adjusted` is
    // the quantity heap_1 bounds against, and it is TOTAL - ALIGN rather than
    // TOTAL — the C subtracts a whole alignment because it aligns ucHeap's
    // start at run time.
    let geometry = lines.next().expect("the trace has a geometry line");
    assert_eq!(
        geometry,
        format!(
            "geometry total={TOTAL} align={ALIGN} adjusted={}",
            Heap::ADJUSTED_SIZE
        ),
        "the two arms are not modelling the same heap"
    );

    let mut heap = Heap::new();
    let mut rng = Lcg::new();
    let mut refusals = 0u32;

    for op in 0..OPS {
        let r = rng.next();
        let size = 1 + (r % MAX_SIZE);

        let got = heap.alloc(size as usize);
        if got.is_none() {
            refusals = refusals.saturating_add(1);
        }
        let offset = got.map_or(-1i64, |o| i64::try_from(o).unwrap_or(-1));

        let ours = format!("alloc {size} {offset} {}", heap.free_bytes());
        let theirs = lines
            .next()
            .unwrap_or_else(|| panic!("the C trace ran out at operation {op}; ours said {ours:?}"));
        assert_eq!(
            ours, theirs,
            "operation {op} diverged — ours {ours:?}, the C {theirs:?}"
        );
    }

    let ours = format!("end refusals={refusals} free={}", heap.free_bytes());
    let theirs = lines.next().expect("the trace has an end line");
    assert_eq!(ours, theirs, "the arms ended in different states");
    assert!(lines.next().is_none(), "the C trace has lines left over");
}

/// The guard, inverted from heap_4's.
///
/// There, the danger was a workload that never refused — agreement about the
/// easy half of an allocator. Here refusal is the only branch worth having, so
/// the danger is a workload that never REACHES exhaustion and therefore only
/// ever exercises the bump.
///
/// Both are the same test in different clothes: a differential whose workload
/// cannot fail is a differential about nothing.
#[test]
fn the_workload_actually_exhausts_the_arena() {
    let mut heap = Heap::new();
    let mut rng = Lcg::new();
    let (mut refusals, mut successes) = (0u32, 0u32);

    for _ in 0..OPS {
        let size = 1 + (rng.next() % MAX_SIZE);
        if heap.alloc(size as usize).is_some() {
            successes = successes.saturating_add(1);
        } else {
            refusals = refusals.saturating_add(1);
        }
    }

    assert!(
        successes > 8,
        "the arena filled too fast to exercise the bump at all: {successes} succeeded"
    );
    assert!(
        refusals > OPS / 2,
        "the arena was never exhausted, so the only interesting branch never ran: \
         {refusals} refusals of {OPS}"
    );
    assert_eq!(
        heap.free_bytes(),
        8,
        "what is left is the tail the STRICTLY-less-than bound can never hand out"
    );
}

/// Freeing is invalid in this scheme, and says so.
///
/// `heap_1.c`'s `vPortFree` is `configASSERT( pv == NULL )` under the comment
/// "Force an assert as it is invalid to call this function". The first draft
/// of this port returned `Ok(())`, which would have let a caller do silently
/// what the C stops them doing loudly — and the first draft of the C driver
/// called it, which is how the C told us: it exited 2 before printing a single
/// operation.
#[test]
fn freeing_is_refused_because_the_c_calls_it_invalid() {
    let mut heap = Heap::new();
    let offset = heap.alloc(64).expect("room for 64 bytes");

    assert_eq!(heap.free(offset), Err(Error::Unsupported));
    assert_eq!(
        heap.free(0),
        Err(Error::Unsupported),
        "there is no offset for which freeing is valid"
    );

    // And it really did not free: the bump index has not moved back.
    let before = heap.free_bytes();
    let _ = heap.free(offset);
    assert_eq!(heap.free_bytes(), before);
}

/// The strictly-less-than bound, on its own.
///
/// `( xNextFreeByte + xWantedSize ) < configADJUSTED_HEAP_SIZE` — so a request
/// for exactly the bytes remaining is REFUSED. Writing `<=` would accept it,
/// and the differential would then diverge by one allocation somewhere near
/// exhaustion, which is a long way to travel to find a one-character mistake.
#[test]
fn a_request_for_exactly_the_remaining_bytes_is_refused() {
    let mut heap = Heap::new();
    let all = Heap::ADJUSTED_SIZE;

    assert!(
        heap.alloc(all).is_none(),
        "the whole adjusted arena must not fit: the bound is strict"
    );
    assert!(
        heap.alloc(all.saturating_sub(ALIGN)).is_some(),
        "one alignment less must fit"
    );
}

/// The request is rounded, not the offset.
///
/// `heap_1.c` rounds `xWantedSize` up to the alignment and bumps by the
/// rounded figure. So a one-byte request consumes a whole alignment, and the
/// next offset is `ALIGN`, not 1.
#[test]
fn the_request_is_what_gets_rounded() {
    let mut heap = Heap::new();

    assert_eq!(heap.alloc(1), Some(0));
    assert_eq!(
        heap.alloc(1),
        Some(ALIGN as u64),
        "a 1-byte request consumed a whole alignment"
    );
    assert_eq!(heap.alloc(ALIGN), Some(2 * ALIGN as u64));
}
