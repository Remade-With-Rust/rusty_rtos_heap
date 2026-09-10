# rusty_rtos_heap — package plan

**One sentence:** FreeRTOS heap_1 / heap_4 / heap_5 remade in Rust behind the Heap seam, with the heap protector; heap_3 as the seam over rusty_alloc small-metal and esp-alloc; static allocation first-class.

Family plan: Kairos `docs/plans/rtos-mission.md` (umbrella repo) — its §2.1
names what this package remakes, wraps and never touches; its §6 carries the
phase this package's kill test belongs to. This file obeys that one.

Written 2026-09-09. Status: **scaffold** — the crate layout, the feature ladder,
the lint policy and the CI gates exist; nothing is measured.

---

## 1. What it is, what it is not

**Is:** the Rust remake of the FreeRTOS component named above, exposing the
names a FreeRTOS developer already knows, with the C original as the oracle.

**Is not:** a binding to the C code, a fork of it, or a place where a chip's
registers are touched (that is a port crate).

## 2. The laws this package encodes

1. The core is `no_std` (+ `alloc`), `forbid(unsafe)`, arch-agnostic.
2. Every parser that takes bytes from a wire, a store or a bus has a
   `tests/no_panic.rs` from the day it exists.
3. Every claim has a kill test or a ledger row; the README copies this plan
   and never upgrades it.
4. Feature ladder `std` ⊃ `alloc` ⊃ core-only; CI proves the two bare-metal
   rungs on four targets on every push.

## 3. The surface as built

Nothing yet. The facade re-exports the core; the core exposes `VERSION`.

## 4. Roadmap

| Milestone | Adds | Driven by | Kill test |
|---|---|---|---|
| scaffold | the shape | K0 | a clean clone builds alone; CI green |

## 5. Deliberately absent

To be written with the first milestone.

## 6. Risks

| Risk | Mitigation |
|---|---|
| | |

## 7. Decision log

| Date | Decision |
|---|---|
| 2026-09-09 | Stamped from the Kairos template; obeys the family plan. |

---

## Measured input from K3: the allocator has a 16% routing cliff at 512 bytes

**Date: 2026-09-10.** Measured on silicon before this package is written, so
its design can account for it rather than discover it. Full method and the
reproducers are in the umbrella's `docs/LEDGER.md` and
`docs/upstream/rusty_alloc-size-class-inversion.md`.

`heap_3` in this package is the seam over `rusty_alloc` small-metal. On an
ESP32-S3, one `alloc` + one `free` through that allocator costs:

| request | cycles/op | route |
|---|---:|---|
| ≤ 512 B | 314 | small page |
| 513 – 2048 B | **271** | medium page |
| > 2048 B | 933, and history-dependent | its own large span |

**A 513-byte request is 43 cycles CHEAPER than a 512-byte one.** The step is
exactly one byte wide, the totals are byte-identical within each range, and
both boundaries are `rusty_alloc` geometry constants under `ra_small_profile`
(`SMALL_OBJ_SIZE_MAX`, `MEDIUM_OBJ_SIZE_MAX`).

### What this package must NOT do about it

Round requests up to cross the boundary. It would buy 16% for up to 25% more
memory per allocation (a 512-byte request would take a 640-byte block), and on
a part where the whole region is 64 KiB that is the wrong currency. It would
also silently stop being a win the day the allocator's own routing is fixed —
tuning against another crate's internals is a liability, not a design.

### What it should do

1. **Say it.** The seam's documentation carries the band, because a caller
   choosing a buffer size is the only party that can act on it for free.
2. **Measure `heap_1`/`heap_4`/`heap_5` against it, not against each other.**
   The comparison already exists: `rusty_rtos_core/firmware/esp32s3-devkit-alloc-ab`
   runs FreeRTOS's own `heap_4.c`, compiled verbatim from the oracle, against
   `rusty_alloc` under one harness. This package's remakes inherit that
   harness rather than growing a second one.
3. **Expect the ranking to depend on fragmentation, and gate on that.** In the
   same measurement `heap_4` looks 1.27× faster than `rusty_alloc` at 256–512
   on a *pristine* heap and is **12.5× slower** against 128 same-size holes,
   degrading ~27 cycles per free-list entry walked. A heap benchmark with one
   live block measures nothing a long-running RTOS will experience.

### And a property this package can rely on

**The kernel allocates nothing** — verified on the linked artifact, not the
source: `kairos check rusty_rtos_kernel` builds `rusty_rtos_kernel-core` for a
bare target and fails if the rlib references `__rust_alloc`. Poison-proven
(a `Box::new` turns 0 allocator symbols into 2).

That matters here twice over. It means the routing cliff above cannot reach
Kairos internally — only an *application's* allocations, through this seam.
And it means `heap_1`, the allocate-only heap FreeRTOS ships for systems that
never free, is a genuinely sufficient configuration for a Kairos system rather
than a curiosity.

**Note the check that does NOT work**, because it was believed for a while:
"it compiles with `--no-default-features`" proves nothing about allocation.
`extern crate alloc` resolves from the sysroot on a bare-metal target whatever
the feature flags say, so a `Box::new` added to the kernel compiled clean
through all eight `no_std` rungs. A claim about allocation has to be read off
the object file.
