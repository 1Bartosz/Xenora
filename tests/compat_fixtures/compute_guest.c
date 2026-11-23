// SPDX-License-Identifier: GPL-3.0-or-later
struct descriptor {
    unsigned command, handle, binding, count;
};
_Static_assert(sizeof(struct descriptor) == 16, "synthetic descriptor size");

__declspec(dllimport) unsigned xenora_compute(unsigned *, const volatile struct descriptor *);
__declspec(dllimport) unsigned xenora_report(unsigned *, unsigned);

unsigned guest_entry(void) {
    volatile struct descriptor request;
    request.command = CASE == 1 ? 2 : 1;
    request.handle = CASE == 2 ? 1 : 0;
    request.binding = CASE == 3 ? 1 : 0;
    request.count = CASE == 4 ? 4 : 3;
    unsigned words[3] = {2, 5, 11};
#if CASE == 5
    unsigned *invalid;
    // Materialize an aligned unmapped address using the guest's MOV subset.
    __asm__ volatile("movl $4, %k0" : "=c"(invalid));
    if (xenora_compute(invalid, &request) != 3)
#else
    if (xenora_compute(words, &request) != 3)
#endif
        return 91;
#if CASE == 6
    if (xenora_compute(words, &request) != 3)
        return 92;
#endif
    return xenora_report(words, 3);
}
