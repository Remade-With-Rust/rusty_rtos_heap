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

## 4b. The K4 build map (2026-09-16)

**K4's kill test passed on 2026-09-10** and the milestone row says so. What
follows is the rest of K4's declared SCOPE, which is a different thing and is
not built. The distinction matters because the row reads PASSED and a reader
could reasonably conclude the package is finished.

| K4 scope | state |
|---|---|
| `heap_4` | **built and proven** — 20,000 operations diffed against `heap_4.c` |
| `StaticAllocation` first-class | **met, and in a stronger form than the C's** — the C demo shows a system *can* be built with `configSUPPORT_DYNAMIC_ALLOCATION 0`; here it cannot be built any other way, gated on the linked rlib and poison-proven both directions |
| RAM table per profile | **met** — an identity, remainder 0, `const`-asserted on four rungs |
| `heap_1` | **built and proven** -- 2,000 operations, 1,973 of them refusals |
| `heap_5` | **built and proven** -- 20,000 operations over three regions |
| `heap_3` seam over `rusty_alloc` / `esp-alloc` | not written |
| the protector | **needs a decision before it needs code** — see below |

### The method is already proven, so reuse it rather than reinvent it

The `heap_4` differential is the template and it works: transcribe the C's
algorithm over **offsets**, compile the C arm **verbatim** from the pinned
kernel, generate its trace once and check it in so the diff needs no C
toolchain, then diff operation-for-operation on the quantities both sides can
name — the offset chosen, free bytes remaining, minimum ever free.

It also carries the lesson that makes it worth copying: **it passed first
time, and that was the problem.** The first workload never refused a single
request, so what agreed was the easy half of an allocator.
`the_workload_reaches_the_branches_that_matter` is the guard against that, and
every new differential below needs its own version of it. A differential whose
workload cannot fail is a differential about nothing.

### 1. `heap_1` — BUILT 2026-09-16

178 lines of C, and `vPortFree` is a no-op that asserts. Allocate-only, bump
upward, never coalesce, never refuse except at exhaustion.

* **Build:** `Heap1<N, ALIGN>` beside `Heap4`, same const-generic shape.
* **Kill test:** the same differential harness, pointed at `heap_1.c`. The
  branch guard is inverted here — the workload must reach **exhaustion**,
  because refusal is the only interesting branch a bump allocator has.
* **Why first:** to learn whether the differential harness generalises beyond
  the allocator it was written for.

**It did, and the harness needed nothing.** `oracle/run-heap1.sh` compiles
`heap_1.c` verbatim beside a second driver, the trace is checked in, and the
Rust side diffs it the same way. 2,000 operations agree.

**What it found before running a single operation.** The first driver freed
every eighth allocation to show that freeing does nothing; the C arm exited 2
with `configASSERT failed: pv == NULL`. `heap_1`'s `vPortFree` is not a no-op,
it is an assertion that you did not call it -- "Force an assert as it is
invalid to call this function". Our first draft answered `Ok(())`, which would
have let a caller do silently what the C stops them doing loudly. It answers
`Error::Unsupported` now, which is also the right answer for a caller generic
over the heaps: choosing `heap_1` for a system that frees should fail at the
first free rather than never.

**The guard is inverted, and that is the transferable part.** heap_4's guard
fails when the workload refuses too FEW requests, because its first version
never refused any and agreed about the easy half of an allocator. A bump
allocator's only interesting branch IS refusal, so heap_1's guard fails when
the arena is never exhausted. Both say the same thing: a differential whose
workload cannot fail is a differential about nothing.

**Poison-proven**, which a first-time pass makes mandatory rather than
optional: weakening the strictly-less-than bound to `>` diverges at operation
42 -- ours accepts an allocation at offset 8,152 that the C refuses.

### 2. `heap_5` — BUILT 2026-09-16

756 lines of C, which is the biggest of them because it is heap_4 plus region
handling. Adds `vPortDefineHeapRegions( const HeapRegion_t * )`, and with it
`vPortGetHeapStats` and `xPortResetHeapMinimumEverFreeHeapSize`.

* **The real question is representation, not algorithm.** `Heap4` uses an
  offset into one arena. Multiple regions means an offset is no longer
  self-describing: either a `(region, offset)` pair, or one global offset space
  with a region table mapping ranges. The second keeps the existing free-list
  code and the existing differential unchanged, and is what the C effectively
  does by linking regions into one address-ordered list.
* **Build:** reuse `Heap4`'s free list wholesale; the difference is
  initialisation and the bounds check.
