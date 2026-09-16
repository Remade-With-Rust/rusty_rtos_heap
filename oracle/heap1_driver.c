/* The C arm of K4's heap_1 differential: FreeRTOS's own heap_1, driven.
 *
 * `heap_1.c` is compiled VERBATIM from the pinned kernel checkout -- not
 * copied into this tree, not edited. Only its includes are supplied, by the
 * shims beside this file, which `heap4_driver.c` already needed.
 *
 * The workload is deliberately NOT the heap_4 one. heap_4's exercises
 * fragmentation: allocate, free, coalesce, refuse when the free list has no
 * block big enough. heap_1 cannot free, so every one of those branches is
 * absent and a workload that leans on them would prove nothing. The only
 * interesting branch a bump allocator has is REFUSAL, so this workload runs
 * the arena to exhaustion and keeps asking afterwards.
 *
 * That is the same lesson the heap_4 differential paid for: its first
 * workload never refused a single request, so what agreed was the easy half
 * of an allocator. Here refusal is the whole of the hard half.
 *
 * Every printed quantity is base-independent. An offset into a known aligned
 * base is comparable across two programs; a pointer is not, which is why
 * `configAPPLICATION_ALLOCATED_HEAP` is set and `ucHeap` is ours.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "FreeRTOS.h"
#include "task.h"

/* heap_1.c declares this `extern` under configAPPLICATION_ALLOCATED_HEAP.
 * Aligned to portBYTE_ALIGNMENT so the allocator's own alignment step loses
 * nothing and offset 0 IS the first byte it hands out. */
uint8_t ucHeap[ configTOTAL_HEAP_SIZE ] __attribute__( ( aligned( portBYTE_ALIGNMENT ) ) );

extern void * pvPortMalloc( size_t xWantedSize );
extern size_t xPortGetFreeHeapSize( void );

void kairos_heap4_assert_failed( const char * pcWhat )
{
    fprintf( stderr, "configASSERT failed: %s\n", pcWhat );
    _Exit( 2 );
}

/* The largest single request, and how many operations to run.
 *
 * With an 8 KiB arena and requests up to 600 bytes, exhaustion arrives after
 * roughly thirty allocations -- so the great majority of these 2,000
 * operations are REFUSALS, which is exactly the branch this differential is
 * for. The Rust side asserts that the refusal count is substantial rather
 * than trusting the shape of the arithmetic.
 */
#define MAX_SIZE 600u
#define OPS      2000u

/* The driver's LCG, written out in full so the Rust side can reproduce it.
 * `rand()` is implementation-defined and would make the two arms
 * incomparable by construction. */
static uint32_t ulSeed = 12345u;

static uint32_t prvNext( void )
{
    ulSeed = ( ulSeed * 1664525u ) + 1013904223u;
    return ulSeed;
}

int main( void )
{
    /* The geometry is a premise, not decoration: if the C is modelling a
     * different heap then every later comparison is meaningless. `adjusted`
     * is the quantity heap_1 actually bounds against, and it is
     * configTOTAL_HEAP_SIZE - portBYTE_ALIGNMENT rather than the total. */
    printf( "geometry total=%u align=%u adjusted=%u\n",
            (unsigned) configTOTAL_HEAP_SIZE,
            (unsigned) portBYTE_ALIGNMENT,
            (unsigned) ( configTOTAL_HEAP_SIZE - portBYTE_ALIGNMENT ) );

    /* The base the offsets are measured from. heap_1 aligns the start of
     * ucHeap itself; ucHeap is already aligned above, so this is ucHeap --
     * but it is computed the way heap_1 computes it rather than assumed. */
    uint8_t * pucBase = ( uint8_t * ) ( ( ( uintptr_t ) &( ucHeap[ portBYTE_ALIGNMENT - 1 ] ) ) &
                                        ( ~( ( uintptr_t ) ( portBYTE_ALIGNMENT - 1 ) ) ) );

    unsigned long ulRefusals = 0;

    for( unsigned long ulOp = 0; ulOp < OPS; ulOp++ )
    {
        uint32_t ulR = prvNext();
        size_t xSize = (size_t) ( 1u + ( ulR % MAX_SIZE ) );

        void * pvGot = pvPortMalloc( xSize );
        long lOffset;

        if( pvGot == NULL )
        {
            lOffset = -1;
            ulRefusals++;
        }
        else
        {
            lOffset = (long) ( ( uint8_t * ) pvGot - pucBase );
        }

        printf( "alloc %lu %ld %u\n",
                (unsigned long) xSize,
                lOffset,
                (unsigned) xPortGetFreeHeapSize() );

        /* vPortFree is NOT called here, and that is a finding rather than an
         * omission.
         *
         * The first draft of this driver freed every eighth allocation to
         * show that freeing does nothing. The C arm exited 2 on the first
         * one: heap_1's vPortFree is not a no-op. Its body is
         *
         *     configASSERT( pv == NULL );
         *
         * under the comment "Force an assert as it is invalid to call this
         * function" -- so calling it with a live pointer is a programming
         * error the C refuses to let pass. The Rust side answers
         * Error::Unsupported for the same reason and a test there asserts
         * it, so there is nothing left for this trace to carry.
         */
    }

    printf( "end refusals=%lu free=%u\n",
            ulRefusals,
            (unsigned) xPortGetFreeHeapSize() );
    return 0;
}
