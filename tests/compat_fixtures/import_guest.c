// SPDX-License-Identifier: GPL-3.0-or-later
// Original synthetic guest. The entry point returns its exit status directly.
__declspec(dllimport) unsigned xenora_transform(unsigned *words, unsigned count);
#if UNRESOLVED_IMPORT
__declspec(dllimport) unsigned xenora_missing(unsigned *words, unsigned count);
#define REPORT xenora_missing
#else
__declspec(dllimport) unsigned xenora_report(unsigned *words, unsigned count);
#define REPORT xenora_report
#endif

unsigned guest_entry(void) {
    unsigned words[3] = {2, 5, 11};
    if (xenora_transform(words, 3) != 3)
        return 91;
    return REPORT(words, 3);
}
