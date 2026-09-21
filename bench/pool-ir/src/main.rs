//! The same workload as `heap4-ir`, served by a fixed-size [`Pool`].
//!
//! Work parity is the point: the identical LCG, the identical slot pattern,
//! the identical 20,000 operations, the identical alternation of take and
//! give back. The only difference is which allocator answers — which is
//! what makes the two instruction counts a comparison rather than two
//! numbers.
//!
//! The band is the one the ask names. `heap4-ir` restricted to `256..=512`
//! measures **133.04 Ir/op**; every allocation there succeeds, so it is not
//! measuring a failing allocator. This serves the same pattern with the size
//! fixed at 512 — the top of that band, and the worst case for a pool, since
//! a pool pays for the size it was declared with whatever is asked.
//!
//! What a pool cannot do is also the point, and is not hidden: one size, a
//! capacity fixed at compile time, no coalescing and no borrowing from a
//! neighbour. It answers a narrower question, which is why it can answer it
//! in fewer instructions.

use rusty_rtos_heap_core::pool::{Pool, Slot};

/// As `heap4-ir`: 48 slots the workload cycles through.
const SLOTS: usize = 48;
/// The band's top. A pool is sized for its largest block, so this is the
/// unflattering end to measure at.
const BLOCK: usize = 512;
/// Enough blocks that the pattern never starves, as the 64 KiB arena the
/// band-restricted `heap4-ir` run used never starved.
const BLOCKS: usize = SLOTS;
const OPS: u32 = 20_000;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

fn main() {
    let mut pool: Pool<BLOCK, BLOCKS> = Pool::new();
    let mut slots: [Option<Slot>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg(12345);
    let mut checksum = 0u64;

    for _ in 0..OPS {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;
        let held = slots.get(slot).copied().flatten();
        if let Some(block) = held {
            let _ = pool.free(block);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
        } else {
            let got = pool.alloc();
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
        }
        // As `heap4-ir`: a checksum so a compiler that removed the work
        // shows up as a changed number rather than a good one.
        checksum = checksum
            .wrapping_add(pool.available() as u64)
            .wrapping_add(u64::from(pool.allocations()));
    }

    println!("checksum {checksum}");
    println!(
        "allocations {} frees {} available {}",
        pool.allocations(),
        pool.frees(),
        pool.available()
    );
}
