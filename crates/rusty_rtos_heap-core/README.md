# rusty_rtos_heap-core

[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/remade-with-rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network)
[![crates.io](https://img.shields.io/crates/v/rusty_rtos_heap-core.svg)](https://crates.io/crates/rusty_rtos_heap-core)
[![docs.rs](https://docs.rs/rusty_rtos_heap-core/badge.svg)](https://docs.rs/rusty_rtos_heap-core)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

The pure `no_std` core of
[`rusty_rtos_heap`](https://crates.io/crates/rusty_rtos_heap): FreeRTOS's
`heap_4` as types and algorithms, with no CPU and no operating system.
`#![forbid(unsafe_code)]`.

- **`Heap4`**: address-ordered first fit, split only when the remainder is
  strictly larger than twice the header, coalesce with the block before and
  after — `heap_4.c`'s rules, transcribed over offsets rather than pointers.
- **Proven against the C**: 20,000 operations agreeing on the offset first fit
  chose, the free bytes remaining and the minimum ever free.

**Known gaps.** `heap_1`, `heap_3` and `heap_5` are not written.

## Conformance

Diffed against `heap_4.c` compiled **verbatim** from the pinned kernel, over
**offsets** — a pointer is not comparable across two programs; an offset into a
known aligned base is.

**20,000 operations**, agreeing on the offset first fit chose, the free bytes
remaining and the minimum ever free. The C arm is generated once and checked
in, so the diff needs no C toolchain.

```sh
cargo test -p rusty_rtos_heap-core --release
```

It passed first time, and the first workload **never refused a request** — so
what agreed was the easy half of an allocator. `the_workload_reaches_the_
branches_that_matter` fails when that is true; the quoted agreement is on the
harder workload it forced: 2,087 refusals, 784 distinct offsets,
minimum-ever-free 1,232 of 8,192.

## Using it

```rust
use rusty_rtos_heap_core::Heap4;

// The arena is a const generic; `alloc` answers with an OFFSET, not a pointer.
let mut heap: Heap4<8192, 8, 8> = Heap4::new();
let p = heap.alloc(64).expect("room for 64 bytes");
heap.free(p);   // coalesces with its neighbours, as `heap_4.c` does
```

## Performance

`total = arena + bookkeeping`, remainder **0**, bookkeeping a fixed **56
bytes** that does not grow with the arena — and because a host test cannot
measure a target, the same identity is a `const` assertion the compiler
evaluates on all four bare-metal rungs.

No cycle counts: nothing here has been timed on a part.

## Portability

`no_std` on host, `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and
`xtensa-esp32s3-none-elf`. The RAM identity is asserted by the compiler on each
of them, so it is a behaviour claim there and not only a build one. The header
is 8 bytes on every Kairos target and 16 on the oracle's host.

## Part of Remade With Rust

This crate is part of **[Kairos](https://github.com/Remade-With-Rust/kairos)** — FreeRTOS remade in memory-safe
Rust, as independent packages that expose the API a FreeRTOS developer already
knows and prove every scheduling decision against the C kernel's own trace.

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

MIT OR Apache-2.0, at your option. FreeRTOS is MIT-licensed by Amazon.com, Inc.
or its affiliates; this crate remakes its API and behaviour from the published
sources and links no FreeRTOS code.
