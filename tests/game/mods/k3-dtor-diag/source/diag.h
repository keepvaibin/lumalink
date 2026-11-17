/* k3-refs diagnosis: which destructors a stage change reaches through hooks, and on which thread. */
#ifndef K3_DIAG_H
#define K3_DIAG_H
#include <stdint.h>

int32_t diag_start(void);
int32_t diag_count(int32_t which);

#endif
