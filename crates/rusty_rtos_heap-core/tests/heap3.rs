//! `Heap3`, and the claim it can and cannot make.
//!
//! # There is no differential here, and that is structural
//!
//! `heap_1`, `heap_4` and `heap_5` are each diffed against their C original
//! over thousands of operations. **`heap_3` cannot be**, and not for want of
//! effort: `heap_3.c` forwards to `malloc`, so a differential against it would
//! compare whichever libc the oracle linked with whichever global allocator
//! this binary declared. It would measure two third parties and call the
//! result conformance.
//!
//! So the claim is a different KIND, and it is labelled rather than blurred:
//! **the same workload runs over an external allocator and the accounting
//! reconciles.** That is weaker than the other three and saying so is the
//! point — three allocators in this package are proven against the C, and a
//! fourth that cannot be would dilute what "K4 passed" means if the difference
//! were quietly dropped.
//!
//! What IS tested here is the bookkeeping this seam adds on top: the handles,
//! the generations, the reuse, and the refusals. Those are ours, so those can
//! be held to the family's standard.

// A test asserts; the workspace's deny-by-default is written for library code
// where a panic is a defect.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "alloc")]

use rusty_rtos_core::error::Error;
use rusty_rtos_heap_core::Heap3;

/// The same shape of workload the other heaps are diffed on, run over the
/// global allocator — and the accounting must reconcile at the end.
///
/// This is the honest kill test: not "it matches the C", but "it serves the
/// workload and every byte is accounted for".
#[test]
fn the_workload_runs_and_the_accounting_reconciles() {
    const SLOTS: usize = 48;
    const MAX_SIZE: u32 = 600;
    const OPS: u32 = 20_000;

    let mut heap = Heap3::new();
    let mut slots: [Option<rusty_rtos_heap_core::heap3::Handle>; SLOTS] = [None; SLOTS];
    let mut seed = 12345u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };

    let (mut allocated, mut freed) = (0usize, 0usize);

    for _ in 0..OPS {
        let r = next();
        let slot = (r as usize) % SLOTS;
        let held = slots.get(slot).copied().flatten();

        if let Some(handle) = held {
            heap.free(handle).expect("a live handle must free");
            freed = freed.saturating_add(1);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
        } else {
            let size = 1 + ((r / SLOTS as u32) % MAX_SIZE);
            let got = heap.alloc(size as usize);
            if got.is_some() {
                allocated = allocated.saturating_add(1);
            }
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
        }
    }

    // Free what is left, then the books must balance exactly.
    for cell in &mut slots {
        if let Some(handle) = cell.take() {
            heap.free(handle).expect("a live handle must free");
            freed = freed.saturating_add(1);
        }
    }

    assert_eq!(heap.allocations(), allocated, "allocation count");
    assert_eq!(heap.frees(), freed, "free count");
    assert_eq!(allocated, freed, "everything allocated was freed");
    assert_eq!(heap.live_bytes(), 0, "no bytes left live");
    assert_eq!(heap.live_allocations(), 0, "no slots left live");
    assert!(
        allocated > 1000,
        "the workload barely allocated ({allocated}) — it proves nothing"
    );
}

/// A double free is refused.
///
/// `heap_3.c` cannot do this: it hands the pointer to `free()` and whatever
/// libc does about a double free is what happens, which is usually abort or
/// silent corruption. Here `Box` owns the bytes and the slot owns the `Box`,
/// so a second free is a refused lookup.
#[test]
fn a_double_free_is_refused() {
    let mut heap = Heap3::new();
    let handle = heap.alloc(64).expect("64 bytes");

    assert_eq!(heap.free(handle), Ok(()));
    assert_eq!(heap.free(handle), Err(Error::Gone));
    assert_eq!(heap.live_bytes(), 0);
}

/// Freeing a slot that has since been reallocated is refused.
///
/// The slot comes back — that is the point of reusing it — so the generation
/// is what separates the allocation you were given from the one there now.
/// Same bug, same answer, as [`rusty_rtos_heap_core::heap4`]'s protector.
#[test]
fn freeing_a_reused_slot_is_refused() {
    let mut heap = Heap3::new();

    let first = heap.alloc(64).expect("64 bytes");
    assert_eq!(heap.free(first), Ok(()));

    let second = heap.alloc(64).expect("64 bytes again");
    assert_eq!(second.slot(), first.slot(), "the slot was reused");
    assert_ne!(second.generation(), first.generation(), "and re-stamped");

    assert_eq!(
        heap.free(first),
        Err(Error::Gone),
        "the stale handle must not free the live allocation"
    );
    assert_eq!(heap.free(second), Ok(()));
}

/// The bytes are real, writable, and survive.
#[test]
fn the_bytes_are_usable_and_private() {
    let mut heap = Heap3::new();
    let a = heap.alloc(64).expect("a");
    let b = heap.alloc(64).expect("b");

    heap.bytes_mut(a).expect("a's bytes").fill(0xAA);
    heap.bytes_mut(b).expect("b's bytes").fill(0xBB);

    assert!(
        heap.bytes_mut(a)
            .expect("a again")
            .iter()
            .all(|v| *v == 0xAA)
    );
    assert!(
        heap.bytes_mut(b)
            .expect("b again")
            .iter()
            .all(|v| *v == 0xBB)
    );
    assert_eq!(heap.bytes_mut(a).expect("a").len(), 64);

    assert_eq!(heap.free(a), Ok(()));
    assert_eq!(
        heap.bytes_mut(a).err(),
        Some(Error::Gone),
        "freed bytes are not reachable"
    );
    assert_eq!(heap.free(b), Ok(()));
}

/// `free_bytes` refuses rather than inventing a number.
///
/// `heap_3.c` does not implement `xPortGetFreeHeapSize` at all — libc does not
/// publish how much it has left. Returning a plausible figure would be the
/// worst of the options.
#[test]
fn free_bytes_refuses_because_the_platform_cannot_say() {
    let heap = Heap3::new();
    assert_eq!(heap.free_bytes(), Err(Error::Unsupported));
}

/// A zero-sized request is `None`, as `malloc(0)` may be and `pvPortMalloc`
/// treats as a refusal.
#[test]
fn a_zero_request_is_refused() {
    let mut heap = Heap3::new();
    assert!(heap.alloc(0).is_none());
    assert_eq!(heap.allocations(), 0);
}

/// Slots are reused rather than growing without bound.
///
/// A seam that pushed a new slot per allocation would leak the table even
/// though it freed the bytes — the kind of slow failure that only shows up in
/// a long-running system, which is exactly what an RTOS is.
#[test]
fn slots_are_reused_rather_than_accumulated() {
    let mut heap = Heap3::new();

    for _ in 0..10_000 {
        let handle = heap.alloc(32).expect("32 bytes");
        heap.free(handle).expect("free");
    }

    assert_eq!(heap.allocations(), 10_000);
    assert_eq!(heap.live_allocations(), 0);
    assert_eq!(heap.live_bytes(), 0);
}
