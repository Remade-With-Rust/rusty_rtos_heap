/* A shim, not task.h. See FreeRTOS.h beside it.
 *
 * heap_4.c brackets its critical work in vTaskSuspendAll() /
 * xTaskResumeAll(). There is no scheduler here to suspend -- and, more to
 * the point, the Rust arm is built `--cfg ra_single_threaded`, so it takes
 * no lock either. Making these no-ops is what keeps the two arms
 * SYMMETRIC: a C arm paying for a scheduler lock that the Rust arm does
 * not pay for would be measuring the lock, not the allocator.
 */
#ifndef INC_TASK_H
#define INC_TASK_H

#include "FreeRTOS.h"

static inline void       vTaskSuspendAll( void ) { }
static inline BaseType_t xTaskResumeAll( void )  { return pdFALSE; }

/* vPortGetHeapStats() takes a real critical section rather than the
 * scheduler lock. Same reasoning, and it is off the measured path anyway:
 * the harness calls it only to read what heap_4 CHARGED for a request. */
#define taskENTER_CRITICAL()    do { } while( 0 )
#define taskEXIT_CRITICAL()     do { } while( 0 )

#endif /* INC_TASK_H */
