/* The C arm of K4's heap_5 differential: FreeRTOS's own heap_5, driven.
 *
 * `heap_5.c` is compiled VERBATIM from the pinned kernel checkout -- not
 * copied into this tree, not edited. Only its includes are supplied, by the
 * shims beside this file.
 *
 * heap_5 IS heap_4 with a different initialiser: `pvPortMalloc`, `vPortFree`
 * and `prvInsertBlockIntoFreeList` are the same code. So this workload is
 * heap_4's -- allocate, free, fragment, refuse -- and what it adds is the one
 * thing heap_5 has that heap_4 does not: THREE REGIONS WITH GAPS BETWEEN THEM.
 *
 * The gaps are the point. Coalescing is an address comparison, so what stops
 * two regions merging into one is that the first one's end is not the second
 * one's start. A transcription that quietly treated the arena as contiguous
 * would hand out a block spanning a gap, and the offsets printed here are what
 * catches it.
 *
 * Every printed quantity is base-independent: offsets from the arena's base,
 * never pointers.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "FreeRTOS.h"
#include "task.h"

/* The three regions live inside ONE array so the Rust side can describe them
 * as offsets into one arena. The gaps between them are real: nothing is
 * allocated there by either arm, and neither may coalesce across them. */
#define ARENA_BYTES 12288u

uint8_t ucArena[ ARENA_BYTES ] __attribute__( ( aligned( portBYTE_ALIGNMENT ) ) );

/* start, size -- deliberately different sizes, with gaps, in increasing
 * address order because heap_5 ASSERTS that order rather than sorting:
 *
 *     /* Check blocks are passed in with increasing start addresses. *\/
 *     configASSERT( ( size_t ) xAddress > ( size_t ) pxEnd );
 */
#define R0_START 0u
#define R0_SIZE  4096u
#define R1_START 4608u      /* a 512-byte gap after region 0 */
#define R1_SIZE  2048u
#define R2_START 7168u      /* a 512-byte gap after region 1 */
#define R2_SIZE  4096u

extern void * pvPortMalloc( size_t xWantedSize );
extern void vPortFree( void * pv );
extern size_t xPortGetFreeHeapSize( void );
extern size_t xPortGetMinimumEverFreeHeapSize( void );
extern void vPortDefineHeapRegions( const HeapRegion_t * const pxHeapRegions );

void kairos_heap4_assert_failed( const char * pcWhat )
{
    fprintf( stderr, "configASSERT failed: %s\n", pcWhat );
    _Exit( 2 );
}

/* The workload shape. Sized so the three regions are genuinely pressured:
 * requests up to 600 bytes against a 4 KiB, a 2 KiB and a 4 KiB region means
 * a request can fit in region 0 or 2 and not in what is left of region 1,
 * which is the case a single-region heap cannot produce. */
#define SLOTS    48u
#define MAX_SIZE 600u
#define OPS      20000u

static uint32_t ulSeed = 12345u;

static uint32_t prvNext( void )
{
    ulSeed = ( ulSeed * 1664525u ) + 1013904223u;
    return ulSeed;
}

int main( void )
{
    HeapRegion_t xRegions[] =
    {
        { &( ucArena[ R0_START ] ), R0_SIZE },
        { &( ucArena[ R1_START ] ), R1_SIZE },
        { &( ucArena[ R2_START ] ), R2_SIZE },
        { NULL,                     0       }
    };

    vPortDefineHeapRegions( xRegions );

    printf( "geometry arena=%u align=%u struct=%u regions=3\n",
            (unsigned) ARENA_BYTES,
            (unsigned) portBYTE_ALIGNMENT,
            (unsigned) ( sizeof( size_t ) * 2 ) );
    printf( "region 0 %u %u\n", R0_START, R0_SIZE );
    printf( "region 1 %u %u\n", R1_START, R1_SIZE );
    printf( "region 2 %u %u\n", R2_START, R2_SIZE );
    printf( "defined %u %u\n",
            (unsigned) xPortGetFreeHeapSize(),
            (unsigned) xPortGetMinimumEverFreeHeapSize() );

    void * pvSlots[ SLOTS ];
    memset( pvSlots, 0, sizeof( pvSlots ) );

    for( unsigned long ulOp = 0; ulOp < OPS; ulOp++ )
    {
        uint32_t ulR = prvNext();
        unsigned uSlot = (unsigned) ( ulR % SLOTS );

        if( pvSlots[ uSlot ] != NULL )
        {
            vPortFree( pvSlots[ uSlot ] );
            pvSlots[ uSlot ] = NULL;
            printf( "free %u -1 %u %u\n",
                    uSlot,
                    (unsigned) xPortGetFreeHeapSize(),
                    (unsigned) xPortGetMinimumEverFreeHeapSize() );
        }
        else
        {
            size_t xSize = (size_t) ( 1u + ( ( ulR / SLOTS ) % MAX_SIZE ) );
            void * pvGot = pvPortMalloc( xSize );
            long lOffset;

            /* The C stores the result even when it is NULL, so a refused
             * request leaves the slot empty and the next visit allocates
             * again. Anything else would desynchronise the sequences. */
            pvSlots[ uSlot ] = pvGot;
            lOffset = ( pvGot == NULL ) ? -1 : (long) ( ( uint8_t * ) pvGot - ucArena );

            printf( "alloc %u %u %ld %u %u\n",
                    uSlot,
                    (unsigned) xSize,
                    lOffset,
                    (unsigned) xPortGetFreeHeapSize(),
                    (unsigned) xPortGetMinimumEverFreeHeapSize() );
        }
    }

    printf( "end %u %u\n",
            (unsigned) xPortGetFreeHeapSize(),
            (unsigned) xPortGetMinimumEverFreeHeapSize() );
    return 0;
}
