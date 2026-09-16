//! What the protector refuses, using handles a caller could really hold.
//!
//! `heap_4.c`'s `configENABLE_HEAP_PROTECTOR` XORs free-list **pointers** with
//! a canary, because a corrupted pointer in C is an arbitrary-write primitive.
//! This heap holds bounds-checked `u64` offsets in a crate that is
//! `#![forbid(unsafe_code)]`, so that primitive does not exist and the canary
//! has nothing to protect. What remains is the failure the canary was never
//! aimed at: freeing a block that has since been reallocated.
//!
//! The cases needing a FORGED offset -- interior, out of range, never
//! allocated -- are unit tests inside `heap4.rs` instead. [`Block`] has no
//! public constructor, deliberately: inventing one is the bug the generation
//! exists to catch, so the tests that need to invent one live where the fields
//! are in scope rather than widening the real API to reach them.
//!
//! Every case here is a bug in the CALLER. None can corrupt memory; the
//! question each asks is whether the caller is told.

// A test asserts; the workspace's deny-by-default is written for library code
// where a panic is a defect. `heap4_differential.rs` opts out the same way.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusty_rtos_core::error::Error;
use rusty_rtos_heap_core::Heap4;

const TOTAL: usize = 8192;
const ALIGN: usize = 8;
const LINK: usize = 8;

type Heap = Heap4<TOTAL, ALIGN, LINK>;

/// A double free is refused, and named.
///
/// The C `configASSERT`s the allocated bit and then, in a release build, does
/// nothing. This returned `()` and did the same -- strictly worse than the C,
/// because the C at least stops in a debug build and a caller here could learn
/// nothing at all. A silent no-op on a double free erases its own evidence.
#[test]
fn a_double_free_is_refused() {
    let mut heap = Heap::new();
    let block = heap.alloc(64).expect("room for 64 bytes");

    assert_eq!(heap.free(block), Ok(()), "the first free is the good one");

    let free_bytes = heap.free_bytes();
    assert_eq!(
        heap.free(block),
        Err(Error::Gone),
        "the second free must be refused rather than ignored"
    );
    assert_eq!(
        heap.free_bytes(),
        free_bytes,
        "a refused free must not have changed the heap"
    );
}

/// **The one the allocated bit cannot see.**
///
/// Free a block, allocate again so the same offset comes back, then free the
/// STALE handle. The block is allocated, so the allocated bit says yes. Only
/// the generation distinguishes the allocation you were handed from the one
/// living there now, which is why the offset carries one.
#[test]
fn freeing_a_reallocated_block_is_refused() {
    let mut heap = Heap::new();

    let first = heap.alloc(64).expect("room for 64 bytes");
    let offset = first.offset();
    assert_eq!(heap.free(first), Ok(()));

    let second = heap.alloc(64).expect("the same room, again");
    assert_eq!(
        second.offset(),
        offset,
        "this test is only meaningful if the block came back at the same place"
    );
    assert_ne!(
        second.generation(),
        first.generation(),
        "and only if the generation moved"
    );

    assert_eq!(
        heap.free(first),
        Err(Error::Gone),
        "the stale handle must not free somebody else's allocation"
    );

    // The live one still frees, which is the half that shows the refusal was
    // targeted rather than the heap simply being broken.
    assert_eq!(heap.free(second), Ok(()));
}

/// The accounting a refused free must not touch.
///
/// A rejection that still decremented `free_bytes`, or still counted a free,
/// would be worse than no check at all: the heap would look healthy and be
/// wrong.
#[test]
fn a_refused_free_changes_nothing() {
    let mut heap = Heap::new();
    let block = heap.alloc(64).expect("room for 64 bytes");
    heap.free(block).expect("the good free");

    let before = (
        heap.free_bytes(),
        heap.minimum_ever_free_bytes(),
        heap.frees(),
    );
    for _ in 0..8 {
        assert_eq!(heap.free(block), Err(Error::Gone));
    }
    let after = (
        heap.free_bytes(),
        heap.minimum_ever_free_bytes(),
        heap.frees(),
    );

    assert_eq!(before, after, "eight refused frees must be eight no-ops");
}

/// Generations are per heap and monotonic, so two live blocks never share one.
#[test]
fn generations_do_not_repeat_while_blocks_are_live() {
    let mut heap = Heap::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut blocks = Vec::new();

    for _ in 0..16 {
        let b = heap.alloc(64).expect("room");
        assert!(seen.insert(b.generation()), "a generation was reused");
        blocks.push(b);
    }
    for b in blocks {
        assert_eq!(heap.free(b), Ok(()));
    }
}

/// The protector must not have changed what the allocator DOES.
///
/// The differential against `heap_4.c` is the real guard for this, but it runs
/// one workload; this is the cheap standing check that the ordinary path still
/// allocates, coalesces and returns the arena to its starting state.
#[test]
fn the_ordinary_path_is_unchanged() {
    let mut heap = Heap::new();

    // `prvHeapInit` runs on the FIRST allocation, exactly as it does in the C,
    // so `free_bytes()` before one is 0 rather than the arena size. Take the
    // baseline after forcing the init, or this compares against nothing.
    let warm = heap.alloc(16).expect("force prvHeapInit");
    heap.free(warm).expect("the warm-up frees");
    let empty = heap.free_bytes();

    let a = heap.alloc(64).expect("a");
    let b = heap.alloc(128).expect("b");
    let c = heap.alloc(256).expect("c");
    assert!(heap.free_bytes() < empty);

    // Free out of order, so the coalescing paths all run.
    assert_eq!(heap.free(b), Ok(()));
    assert_eq!(heap.free(a), Ok(()));
    assert_eq!(heap.free(c), Ok(()));

    assert_eq!(
        heap.free_bytes(),
        empty,
        "every byte came back, so the three blocks coalesced into one"
    );
}
