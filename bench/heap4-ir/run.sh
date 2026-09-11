#!/bin/sh
# Instruction counts for heap_4, under callgrind. Run inside WSL.
#
# The count is the verdict; a clock on a 20,000-operation workload would be
# measuring the box. The checksum is printed so a compiler that removed the
# work shows up as a changed number rather than a good one.
#
# THE FRESHNESS CHECK IS NOT OPTIONAL, and this script has already been
# wrong once. Its first version tried `cargo build --target
# x86_64-unknown-linux-gnu` with stderr discarded and fell back to a plain
# build; the explicit target SUCCEEDED, putting the binary under
# target/x86_64-unknown-linux-gnu/release/, while the script went on to
# profile the stale one left in target/release/ by an earlier run. It
# reported an instruction count identical to the digit across a real source
# change — which is exactly what a stale binary looks like, and exactly why
# `codec-measurement` says to check.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here"

bin=target/release/heap4-ir
cargo build --release
[ -f "$bin" ] || { echo "no binary at $bin" >&2; exit 1; }

# Every source that can change the count must be OLDER than the binary.
newest=$(find ../../crates/rusty_rtos_heap-core/src src -name '*.rs' -newer "$bin" -print -quit)
if [ -n "$newest" ]; then
    echo "STALE: $newest is newer than $bin — the build did not take" >&2
    exit 1
fi

rm -f callgrind.out
valgrind --tool=callgrind --callgrind-out-file=callgrind.out \
         --cache-sim=no --branch-sim=no "./$bin" 2>&1 | grep -E "checksum|allocations|refs:"
echo "--- per file ---"
callgrind_annotate callgrind.out 2>/dev/null | sed -n '18,26p'
