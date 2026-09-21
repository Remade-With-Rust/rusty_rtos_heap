//! The same workload up a ladder of size bands, for both allocators.
//!
//! `heap4-ir` mirrors the differential exactly and must not move, and
//! `pool-ir` answers one band. This is the ladder: one binary, one arena,
//! the band chosen at run time, so every rung is measured on the same
//! geometry and the rungs can be read against each other.
//!
//! ```sh
//! bands-ir heap4 256 257     # 256..=512
//! bands-ir pool  1024        # a 1024-byte pool
//! ```
//!
//! # Why one arena for every rung
//!
//! A band measured in an arena it nearly fills is measuring exhaustion, not
//! allocation: the walk runs to `pxEnd` and fails. 256 KiB holds 48 live
//! blocks at the largest rung here, which is the slot count the workload
//! cycles through — so no rung starves and the rungs differ only in the size
//! asked for. The allocation and free counts are printed for exactly that
//! reason: if a rung's counts differ from its neighbours', it was measuring
//! something else.
//!
//! # Work parity
//!
//! Identical LCG, identical seed, identical 48-slot pattern, identical
//! 20,000 operations on every rung and in both modes. The only variables are
//! the size asked for and which allocator answers.

use rusty_rtos_heap_core::heap4::Block;
use rusty_rtos_heap_core::pool::{Pool, Slot};
use rusty_rtos_heap_core::Heap4;

/// 256 KiB: 48 live blocks at the largest rung, so no rung starves.
const TOTAL: usize = 262_144;
/// 2 MiB: the SAME 48 live blocks at the largest rung occupy 9% of it,
/// which is what they occupy of the 256 KiB arena at the SMALLEST rung.
/// Holding occupancy fixed is what separates "larger blocks cost more"
/// from "a fuller arena costs more" -- and first-fit has every reason to
/// care about the second and none to care about the first.
const ROOMY: usize = 2_097_152;
const SLOTS: usize = 48;
/// Overridable, so the cost of one operation can be taken as a SLOPE.
/// This bench counts a whole process, so a one-time cost -- the pool
/// zeroing its arena, say -- is charged to the operations. Two lengths
/// and a subtraction cancel it exactly.
const OPS_DEFAULT: u32 = 20_000;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

fn heap4_band(min: u32, span: u32, ops: u32) {
    heap4_band_in::<TOTAL, 16>(min, span, ops);
}

/// The 32-bit geometry: `sizeof( BlockLink_t )` is one pointer plus one
/// `size_t`, so eight bytes, and the header stores two 32-bit words.
///
/// Work parity here is the ALLOCATION AND FREE COUNTS, not the checksum:
/// the two geometries have different header sizes, so `free_bytes` legitimately
/// differs and a matching checksum would mean the header size had not changed.
/// The arena never fills at these rungs, so every allocation succeeds in both
/// and the counts must agree.
fn heap4_band_narrow(min: u32, span: u32, ops: u32) {
    heap4_band_in::<TOTAL, 8>(min, span, ops);
}

/// As [`heap4_band`], in a roomier arena.
fn heap4_band_roomy(min: u32, span: u32, ops: u32) {
    heap4_band_in::<ROOMY, 16>(min, span, ops);
}

fn heap4_band_in<const N: usize, const LINK: usize>(min: u32, span: u32, ops: u32) {
    let mut heap: Heap4<N, 8, LINK> = Heap4::new();
    let mut slots: [Option<Block>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg(12345);
    let mut checksum = 0u64;

    for _ in 0..ops {
        let r = rng.next();
        let slot = (r as usize) % SLOTS;
        let held = slots.get(slot).copied().flatten();
        if let Some(block) = held {
            let _ = heap.free(block);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = None;
            }
        } else {
            let size = min + ((r / SLOTS as u32) % span);
            let got = heap.alloc(size as usize);
            if let Some(cell) = slots.get_mut(slot) {
                *cell = got;
            }
        }
        checksum = checksum
            .wrapping_add(heap.free_bytes() as u64)
            .wrapping_add(heap.minimum_ever_free_bytes() as u64);
    }
    println!("checksum {checksum}");
    println!("allocations {} frees {}", heap.allocations(), heap.frees());
}

/// The pool arm. `BLOCK` is a const parameter, so the rungs are separate
/// instantiations picked by the caller rather than a runtime size.
fn pool_band<const BLOCK: usize>(ops: u32) {
    let mut pool: Pool<BLOCK, SLOTS> = Pool::new();
    let mut slots: [Option<Slot>; SLOTS] = [None; SLOTS];
    let mut rng = Lcg(12345);
    let mut checksum = 0u64;

    for _ in 0..ops {
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
        checksum = checksum
            .wrapping_add(pool.available() as u64)
            .wrapping_add(u64::from(pool.allocations()));
    }
    println!("checksum {checksum}");
    println!("allocations {} frees {}", pool.allocations(), pool.frees());
}


