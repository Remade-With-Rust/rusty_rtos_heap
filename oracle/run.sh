#!/bin/sh
# Generate the C arm of K4's differential.
#
# heap_4.c is compiled VERBATIM out of the pinned kernel checkout in the
# umbrella; this script never copies or edits it. The trace it writes is
# checked in, so the Rust side can be diffed in CI with no C toolchain —
# the same arrangement the kernel corpus uses for `oracle/traces/*`.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
kernel="$here/../../oracle/FreeRTOS-Kernel"
heap4="$kernel/portable/MemMang/heap_4.c"

[ -f "$heap4" ] || { echo "no heap_4.c at $heap4 — run \`kairos oracle fetch\` first" >&2; exit 1; }

cc -O2 -g -Wall -Wextra -Wno-unused-parameter \
   -I "$here" \
   -o "$here/heap4_driver" \
   "$heap4" "$here/heap4_driver.c"

"$here/heap4_driver" > "$here/heap4.trace"
echo "wrote $(wc -l < "$here/heap4.trace") lines to $here/heap4.trace"