* **Kill test as built:** three regions of different sizes with real gaps,
  20,000 operations, allocations landing in all three (5,717 / 1,581 / 2,332).
  The guard fails if any region is allocated in fewer than a hundred times — a
  three-region differential that only ever reaches the first one is a
  one-region differential in a costume.

**The plan's branch guard was wrong, and the C said so.** It read "defined out
of address order — the C sorts them". The C does not sort: it asserts.

```c
/* Check blocks are passed in with increasing start addresses. */
configASSERT( ( size_t ) xAddress > ( size_t ) pxEnd );
```

So accepting unsorted regions would have been the bug, not refusing them. That
is the second time in this milestone that reading the pinned source corrected a
plan written from memory — the first was `heap_1`'s `vPortFree`, which is an
assertion and not a no-op. The oracle is pinned so it can be read; a plan is a
hypothesis about it.

**And a test that failed taught the more interesting fact.** It was written to
assert that two ABUTTING regions coalesce, "because then they are one region".
They do not: every region reserves its own end marker at its top, so region
0's block ends at its marker rather than at region 1's start, and the addresses
never meet. `vPortDefineHeapRegions` is not a way to describe one arena in
pieces — each piece costs a marker and none of them merge.

**Poison-proven:** letting each region's block claim eight bytes it does not
own diverges from the C immediately.

### 3. `heap_3` — the battlefield, mapped 2026-09-16

**The blocker this plan recorded was stale.** It said `rusty_alloc`'s
`prim::fixed` was "measured on the S3 only" and unfiled on Cortex-M. It has run
on a Cortex-M3 since 2026-09-10 and still does — re-run today,
`rusty_rtos_core/firmware/mps2-an385-qemu-region`, **9/9**: `give()` answering
196,608 usable bytes, a second `give()` refused with `FERR_REGISTERED`, a `Box`
and a collection served, and every allocation proven inside the region by
`region_contains`. `build-me-bare` B4a closed it and this plan was not updated.

So the correctness question is answered. What follows is the rest of the
ground, because the interesting part was never whether it works.

#### The two floors are different FUNCTIONS

| | floor |
|---|---|
| `heap_4` | `bytes_live + header` — 8 bytes per block, in whatever arena you declare. The differential runs in **8 KiB** |
| `rusty_alloc` | `segments x 64 KiB` — **`MIN_REGION` is 65,536 bytes**, `REGION_ALIGN` 16, and a region is whole segments: 225,280 asked gives 196,608 reserved |

That is not one function tuned differently, it is two shapes. `heap_4`'s cost
follows what you use; `rusty_alloc`'s does not depend on what you allocate at
all.

**And the consequence has already bitten this repository once.** The M3 region
cell had to move from `lm3s6965evb` to `mps2-an385` for a reason that is
arithmetic rather than taste:

```text
MIN_REGION      65536 bytes    one segment at this geometry
LM3S6965 SRAM   65536 bytes    the whole chip
```

The smallest region the allocator accepts **is the entire SRAM** of a classic
FreeRTOS QEMU target, leaving nothing for stack, `.data` or `.bss`. A great
many of the parts FreeRTOS exists for cannot run `rusty_alloc` at all.

#### Speed, from silicon, with a zero-cycle null arm

`esp32s3-devkit-alloc-ab`, `heap_4.c` compiled verbatim with
`xtensa-esp32s3-elf-gcc` under the identical CCOUNT harness, arms ABBA
interleaved, checksums compared, null A/B **0 cycles**:

| request | `rusty_alloc` | `heap_4` | |
|---|---:|---:|---|
| 16 B | 99 | 236 | **2.38x faster** |
| 32 B | 109 | 236 | 2.17x faster |
| 64 B | 141 | 236 | 1.67x faster |
| 128 B | 193 | 236 | 1.22x faster |
| 256-512 B | 298 | 236 | **1.27x SLOWER** |
| 1024-2048 B | 255 | 236 | 1.08x slower |

A crossover, not a win — and `heap_4`'s flat 236 is its **best case**, which
that row says out loud: the workload keeps one block live, so the free list is
one entry and first fit answers in one step.

#### What is genuinely outstanding, and it is hardware

