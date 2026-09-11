/* The minimum configuration `heap_4.c` reads, for the differential driver.
 *
 * `configAPPLICATION_ALLOCATED_HEAP` is the point: it makes `ucHeap` the
 * APPLICATION's array rather than a static inside heap_4.c, so the driver
 * controls its alignment and knows its base address. Offsets into a known
 * base are comparable across two programs; pointers are not.
 */
#ifndef FREERTOS_CONFIG_H
#define FREERTOS_CONFIG_H

#define configTOTAL_HEAP_SIZE            ( ( size_t ) 8192 )
#define portBYTE_ALIGNMENT               8
#define configAPPLICATION_ALLOCATED_HEAP 1
#define configUSE_MALLOC_FAILED_HOOK     0
#define configENABLE_HEAP_PROTECTOR      0
#define configSUPPORT_DYNAMIC_ALLOCATION 1
#define configHEAP_CLEAR_MEMORY_ON_FREE  0

#endif /* FREERTOS_CONFIG_H */
