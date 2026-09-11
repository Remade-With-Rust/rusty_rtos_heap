/* The C arm of K4's differential: FreeRTOS's own heap_4, driven.
 *
 * `heap_4.c` is compiled VERBATIM from the pinned kernel checkout — not
 * copied into this tree, not edited. Only its includes are supplied, by
 * the three shims beside this file. What this driver adds is a
 * deterministic request sequence and a line per operation, so the Rust
 * remake can be diffed against it.
 *
 * The sequence must be reproducible in another language, so the generator
 * is a plain LCG written out in full rather than anything from libc:
 * `rand()` is implementation-defined and would make the two arms
 * incomparable by construction.
 *
 * Every printed quantity is base-independent. An offset into a known
 * aligned base is comparable across two programs; a pointer is not, which
 * is why `configAPPLICATION_ALLOCATED_HEAP` is set and `ucHeap` is ours.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "FreeRTOS.h"
#include "task.h"

/* heap_4.c declares this `extern` under configAPPLICATION_ALLOCATED_HEAP.
 * Aligned to portBYTE_ALIGNMENT so prvHeapInit loses nothing to the
 * alignment step and offset 0 IS the first block. */
uint8_t ucHeap[ configTOTAL_HEAP_SIZE ] __attribute__( ( aligned( portBYTE_ALIGNMENT ) ) );

extern void * pvPortMalloc( size_t xWantedSize );
extern void vPortFree( void * pv );
extern size_t xPortGetFreeHeapSize( void );
extern size_t xPortGetMinimumEverFreeHeapSize( void );

void kairos_heap4_assert_failed( const char * pcWhat )
{
    fprintf( stderr, "configASSERT failed: %s\n", pcWhat );
    /* Loud, and fatal: an assert that prints and continues would let the
     * differential keep comparing a heap that has already gone wrong. */
    _Exit( 2 );
}

/* How many slots the workload keeps live at once, and the largest single
 * request. These two set whether the arena is ever actually FULL, and the
 * first pair chosen (32 slots, 300 bytes) never was: occupancy hovers at
 * half the slots, so the steady state was ~2.6 KB of 8 KB and not one
 * request was ever refused. Agreement on a workload that never fails is
 * agreement about the easy half of an allocator. `the_workload_reaches_
 * the_branches_that_matter` is the test that caught it, and these numbers
 * are what it takes to drive the arena to exhaustion. */
#define SLOTS       48U
/* The largest single request. Chosen so several fit but the arena still
 * fragments and refuses. */
#define MAX_SIZE    600U
/* Operations. Long enough that every branch is reached many times. */
#define OPS         20000U

static void * pvSlots[ SLOTS ];

/* An LCG, written out so the Rust can be the same one. The constants are
 * Numerical Recipes'. */
static uint32_t ulSeed = 12345U;

static uint32_t prvNextRand( void )
{
    ulSeed = ( ulSeed * 1664525U ) + 1013904223U;
    return ulSeed;
}

int main( void )
{
    uint32_t ulOp;

    /* The geometry first, so the Rust side asserts it is modelling the
     * same heap rather than assuming it. */
    printf( "geometry total=%lu align=%d struct=%lu minblock=%lu\n",
            ( unsigned long ) configTOTAL_HEAP_SIZE,
            portBYTE_ALIGNMENT,
            ( unsigned long ) ( ( sizeof( void * ) + sizeof( size_t ) + ( portBYTE_ALIGNMENT - 1 ) ) & ~( ( size_t ) ( portBYTE_ALIGNMENT - 1 ) ) ),
            ( unsigned long ) ( ( ( ( sizeof( void * ) + sizeof( size_t ) + ( portBYTE_ALIGNMENT - 1 ) ) & ~( ( size_t ) ( portBYTE_ALIGNMENT - 1 ) ) ) ) << 1 ) );

    for( ulOp = 0U; ulOp < OPS; ulOp++ )
    {
        uint32_t ulR = prvNextRand();
        uint32_t ulSlot = ulR % SLOTS;

        if( pvSlots[ ulSlot ] != NULL )
        {
            vPortFree( pvSlots[ ulSlot ] );
            pvSlots[ ulSlot ] = NULL;
            printf( "free %lu -1 %lu %lu\n",
                    ( unsigned long ) ulSlot,
                    ( unsigned long ) xPortGetFreeHeapSize(),
                    ( unsigned long ) xPortGetMinimumEverFreeHeapSize() );
        }
        else
        {
            size_t xSize = ( size_t ) ( 1U + ( ( ulR / SLOTS ) % MAX_SIZE ) );
            void * pv = pvPortMalloc( xSize );
            long lOffset = -1;

            pvSlots[ ulSlot ] = pv;

            if( pv != NULL )
            {
                lOffset = ( long ) ( ( uint8_t * ) pv - ucHeap );
            }

            printf( "alloc %lu %lu %ld %lu %lu\n",
                    ( unsigned long ) ulSlot,
                    ( unsigned long ) xSize,
                    lOffset,
                    ( unsigned long ) xPortGetFreeHeapSize(),
                    ( unsigned long ) xPortGetMinimumEverFreeHeapSize() );
        }
    }

    printf( "end free=%lu minfree=%lu\n",
            ( unsigned long ) xPortGetFreeHeapSize(),
            ( unsigned long ) xPortGetMinimumEverFreeHeapSize() );
    return 0;
}