`build-me-bare` B4b wants that cycle row on a **Kairos target**. The Xtensa arm
is done; the riscv32 arm needs an ESP32-C6, and none is in hand. **QEMU cannot
substitute**, which the M3 cell measured rather than assumed: DWT `CYCCNT`
reads **0**, and SysTick deltas SHRINK as the loop grows — 848 / 548 / 353 for
1k / 2k / 4k iterations — because they track host wall time while TCG's
translation cache warms, not guest work. Under `-icount shift=0` they read
1 / 0 / 0. A cell that cannot count cannot supply a number.

#### The thing `heap_3` would have to solve that `heap_3.c` does not

`heap_3.c` is thirty lines and all of them are `malloc`/`free` inside
`vTaskSuspendAll()` / `xTaskResumeAll()`. **Its entire content is the lock, not
the allocation.**

In Rust the platform allocator is the *global* allocator, and there is a
mismatch the C does not have: `free( void * )` takes an address, and
`alloc::alloc::dealloc` requires the `Layout` back. So a Rust `heap_3` must
store the size it allocated — a header, which `heap_3.c` does not need because
libc keeps that bookkeeping itself. That is a real divergence and it means a
Rust `heap_3` is not quite the trivial forward the C is.

#### The decision

**Build it over the GLOBAL allocator, not over `rusty_alloc`.**

`heap_3.c` routes to the platform's allocator. In Rust that is whatever the
deliverable declared — the system allocator on a host, `rusty_alloc` on a
firmware that registered a region, anything else a consumer chooses. A seam
that named `rusty_alloc` would be choosing for them, and choosing badly for
every part under 64 KiB.

That also makes it the `Heap5` shape again: a thin type over machinery it does
not own, exposing the same `alloc` / `free_raw` / `free_bytes` surface so a
caller generic over the heaps sees one shape.

**And its kill test is a different KIND of claim, which must be labelled.**
There is no differential to write: diffing against `heap_3.c` would measure
whichever libc the oracle linked, not ours. So the honest claim is "the same
workload runs over an external allocator and the accounting reconciles", and it
must not be presented as a fourth differential. Three allocators are proven
against the C; a fourth that cannot be would dilute what "K4 passed" means if
the difference were blurred.

**What a consumer must be told:** an external allocator gets neither the
generational protector nor the allocated-bit check, because both live in *our*
block headers. A `heap_3` caller gets whatever the global allocator gives them.

### 3b. `heap_3` — the original note

106 lines of C, and all of them wrap `malloc`/`free` in a critical section.
Kairos's version is the seam over `rusty_alloc` small-metal
(`prim::fixed::Region<N>`) and `esp-alloc`.

* **There is no differential to write.** The C arm's behaviour is libc's, so
  diffing against it would measure whichever libc the oracle linked. The kill
  test is the seam's: the same kernel workload runs unchanged over this heap,
  and the arena accounting still reconciles.
* **Blocked on a measurement that is already recorded as owed:** the mission
  plan's own row says `rusty_alloc`'s `prim::fixed` has been measured on the S3
  only, and its behaviour on Cortex-M is unfiled. That is a prerequisite, not a
  detail — a seam over an allocator nobody has run on the target is a seam over
  an assumption.

### 4. The protector — DECIDED AND BUILT 2026-09-16

`configENABLE_HEAP_PROTECTOR` in the pinned kernel is two things:
`heapPROTECT_BLOCK_POINTER( pxBlock )`, which XORs a free-list **pointer** with
a random canary, and `heapVALIDATE_BLOCK_POINTER`, which asserts the pointer
lies inside the heap.

**Most of what it protects against, this representation has already removed.**
The canary exists because a corrupted free-list pointer in C is an arbitrary
write primitive. `Heap4` holds `u64` offsets, bounds-checked on use, in a crate
that is `#![forbid(unsafe_code)]` — a corrupted offset is a wrong answer, not a
write to an attacker's address. Transcribing the XOR verbatim would be
cargo-culting a mitigation for a bug class that cannot occur here, and it would
make the differential harder rather than the heap safer.

**What does survive the change of representation**, and is the real work:

* **double free** — freeing an offset twice is still possible and still
  corrupts the free list;
* **a stale offset** — freeing an offset from a previous allocation at the same
  place, which is the offset-world equivalent of a dangling pointer;
* **a bogus offset** — one that never came from `alloc`.

The third is already handled by the bounds check. The first two are not, and
the family already has the answer it uses everywhere else for exactly this
shape of problem: `rusty_rtos_core`'s **generational handle**. An
`alloc` that answers with `(offset, generation)` and a `free` that checks the
generation makes a double free and a stale free both `Error::Gone` rather than
silent corruption — the same trade the kernel's arenas already make.

