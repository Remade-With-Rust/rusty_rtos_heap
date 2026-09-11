//! K4's kill test: our `heap_4` against FreeRTOS's, operation for operation.
//!
//! The C arm is in `oracle/heap4.trace`, written by `oracle/run.sh` from
//! `heap_4.c` compiled **verbatim** out of the pinned kernel checkout. It
//! is checked in for the reason the kernel corpus checks its traces in:
//! the diff then runs anywhere, with no C toolchain, on every push.
//!
//! # Why offsets and not pointers
//!
//! A pointer is not comparable across two programs. The C driver sets
//! `configAPPLICATION_ALLOCATED_HEAP` so `ucHeap` is its own aligned
//! array, and prints every result as an offset into it. This side models
//! the arena as a byte array whose offset 0 is that same aligned base, so
//! the two are comparing the same quantity.
//!
//! # What agreement means
//!
//! For every one of 20,000 operations: the **offset** first fit chose, the
//! **free bytes** remaining, and the **minimum ever** free. Offsets catch
//! a different block being picked or split at a different point; free
//! bytes catch a block being sized wrong; minimum-ever catches a
//! transient the other two would have healed. A first-fit allocator that
//! coalesced differently diverges within a few hundred operations.
//!
//! This remake agreed on the first run, which is a weaker result than it
//! sounds and is recorded as such: the first workload never drove the
//! arena to exhaustion — 32 slots of at most 300 bytes leaves a steady
//! state of ~2.6 KB in 8 KB, and **not one request was refused**. Agreeing
//! about an allocator that never has to say no is agreeing about its easy
//! half. `the_workload_reaches_the_branches_that_matter` caught that, and
//! 48 slots of at most 600 bytes is what it takes: 2,087 refusals, 784
//! distinct offsets, minimum-ever free down to 1,232 of 8,192. The
//! agreement below is on THAT workload.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusty_rtos_heap_core::Heap4;

/// The C driver's arena, alignment and `sizeof( BlockLink_t )`.
const TOTAL: usize = 8192;
const ALIGN: usize = 8;
const LINK: usize = 16;

/// The C driver's workload shape.
const SLOTS: usize = 48;
const MAX_SIZE: u32 = 600;
const OPS: u32 = 20_000;

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
fn our_heap4_matches_the_c_kernels_operation_for_operation() {
    let oracle = include_str!("../../../oracle/heap4.trace");
    let mut lines = oracle.lines();

    // The geometry line is a premise, not decoration: if the C is
    // modelling a different heap, every later comparison is meaningless.
    let geometry = lines.next().expect("the trace has a geometry line");
    assert_eq!(
        geometry,
        format!(
            "geometry total={TOTAL} align={ALIGN} struct={} minblock={}",
            Heap4::<TOTAL, ALIGN, LINK>::STRUCT_SIZE,
            Heap4::<TOTAL, ALIGN, LINK>::MINIMUM_BLOCK_SIZE
        ),
        "the C arm and this one are not modelling the same heap"
    );

    let mut heap: Heap4<TOTAL, ALIGN, LINK> = Heap4::new();
    let mut slots: [Option<u64>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg::new();
    let mut compared = 0usize;

    for op in 0..OPS {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;

        // `slot` is `r % SLOTS`, so it is in range by construction — but
        // the workspace forbids indexing that may panic, and a differential
        // that panics instead of reporting a mismatch is a worse instrument.
        let held = slots.get(slot).copied().flatten();
        let ours = if let Some(offset) = held {
            heap.free(offset);
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
            // The C stores the result even when it is NULL, so a refused
            // request leaves the slot empty and the next visit allocates
            // again. Anything else would desynchronise the sequences.
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
            let offset = got.map_or(-1i64, |o| o as i64);
            format!(
                "alloc {slot} {size} {offset} {} {}",
                heap.free_bytes(),
                heap.minimum_ever_free_bytes()
            )
        };

        let theirs = lines
            .next()
            .unwrap_or_else(|| panic!("the C trace ended at operation {op}"));
        assert_eq!(
            ours, theirs,
            "operation {op}: ours is left, the C kernel's is right"
        );
        compared += 1;
    }

    let theirs = lines.next().expect("the trace has an end line");
    assert_eq!(
        format!(
            "end free={} minfree={}",
            heap.free_bytes(),
            heap.minimum_ever_free_bytes()
        ),
        theirs
    );
    assert_eq!(compared, OPS as usize);
}

/// The workload has to reach the branches, or agreeing proves nothing.
///
/// A differential that never fragments the arena, never refuses a request
/// and never splits a block would agree with almost any allocator. This
/// asserts the C trace exercised all three, so the agreement above is
/// worth having.
#[test]
fn the_workload_reaches_the_branches_that_matter() {
    let oracle = include_str!("../../../oracle/heap4.trace");

    let mut refusals = 0usize;
    let mut offsets = std::collections::BTreeSet::new();
    let mut min_free = usize::MAX;

    for line in oracle.lines().skip(1) {
        let f: Vec<&str> = line.split(' ').collect();
        if f.first() == Some(&"alloc") {
            let Some(&offset) = f.get(3) else { continue };
            if offset == "-1" {
                refusals += 1;
            } else {
                offsets.insert(offset.to_owned());
            }
            if let Some(&free) = f.get(5) {
                min_free = min_free.min(free.parse::<usize>().unwrap_or(usize::MAX));
            }
        }
    }

    assert!(
        refusals > 100,
        "the arena was never driven to exhaustion: {refusals} refusals"
    );
    assert!(
        offsets.len() > 100,
        "the arena never fragmented: {} distinct offsets",
        offsets.len()
    );
    assert!(
        min_free < TOTAL / 4,
        "the arena was never filled: minimum free was {min_free} of {TOTAL}"
    );
}
