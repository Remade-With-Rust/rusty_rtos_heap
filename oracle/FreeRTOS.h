/* A shim, not FreeRTOS.
 *
 * heap_4.c includes "FreeRTOS.h" and "task.h" for a handful of typedefs
 * and macros. Pulling in the real headers would drag in a port -- a
 * portmacro.h, a scheduler, an interrupt model -- none of which an
 * allocator benchmark uses, and all of which would be code the C arm
 * carries and the Rust arm does not. So this defines exactly what
 * heap_4.c reads and nothing else, and every definition is listed here
 * so a reader can check it against the real one.
 */
#ifndef INC_FREERTOS_H
#define INC_FREERTOS_H

#include <stddef.h>
#include <stdint.h>

#include "FreeRTOSConfig.h"

typedef int      BaseType_t;
typedef uint32_t TickType_t;

#define portBYTE_ALIGNMENT_MASK  ( portBYTE_ALIGNMENT - 1 )
#define portPOINTER_SIZE_TYPE    uintptr_t
#define portMAX_DELAY            ( ( TickType_t ) 0xffffffffUL )
#define pdFALSE                  ( ( BaseType_t ) 0 )
#define pdTRUE                   ( ( BaseType_t ) 1 )

#define PRIVILEGED_DATA
#define PRIVILEGED_FUNCTION
#define portMEMORY_BARRIER()     __asm volatile ( "" ::: "memory" )

/* An assertion that TRAPS. A no-op `configASSERT` would let heap_4
 * violate its own invariants silently and still produce a cycle number,
 * which is the shape of a benchmark that measures nothing. */
extern void kairos_heap4_assert_failed( const char * pcWhat );
#define configASSERT( x )        if( ( x ) == 0 ) { kairos_heap4_assert_failed( #x ); }

#define mtCOVERAGE_TEST_MARKER()
#define traceMALLOC( pvReturn, xWantedSize )
#define traceFREE( pv, xBlockSize )

/* `HeapRegion_t` lives in `portable.h` in the real kernel, which these shims
 * do not provide. heap_5.c needs it and heap_4.c does not, which is why it
 * arrives here only now: the shims grow when a file being compiled verbatim
 * asks for something, never ahead of that. */
typedef struct HeapRegion
{
    uint8_t * pucStartAddress;
    size_t    xSizeInBytes;
} HeapRegion_t;

typedef struct xHeapStats
{
    size_t xAvailableHeapSpaceInBytes;
    size_t xSizeOfLargestFreeBlockInBytes;
    size_t xSizeOfSmallestFreeBlockInBytes;
    size_t xNumberOfFreeBlocks;
    size_t xMinimumEverFreeBytesRemaining;
    size_t xNumberOfSuccessfulAllocations;
    size_t xNumberOfSuccessfulFrees;
} HeapStats_t;

#endif /* INC_FREERTOS_H */
