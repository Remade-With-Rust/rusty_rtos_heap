### In The Wild with 22 Active Installs

FREE RAG Converter Online -- <a href="https://RAGconverter.com">RAGconverter.com</a>

# rusty_rtos_heap

[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/remade-with-rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network)
[![crates.io](https://img.shields.io/crates/v/rusty_rtos_heap.svg)](https://crates.io/crates/rusty_rtos_heap)
[![docs.rs](https://docs.rs/rusty_rtos_heap/badge.svg)](https://docs.rs/rusty_rtos_heap)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

The allocators for Kairos. FreeRTOS's `heap_4` and `heap_5` remade in Rust,
proven against the C by a differential over 20,000 operations: the offset
chosen, the free bytes remaining and the minimum ever free.

- **Proven**: `heap_4`'s address-ordered first fit, splitting and coalescing,
  diffed operation-for-operation against `heap_4.c` compiled verbatim from the
  pinned kernel — agreeing on the offset chosen, the free bytes remaining and
  the minimum ever free.
- **`heap_5` too**, the same free list laid out over several regions with
  real gaps — 20,000 operations, allocations landing in all three.
- **`heap_1` too**, allocate-only, proven the same way -- with its guard
  inverted, because refusal is the only branch a bump allocator has.
- **The protector is the one the representation needs**, not the C's: a
  generational handle that refuses a double free, a stale free and an
  interior offset — the last of which `heap_4.c` does not catch.
- **The RAM cost is an identity**: `total = arena + bookkeeping`, remainder 0,
  bookkeeping a fixed 56 bytes — asserted by the compiler on all four
  bare-metal targets, not just measured on the host.

**Known gaps.** `heap_3` cannot be proven against the C the way the other
three are —
`heap_3` (the seam over `rusty_alloc` small-metal and `esp-alloc`). This crate
is `heap_4` and the RAM identity. Note also that the kernel does **not need**
it: Kairos places every object in an arena declared at compile time, which is
how the family reached a byte-identical corpus on four architectures with no
heap at all.

- This package's plan: [docs/plans/rusty_rtos_heap.md](https://github.com/Remade-With-Rust/rusty_rtos_heap/blob/main/docs/plans/rusty_rtos_heap.md)
- Every number: [docs/LEDGER.md](https://github.com/Remade-With-Rust/rusty_rtos_heap/blob/main/docs/LEDGER.md)
- The family plan: Kairos [`docs/plans/rtos-mission.md`](https://github.com/Remade-With-Rust/kairos/blob/main/docs/plans/rtos-mission.md)

**Claims discipline:** this README makes no performance or capability claim that
is not backed by a test, a benchmark ledger entry, or a kill test recorded in
the plan. "Scaffold" means scaffold. "Sim only" means the sim port; "builds, not
flashed" means no chip has run it.

## Conformance

`heap_4.c`'s algorithm, transcribed over **offsets rather than pointers**, and
diffed against `heap_4.c` compiled **verbatim** from the pinned kernel.

Offsets because a pointer is not comparable across two programs and an offset
into a known aligned base is. The C driver sets
`configAPPLICATION_ALLOCATED_HEAP` so `ucHeap` is its own aligned array, which
is what makes the two arenas describable in the same coordinates.

| | |
|---|---:|
| operations diffed | **20,000** |
| agreeing on the offset first fit chose | ✅ |
| agreeing on free bytes remaining | ✅ |
| agreeing on the minimum ever free | ✅ |

```sh
cargo test -p rusty_rtos_heap-core --release
```

The C arm is generated once and checked in, so the diff runs with **no C
toolchain**.

**What the first pass got wrong, and how it was caught.** It passed first time
— and that was the problem. The original workload **never refused a single
request**, so what agreed was the easy half of an allocator: the half that
never has to say no. `the_workload_reaches_the_branches_that_matter` exists to
fail when that is true, and the numbers above are from the harder workload it
forced: **2,087 refusals, 784 distinct offsets, minimum-ever-free 1,232 of
8,192**.

**The RAM table is an identity, not a total.** `total = arena + bookkeeping`,
remainder **0** on every row, with bookkeeping a fixed **56 bytes** that does
not grow with the arena. Because a host test cannot measure a target, that same
identity is a `const` assertion the compiler evaluates on all four bare-metal
rungs — it cannot be true on the host and quietly false on the chip.

Per profile means **per pointer width**: the block header is 8 bytes on every
Kairos target and 16 on the oracle's host, and the minimum block size moves
with it.

### The protector, and why it is not the C's

`heap_4.c`'s `configENABLE_HEAP_PROTECTOR` does two things: it XORs free-list
**pointers** with a random canary, and it range-checks them. The canary exists
because a corrupted free-list pointer in C is an arbitrary-write primitive.

**That primitive does not exist here.** This heap holds bounds-checked `u64`
offsets in a crate that is `#![forbid(unsafe_code)]`, so a corrupted offset is
a wrong answer, never a write to an address somebody chose. Transcribing the
XOR would carry a mitigation across to a bug class the representation already
removed — and it would make the differential harder rather than the heap safer.

What survives the change of representation is the failure the canary was never
aimed at, plus two the C only half-checks:

| | `heap_4.c` | here |
|---|---|---|
| double free | `configASSERT`, then nothing in a release build | **`Error::Gone`** |
| freeing a **reallocated** block | undetectable — the block IS allocated | **`Error::Gone`**, via the generation |
| an **interior** offset | passes: `heapVALIDATE_BLOCK_POINTER` only range-checks, then reads a header out of user data | **`Error::InvalidArgument`**, on the alignment |
| out of range | `configASSERT` | **`Error::InvalidArgument`** |

`alloc` answers with a [`Block`] — eight bytes, `Copy`, carrying the offset and
the generation it was handed out under — and the generation is stamped into the
block's link word, which the C writes `NULL` to and nobody reads while the
block is live. **The protector therefore costs one counter and no extra
memory**: the store already happened.

`free` now returns a `Result` rather than `()`. That is the smaller half of
this change and the more useful one: silently ignoring a double free is worse
than the C, which at least stops in a debug build. Nothing is corrupted either
way — what changes is that the caller can tell.

**The generation is not optional, where the C's protector is.** The C makes it
a config because the canary costs cycles and a word of RAM; this costs a store
that was already being made, so there is no version of the package worth
shipping without it.

**It did not move the differential.** The same 20,000 operations still agree
with `heap_4.c` on the offset chosen, the free bytes remaining and the minimum
ever free — and the differential now also asserts, 20,000 times, that the
protector never refuses a legitimate free.

### `heap_1`, and why its differential is not heap_4's

`heap_1` is the allocate-only scheme: a bump index, no headers, no free list,
and `vPortFree` is **invalid to call** rather than a no-op -- `heap_1.c`'s body
is `configASSERT( pv == NULL )` under the comment "Force an assert as it is
invalid to call this function."

heap_4's workload exercises fragmentation, and none of those branches exist
here. The only interesting branch a bump allocator has is **refusal**, so this
workload runs the arena to exhaustion and keeps asking: **1,973 of 2,000
operations are refused**, and all 2,000 agree with the C on the offset chosen
and the free bytes remaining.

That is heap_4's lesson applied rather than relearned. Its first workload never
refused a request, so what agreed was the easy half of an allocator, and the
guard there fails when refusals are too few. Here the guard is **inverted** --
it fails if the arena is never exhausted. Same test, opposite direction: a
differential whose workload cannot fail is a differential about nothing.

**Poison-proven.** Weakening the bound from `>=` to `>` -- one character --
diverges at operation 42: ours accepts an allocation at offset 8,152 that the C
refuses. Three tests catch it.

Three of the C's quirks are the specification here, and a reimplementation
would get all three wrong: the alignment is applied to the **request** before
the bump rather than to the resulting offset; the bound is **strictly** less
than, so the last bytes can never be handed out; and the usable arena is
`N - ALIGN`, because the C aligns its heap's start at run time and can lose up
to `ALIGN - 1` bytes doing it. We lose none and subtract it anyway, because the
size this allocator refuses on has to be the size the C refuses on.

### `heap_5`, which is `heap_4` over several regions

`heap_5.c` and `heap_4.c` share `pvPortMalloc`, `vPortFree` and
`prvInsertBlockIntoFreeList` almost line for line; only initialisation
differs. So there is **one free list here and two ways to lay it out**, rather
than the duplication the two C files carry — copying that would copy its cost,
two homes for one bug.

**20,000 operations across three regions** agree with the C: a 4 KiB, a 2 KiB
and a 4 KiB region with real 512-byte gaps between them, and allocations land
in all three (5,717 / 1,581 / 2,332).

The gaps are the point. Coalescing is an address comparison, so what stops two
regions merging is that the first one's end is not the second one's start. A
transcription that quietly treated the arena as contiguous would hand out a
block spanning a gap, and a standing test checks every allocation against both
gaps for exactly that.

**Two things the C decided, that the plan for this work had guessed wrong:**

* **Regions must arrive in increasing address order, and the C asserts it
  rather than sorting** — `configASSERT( ( size_t ) xAddress > ( size_t )
  pxEnd )`, under the comment "Check blocks are passed in with increasing
  start addresses". The plan had assumed the C sorted them and that expecting
  sorted input would be the bug; it is the opposite, so out-of-order regions
  are refused.
* **Even ABUTTING regions do not merge.** A test written to assert they would
  failed: every region reserves its own end marker at its top, so region 0's
  block ends at its marker and not at region 1's start. The addresses never
  meet. `vPortDefineHeapRegions` is therefore not a way to describe one arena
  in pieces — each piece costs a marker and none of them merge.

**Poison-proven.** Letting each region's block claim eight bytes it does not
own diverges from the C immediately.

### `heap_3`, and the claim it cannot make

`heap_3.c` is thirty lines and all of them are `malloc` / `free` inside
`vTaskSuspendAll()` / `xTaskResumeAll()`. **Its entire content is the lock, not
the allocation.**

Ours routes to the **global** allocator — whatever the deliverable declared —
rather than to a named one. A seam that named `rusty_alloc` would be choosing
for the consumer, and choosing badly for much of the hardware FreeRTOS exists
to serve: `rusty_alloc`'s `MIN_REGION` is **65,536 bytes**, the entire SRAM of
a classic FreeRTOS QEMU target, while `heap_4` runs in whatever arena you
declare and its differential runs in 8 KiB. Those are different floor
*functions*, not one tuned differently.

**There is no differential, and that is structural.** `heap_3.c` forwards to
`malloc`, so diffing against it would compare whichever libc the oracle linked
with whichever global allocator the binary declared — two third parties, called
conformance. The claim here is a different KIND and is labelled as such: the
same 20,000-operation workload runs over an external allocator and **the
accounting reconciles exactly** — allocations equal frees, live bytes zero,
live slots zero.

**It hands out a handle, not a pointer**, and the lint is why. The first draft
returned `*mut u8` and needed `unsafe` for the raw allocation, the header
arithmetic and the `Layout` reconstruction; this crate is
`#![forbid(unsafe_code)]` and the family keeps its `unsafe` in the port crates.
The lint was right — a handle is what the rest of the family hands out, and it
removes the same bug class here. `Box<[u8]>` owns the bytes, so a double free
and a free of a reused slot are both refused, which `heap_3.c` cannot do at
all.

**Still open:** nothing in this package. `heap_1`, `heap_3`, `heap_4` and
`heap_5` are all built. `heap_3` — the seam over
`rusty_alloc` small-metal and `esp-alloc` — is not here either. This crate is
`heap_4` and the RAM identity, and nothing else claims to exist.

## Using it

```rust
use rusty_rtos_heap_core::Heap4;

// The arena is a CONST GENERIC, not a borrowed slice: the geometry is declared
// and the `.bss` cost is exactly what was asked for. `ALIGN` and `LINK` are the
// port's alignment and the link-field width.
const TOTAL: usize = 8192;
const ALIGN: usize = 8;
const LINK: usize = 8;

let mut heap: Heap4<TOTAL, ALIGN, LINK> = Heap4::new();

// `alloc` answers with an OFFSET into the arena, not a pointer -- which is the
// same choice that made the differential against the C possible at all, since
// a pointer is not comparable across two programs.
let a = heap.alloc(64).expect("room for 64 bytes");
let b = heap.alloc(128).expect("room for 128 bytes");

// The three quantities the differential checks are the ones you can read back.
let _free = heap.free_bytes();
let _low_water = heap.minimum_ever_free_bytes();

heap.free(a);
heap.free(b);   // adjacent blocks coalesce, as `heap_4.c` does

// Need a real address? Ask for one; the offset stays the currency.
// let ptr = heap.address_of(a);
```

Address-ordered first fit; a block is split only when the remainder is strictly
larger than twice the header; a freed block coalesces with the block before
**and** the block after. Those are `heap_4.c`'s rules, and the differential is
what proves they are followed rather than approximated.

## Performance

One row, and it is an arithmetic identity rather than a timing:

| | bytes |
|---|---:|
| arena | as declared |
| bookkeeping | **56**, fixed — it does not grow with the arena |
| **total** | **arena + 56**, remainder **0** on every row |

`what_one_allocation_costs_is_the_cs_arithmetic` pins the per-allocation cost
against the C's own arithmetic rather than against a measurement, because the
cost of one allocation in `heap_4` IS arithmetic: the header, plus the rounding
to the alignment.

No cycle counts. Nothing in this crate has been timed on a part, and a
`Performance` section quoting a number nobody measured is exactly what this
family's ledger discipline exists to prevent.

## Portability

`no_std` everywhere, with `alloc` and `std` rungs above it. The RAM identity is
a `const` assertion, so it is checked by the compiler on each of these rather
than inferred from the host:

| target | builds | RAM identity asserted |
|---|---|---|
| host (x86-64 Windows, Linux) | ✅ | ✅ |
| `thumbv7m-none-eabi` | ✅ | ✅ |
| `riscv32imac-unknown-none-elf` | ✅ | ✅ |
| `xtensa-esp32s3-none-elf` | ✅ | ✅ |

The header is 8 bytes on every Kairos target and 16 on the oracle's host, which
is why "per profile" in the plan means per pointer width.

## Layout

```text
crates/rusty_rtos_heap          facade: re-exports + prelude; the crate you depend on
crates/rusty_rtos_heap-core     no_std (+ alloc); forbid(unsafe); types, traits, algorithms
firmware/                per-chip example projects, excluded from the workspace
docs/plans/              this package's plan and its hardening audit
docs/LEDGER.md           every number, with its method line
```

## Build

```sh
cargo test --workspace                                   # host: the tests
cargo check -p rusty_rtos_heap-core --no-default-features \
  --target thumbv7em-none-eabihf                         # Cortex-M4F class, no alloc
cargo check -p rusty_rtos_heap-core --no-default-features --features alloc \
  --target riscv32imac-unknown-none-elf                  # ESP32-C6 class, with alloc
```

CI holds the core to `thumbv7em-none-eabihf`, `thumbv8m.main-none-eabihf`,
`riscv32imac-unknown-none-elf` and `riscv32imafc-unknown-none-elf`, with and
without `alloc`, plus `cargo deny check`. Firmware examples (Xtensa needs the
esp toolchain; Cortex-M and RISC-V work on stable) are built from their own
directories under `firmware/`.

## Part of Remade With Rust

This crate is part of **[Kairos](https://github.com/Remade-With-Rust/kairos)** —
FreeRTOS remade in memory-safe Rust, as independent packages that expose the API
a FreeRTOS developer already knows and prove every scheduling decision against
the C kernel's own trace. `rusty_rtos_heap` is the allocator seam: `heap_4`, proven against the C by a differential.

**Where this sits for Mata.** Kairos is the real-time layer on the device
itself, and [`rusty_rtos_mqtt`](https://github.com/Remade-With-Rust/rusty_rtos_mqtt) is the way out of it.
Paired with the **MATA distributed cloud**, robotics and sensor data has two
routes — read it on the machine, or reach it through the cloud — with the same
memory-safe crates at both ends.

The family:
[`rusty_rtos_core`](https://crates.io/crates/rusty_rtos_core) (the shared vocabulary),
[`rusty_rtos_kernel`](https://crates.io/crates/rusty_rtos_kernel) (the scheduler),
[`rusty_rtos_port`](https://crates.io/crates/rusty_rtos_port) (the architecture seam),
[`rusty_rtos_heap`](https://crates.io/crates/rusty_rtos_heap) (the allocators),
[`rusty_rtos_json`](https://github.com/Remade-With-Rust/rusty_rtos_json) (coreJSON),
[`rusty_rtos_sntp`](https://github.com/Remade-With-Rust/rusty_rtos_sntp) (coreSNTP),
[`rusty_rtos_mqtt`](https://github.com/Remade-With-Rust/rusty_rtos_mqtt) (coreMQTT),
[`rusty_rtos_backoff`](https://github.com/Remade-With-Rust/rusty_rtos_backoff) (backoffAlgorithm),
[`rusty_rtos-capi`](https://github.com/Remade-With-Rust/rusty_rtos-capi) (the C ABI) and
[`rusty_rtos_demo`](https://github.com/Remade-With-Rust/rusty_rtos_demo) (the conformance corpus).
The last six are on GitHub and not yet on crates.io. Also check out
the rest of **[github.com/remade-with-rust](https://github.com/remade-with-rust)**.

## About Mata Network

<!-- ORG BOILERPLATE — keep identical across repos -->

**[Mata Network](https://www.mata.network/)** builds sovereign, self-hostable
privacy infrastructure — *"stop sacrificing your privacy for convenience"*:
wallet & identity, a password manager, a contact manager, and a browser
extension that stops your information leaking as you browse.

**Remade With Rust** is our open-source home for the permissively-licensed
building blocks that work depends on — including
[remade_ffmpeg_rs](https://github.com/Remade-With-Rust/remade_ffmpeg_rs) (the
FFmpeg alternative) and [FFAI](https://github.com/Remade-With-Rust/FFAI) (the
AI media toolkit).

→ **[www.mata.network](https://www.mata.network/)**

<!-- /ORG BOILERPLATE -->

## License

MIT OR Apache-2.0, at your option. FreeRTOS is MIT-licensed by Amazon.com,
Inc. or its affiliates; this crate remakes its API and behaviour from the
published sources and links no FreeRTOS code.

---

<!-- HARDENING-TABLE:BEGIN generated by use-protection-please — edit docs/plans/use-protection-please.md, not this block -->
## Hardening status

**Tier** critical-path · **Audited** 2026-09-16 (v0.1.0 release pass) · **v1.0.0 gates** 7/17 · [Full checklist](https://github.com/Remade-With-Rust/rusty_rtos_heap/blob/main/docs/plans/use-protection-please.md)

`██████░░░░░░░░░░░░░░` **31%** &nbsp;·&nbsp; 11 Completed · 0 Scheduled · 25 Incomplete · 19 N/A

| Phase | ✅ Completed | 🗓 Scheduled | ⬜ Incomplete | · N/A |
|---|--:|--:|--:|--:|
| 0 — Threat modeling | 0 | 0 | 2 | 0 |
| 1 — Toolchain | 2 | 0 | 2 | 0 |
| 2 — Supply chain | 5 | 0 | 3 | 0 |
| 3 — Code level | 3 | 0 | 4 | 0 |
| 4 — Static analysis | 0 | 0 | 1 | 0 |
| 5 — Dynamic analysis | 0 | 0 | 3 | 0 |
| 6 — Fuzzing and properties | 0 | 0 | 4 | 0 |
| 7 — Formal verification | 0 | 0 | 1 | 0 |
| 8 — Build and binary | 0 | 0 | 1 | 1 |
| 9 — Runtime privilege | 0 | 0 | 0 | 1 |
| 10 — Cryptography | 0 | 0 | 0 | 3 |
| 11 — CI/CD, release, and operations | 1 | 0 | 4 | 0 |
| 12 — Compliance controls | 0 | 0 | 0 | 14 |
| **Total** | **11** | **0** | **25** | **19** |

Gates waived for 0.x are listed with their reasons in the plan's "v0.1.0 release decision" section — an Incomplete gate not listed there is an omission, not a decision.

**Architect** — [Tim Almond](https://github.com/Ttimmahlax) — accountable for this unit's security design; rendered
<!-- HARDENING-TABLE:END -->