**The decision taken:** `alloc` answers with a `Block` — offset plus
generation, eight bytes, `Copy` — and `free` takes one and returns a `Result`.

Two worries in the original framing turned out not to apply.

*"It costs bytes per block."* It costs none. The generation is stamped into the
block's link word, which `heap_4.c` writes `NULL` to on allocation and nobody
reads while the block is live. The store already happened; only the value
changed. The allocated bit still separates a live block from a free one, so the
free path checks that FIRST and never mistakes a free block's next pointer for
a generation.

*"The differential needs a mode that switches it off."* It does not. The
differential compares the offset chosen, the free bytes remaining and the
minimum ever free — none of which the generation touches. All 20,000 operations
still agree, and the differential now additionally asserts 20,000 times that
the protector never refuses a legitimate free, which is a guard it did not have
before.

What the change did cost was the public shape of `alloc`, and that is a real
divergence from `pvPortMalloc`. It is the same KIND of divergence the package
already made deliberately when it chose offsets over pointers, for the same
reason: a pointer is not comparable across two programs, and an offset alone
cannot distinguish the allocation you were handed from the one living at that
address now. `rusty_rtos_core::Handle` answers that question everywhere else in
this family; the heap now answers it the same way.

`free` also returns `Result` rather than `()`. That is the smaller half and the
more useful one: the old code DETECTED a double free and then returned
silently, which is worse than the C — the C at least stops in a debug build.
Nothing was ever corrupted; the caller simply could not be told.

Built with five unit tests for the cases needing a forged offset (interior, out
of range, below the first header, never allocated, right offset with the wrong
generation) and five integration tests for the ones a caller could really reach
(double free, freeing a reallocated block, refused frees changing nothing,
generations not repeating, the ordinary path unchanged). `Block` has no public
constructor, which is why the forging tests live inside the module rather than
widening the API to reach them.

### Order, and why

~~`heap_1`~~ → ~~protector~~ → ~~`heap_5`~~ **(all done 2026-09-16)** → `heap_3`.

`heap_1` first because it is small and it tests the harness. The protector
decision next because `heap_5` would inherit the API. `heap_3` last because it
is blocked on a `rusty_alloc` measurement this plan does not own.

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
exactly one byte wide and the totals are byte-identical within each range.

**Why, resolved upstream 2026-09-10 — and it is a POINTER-WIDTH boundary, not
a page-kind one.** `SMALL_SIZE_MAX = SMALL_WSIZE_MAX * INTPTR_SIZE`, so it is
1,024 on a 64-bit host and **512 on every Kairos target**, which are all
32-bit. Below it an allocation takes the `direct[]` route, which retires and
re-carves its page on every periodic `GENERIC_COLLECT_DEFAULT` sweep (512 at
the small profile); above it the bin route never churns. Counted on silicon:
**21 page retires per 10,240 ops below the line, 0 above it**, with the
generic path entered on *every* operation either side — so the slow path is
not the difference, the churn is.

**This matters to this package specifically**: every target in `KAIROS.toml`
is 32-bit, so `heap_3` over this allocator sits on the 512 boundary on all of
them, and a host build of the same code will not show it.

### What this package must NOT do about it

Round requests up to cross the boundary. It would buy 16% for up to 25% more
memory per allocation (a 512-byte request would take a 640-byte block), and on
a part where the whole region is 64 KiB that is the wrong currency. It would
also silently stop being a win the day the allocator's own routing is fixed —
tuning against another crate's internals is a liability, not a design.

### What it should do

1. **Say it.** The seam's documentation carries the band, because a caller
   choosing a buffer size is the only party that can act on it for free.
2. **`--cfg ra_generic_collect` is the knob, and it is a workload question.**
   Upstream shipped `"64" | "4096" | "65536"` after this was reported: short
   sweeps buy back a starvation the small profile had at 10,000 allocations
   and cost page churn; long sweeps do the reverse. A bench holding one block
   live cannot decay, so it sees only the cost — which is why this package
   must measure its own workload before moving it, and why nothing here
   should change the default.
3. **Measure `heap_1`/`heap_4`/`heap_5` against it, not against each other.**
   The comparison already exists: `rusty_rtos_core/firmware/esp32s3-devkit-alloc-ab`
   runs FreeRTOS's own `heap_4.c`, compiled verbatim from the oracle, against
   `rusty_alloc` under one harness. This package's remakes inherit that
   harness rather than growing a second one.
4. **Expect the ranking to depend on fragmentation, and gate on that.** In the
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