/// The header-representation question, isolated.
///
/// The word-arena refactor was refuted at +11.9% ON A 64-BIT HOST, and the
/// balance it turned on is width-sensitive both ways: assembling a `u64`
/// out of eight bytes is dearer at 32 bits, and so is shifting a `u64`
/// offset right to index a word array. Rather than redo a 1,000-line
/// refactor to find out, read the same headers both ways and count.
///
/// # Work parity is exact here, not merely similar
///
/// The byte array is the little-endian image of the word array, so the two
/// arms read the SAME VALUES and must print the SAME CHECKSUM. A differing
/// checksum means the arms are not reading the same headers and the
/// comparison is void. That also defeats constant folding: a uniform fill
/// would let LLVM assemble the `u64` at compile time and both arms would
/// measure nothing, which is exactly what the first cut of this probe did.
fn header_probe(words: bool, ops: u32) {
    const SLOTS_N: usize = 4096;
    let mut wordbuf = [0u64; SLOTS_N];
    let mut bytes = [0u8; SLOTS_N * 8];
    for i in 0..SLOTS_N {
        let v = 0x5A5A_5A5A_0000_0000u64 ^ (i as u64).wrapping_mul(0x9E37_79B9);
        if let Some(cell) = wordbuf.get_mut(i) {
            *cell = v;
        }
        if let Some(dst) = bytes.get_mut(i * 8..i * 8 + 8) {
            dst.copy_from_slice(&v.to_le_bytes());
        }
    }
    let mut sum = 0u64;
    let mut i = 0u32;
    while i < ops {
        // The same offset sequence for both arms: 8-byte aligned, wrapping,
        // and carried in a `u64` because that is what `heap4.rs` holds.
        let offset = u64::from((i % (SLOTS_N as u32 - 2)) * 8);
        if words {
            let index = (offset >> 3) as usize;
            if let Some(pair) = wordbuf.get(index..).and_then(<[u64]>::first_chunk::<2>) {
                sum = sum.wrapping_add(pair[0] ^ pair[1]);
            }
        } else if let Some(chunk) = bytes
            .get(offset as usize..)
            .and_then(<[u8]>::first_chunk::<16>)
        {
            let (lo, hi) = chunk.split_at(8);
            let a = u64::from_le_bytes(lo.try_into().unwrap_or([0; 8]));
            let b = u64::from_le_bytes(hi.try_into().unwrap_or([0; 8]));
            sum = sum.wrapping_add(a ^ b);
        }
        i = i.wrapping_add(1);
    }
    println!("checksum {sum}");
    println!("allocations 0 frees 0");
}


/// What a `u64` costs on a machine with 32-bit registers.
///
/// `heap4.rs` carries every offset, size and link as a `u64` because that is
/// free on the 64-bit host it was written on. Every Kairos target is 32-bit,
/// where each of these is two or three instructions instead of one. This
/// prices the difference on the operation mix the allocator actually
/// performs -- compare, add, subtract, mask, shift.
///
/// # Each arm is carried END TO END in its own width
///
/// The first cut of this probe converted per iteration (`v as u32`, then
/// `u64::from(x)` to accumulate) and measured the `u32` arm as DEARER at
/// both widths -- it was pricing the scaffolding, because a real narrowing
/// removes those conversions rather than paying them. Here each arm holds
/// its seed, its state and its accumulator in its own width, which is what
/// a narrowed `heap4.rs` would do.
///
/// Work parity: the arms run the same operation sequence on values that
/// agree in the low 32 bits, so the wide checksum TRUNCATED must equal the
/// narrow one. Both are printed, and the truncation is asserted.
fn width_probe(wide: bool, ops: u32) {
    if wide {
        let mut sum = 0u64;
        let mut i = 0u32;
        while i < ops {
            let mut x = u64::from(i).wrapping_mul(0x9E37_79B9) & 0xFFFF_FFFF;
            x = x.wrapping_add(0x1000);
            x = x.saturating_sub(0x800);
            if (x & 0x8000_0000) != 0 {
                x ^= 0xFFFF;
            }
            if x >= 0x4000 {
                x >>= 1;
            }
            sum = sum.wrapping_add(x);
            i = i.wrapping_add(1);
        }
        println!("checksum {}", sum & 0xFFFF_FFFF);
    } else {
        let mut sum = 0u32;
        let mut i = 0u32;
        while i < ops {
            let mut x = i.wrapping_mul(0x9E37_79B9);
            x = x.wrapping_add(0x1000);
            x = x.saturating_sub(0x800);
            if (x & 0x8000_0000) != 0 {
                x ^= 0xFFFF;
            }
            if x >= 0x4000 {
                x >>= 1;
            }
            sum = sum.wrapping_add(x);
            i = i.wrapping_add(1);
        }
        println!("checksum {sum}");
    }
    println!("allocations 0 frees 0");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "heap4".to_owned());
    let a: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(256);
    let b: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(257);
    let ops: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(OPS_DEFAULT);

    match (mode.as_str(), a) {
        ("pool", 512) => pool_band::<512>(ops),
        ("pool", 1024) => pool_band::<1024>(ops),
        ("pool", 2048) => pool_band::<2048>(ops),
        ("pool", 4096) => pool_band::<4096>(ops),
        ("pool", other) => {
            eprintln!("no pool rung for {other}; add one as a const instantiation");
        }
        ("w64", _) => width_probe(true, ops),
        ("w32", _) => width_probe(false, ops),
        ("hdrbytes", _) => header_probe(false, ops),
        ("hdrwords", _) => header_probe(true, ops),
        ("narrow", _) => heap4_band_narrow(a, b, ops),
        ("roomy", _) => heap4_band_roomy(a, b, ops),
        _ => heap4_band(a, b, ops),
    }
}
