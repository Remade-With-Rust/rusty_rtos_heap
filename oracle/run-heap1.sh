#!/bin/sh
# Generate the C arm of K4's heap_1 differential.
#
# heap_1.c is compiled VERBATIM out of the pinned kernel checkout in the
# umbrella; this script never copies or edits it. The trace it writes is
# checked in, so the Rust side can be diffed in CI with no C toolchain --
# the same arrangement `run.sh` uses for heap_4 and the kernel corpus uses
# for `oracle/traces/*`.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
kernel="$here/../../oracle/FreeRTOS-Kernel"
heap1="$kernel/portable/MemMang/heap_1.c"

[ -f "$heap1" ] || { echo "no heap_1.c at $heap1 -- run \`kairos oracle fetch\` first" >&2; exit 1; }

cc -O2 -g -Wall -Wextra -Wno-unused-parameter \
   -I "$here" \
   -o "$here/heap1_driver" \
   "$heap1" "$here/heap1_driver.c"

"$here/heap1_driver" > "$here/heap1.trace"
echo "wrote $(wc -l < "$here/heap1.trace") lines to $here/heap1.trace"
