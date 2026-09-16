#!/bin/sh
# Generate the C arm of K4's heap_5 differential.
#
# heap_5.c is compiled VERBATIM out of the pinned kernel checkout in the
# umbrella; this script never copies or edits it. The trace it writes is
# checked in, so the Rust side can be diffed in CI with no C toolchain --
# the same arrangement `run.sh` uses for heap_4 and the kernel corpus uses
# for `oracle/traces/*`.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
kernel="$here/../../oracle/FreeRTOS-Kernel"
heap5="$kernel/portable/MemMang/heap_5.c"

[ -f "$heap5" ] || { echo "no heap_5.c at $heap5 -- run \`kairos oracle fetch\` first" >&2; exit 1; }

cc -O2 -g -Wall -Wextra -Wno-unused-parameter \
   -I "$here" \
   -o "$here/heap5_driver" \
   "$heap5" "$here/heap5_driver.c"

"$here/heap5_driver" > "$here/heap5.trace"
echo "wrote $(wc -l < "$here/heap5.trace") lines to $here/heap5.trace"
