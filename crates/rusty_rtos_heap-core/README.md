# rusty_rtos_heap-core

[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/remade-with-rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network)
[![crates.io](https://img.shields.io/crates/v/rusty_rtos_heap-core.svg)](https://crates.io/crates/rusty_rtos_heap-core)
[![docs.rs](https://docs.rs/rusty_rtos_heap-core/badge.svg)](https://docs.rs/rusty_rtos_heap-core)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

The pure `no_std` core of
[`rusty_rtos_heap`](https://crates.io/crates/rusty_rtos_heap): FreeRTOS's
`heap_1` … `heap_5` as types and algorithms, with no CPU and no operating
system. `#![forbid(unsafe_code)]`.

**This crate is a scaffold**, and is published to reserve the name and pin the
API shape rather than to be depended on for behaviour.

- **What exists**: the crate layout, the `no_std` / `alloc` / `std` feature
  ladder, the lint policy and the shared CI gate.
- **What does not**: the differential trace against the C `heap_4` — milestone
  **K4**, and the first thing here with a kill test.

**Known gaps.** Everything above the scaffold.

## Conformance

**None yet, and that is the honest answer.** The Kairos rule is that a README
makes no capability claim not backed by a test, a ledger entry or a recorded
kill test. This section stays empty until K4's differential trace passes.

## Using it

Not yet. The API is not stable and nothing behind it is proven.

## Performance

No rows. Nothing here is measured.

## Portability

Builds `no_std` on host, `thumbv7m-none-eabi`,
`riscv32imac-unknown-none-elf` and `xtensa-esp32s3-none-elf`, with and without
`alloc`. A build claim, not a behaviour claim.

## Part of Remade With Rust

This crate is part of **[Kairos](https://github.com/Remade-With-Rust/kairos)** —
FreeRTOS remade in memory-safe Rust, as independent packages that expose the API
a FreeRTOS developer already knows and prove every scheduling decision against
the C kernel's own trace. The family:
[`rusty_rtos_core`](https://crates.io/crates/rusty_rtos_core),
[`rusty_rtos_kernel`](https://crates.io/crates/rusty_rtos_kernel),
[`rusty_rtos_port`](https://crates.io/crates/rusty_rtos_port),
[`rusty_rtos_heap`](https://crates.io/crates/rusty_rtos_heap). Also check out the
rest of **[github.com/remade-with-rust](https://github.com/remade-with-rust)**.

## About Mata Network

<!-- ORG BOILERPLATE — keep identical across repos -->

[Mata Network](https://www.mata.network) builds sovereign, self-hostable
infrastructure. **Remade With Rust** is our open-source home for the
permissively-licensed building blocks that work depends on.

<!-- /ORG BOILERPLATE -->

## License

MIT OR Apache-2.0, at your option. FreeRTOS is MIT-licensed by Amazon.com, Inc.
or its affiliates; this crate remakes its API and behaviour from the published
sources and links no FreeRTOS code.
