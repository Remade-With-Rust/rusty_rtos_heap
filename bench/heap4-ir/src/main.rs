//! The differential's workload, run for counting rather than comparing.
//!
//! Exactly the sequence `heap4_differential` replays, so an instruction
//! count taken here is a count of the work the gated path does — and any
//! change that moves it has to leave those 20,000 operations identical.

use rusty_rtos_heap_core::Heap4;

const TOTAL: usize = 8192;
const SLOTS: usize = 48;
const MAX_SIZE: u32 = 600;
const OPS: u32 = 20_000;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

fn main() {
    let mut heap: Heap4<TOTAL, 8, 16> = Heap4::new();
    let mut slots: [Option<u64>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg(12345);
    let mut checksum = 0u64;

    for _ in 0..OPS {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;
        let held = slots.get(slot).copied().flatten();
        if let Some(offset) = held {
            heap.free(offset);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
        } else {
            let size = 1 + ((r / SLOTS as u32) % MAX_SIZE);
            let got = heap.alloc(size as usize);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
        }
        // A checksum so a compiler that removed the work is visible as a
        // changed number rather than as a suspiciously good count.
        checksum = checksum
            .wrapping_add(heap.free_bytes() as u64)
            .wrapping_add(heap.minimum_ever_free_bytes() as u64);
    }

    println!("checksum {checksum}");
    println!("allocations {} frees {}", heap.allocations(), heap.frees());
}
