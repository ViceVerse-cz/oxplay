// SPDX-License-Identifier: GPL-3.0-or-later
// Explicit native ABI/self-query check, never launched by the app or sampler.
#include <libproc.h>
#include <stddef.h>
#include <stdio.h>
#include <sys/resource.h>
#include <unistd.h>

_Static_assert(RUSAGE_INFO_V0 == 0, "unexpected fixed rusage flavor");
_Static_assert(sizeof(struct rusage_info_v0) == 96, "unexpected rusage v0 size");
_Static_assert(offsetof(struct rusage_info_v0, ri_phys_footprint) == 72, "footprint offset");
_Static_assert(offsetof(struct rusage_info_v0, ri_proc_start_abstime) == 80, "start offset");
_Static_assert(offsetof(struct rusage_info_v0, ri_proc_exit_abstime) == 88, "exit offset");

int main(void) {
    struct rusage_info_v0 usage = {0};
    if (proc_pid_rusage(getpid(), RUSAGE_INFO_V0, (rusage_info_t *)&usage) != 0 ||
        usage.ri_phys_footprint == 0 || usage.ri_proc_start_abstime == 0 ||
        usage.ri_proc_exit_abstime != 0) {
        fputs("native footprint self-query failed\n", stderr);
        return 1;
    }
    printf("{\"structure_bytes\":96,\"footprint_offset\":72,\"start_offset\":80,"
           "\"exit_offset\":88,\"self_query_live\":true,\"self_footprint_bytes\":%llu}\n",
           (unsigned long long)usage.ri_phys_footprint);
    return 0;
}
