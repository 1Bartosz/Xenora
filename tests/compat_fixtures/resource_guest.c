// SPDX-License-Identifier: GPL-3.0-or-later
struct request {
    unsigned opcode, handle, offset, length, kind;
};
_Static_assert(sizeof(struct request) == 20, "synthetic descriptor size");

__declspec(dllimport) unsigned xenora_resource(const volatile struct request *, const void *);
__declspec(dllimport) unsigned xenora_report(const unsigned *, unsigned);

// Keep each import call in its own function so the compiler emits an IAT
// call at the boundary instead of caching the thunk in a register.
__attribute__((noinline))
static unsigned resource_call(const volatile struct request *args, const void *data) {
    return xenora_resource(args, data);
}

__attribute__((noinline))
static unsigned report_call(const unsigned *words) {
    return xenora_report(words, 3);
}

const unsigned initial_words[] = {0xfeedface, 2, 5, 11, 0xdeadbeef};
const unsigned update_words[] = {17, 19, 23};
const unsigned other_words[] = {0x12345678, 27, 31, 41, 0xa1b2c3d4};
const float vertex_words[] = {
    -1.0f, -1.0f, 0.5f, 1.0f,  3.0f, -1.0f, 0.5f, 1.0f,  -1.0f, 3.0f, 0.5f, 1.0f,
     0.0f,  0.0f, 1.0f, 1.0f,  0.0f,  1.0f, 1.0f, 1.0f,   1.0f, 0.0f, 1.0f, 1.0f,
};

struct bc1_source {
    unsigned width, height, mips, layers, format, usage, row_pitch, slice_pitch;
    unsigned blocks[32];
};
_Static_assert(sizeof(struct bc1_source) == 160, "synthetic BC1 upload size");

const struct bc1_source texture_source = {
    5, 3, 2, 2, 1, CASE == 13 ? 2 : 1, 24, 32,
    {
        [0] = 0xf800, // Layer 0, mip 0, first block is red.
        [8] = 0xf800, // Layer 0, mip 1.
        [16] = 0x07e0, [18] = 0x001f, // Layer 1, mip 0, green then blue.
        [20] = 0xf800, // Padding in the block row.
        [24] = 0xffff, // Layer 1, mip 1, white.
    },
};

// The fixture is fixed-base. These LE-address materializations keep the C
// source within the interpreter's supported integer instruction subset.
#define ADDRESS(NAME) ({     const void *arg;     __asm__ volatile("movabsq $" #NAME ", %0" : "=d"(arg));     arg; })
#define NO_DATA ({     const void *arg;     __asm__ volatile("movl $0, %k0" : "=d"(arg));     arg; })
#define CALL(OP, HANDLE, OFFSET, LENGTH, KIND, DATA) ({     args.opcode = (OP);     args.handle = (HANDLE);     args.offset = (OFFSET);     args.length = (LENGTH);     args.kind = (KIND);     resource_call(&args, (DATA)); })

unsigned guest_entry(void) {
    volatile struct request args;
#if CASE == 1 || CASE == 2 || CASE == 10 || CASE == 18
    unsigned pixels[12];
    unsigned resource = CALL(1, 0, 0, 96, 2, ADDRESS(vertex_words));
#if CASE == 10
    CALL(6, resource, 0, 92, 1, NO_DATA);
    return 90;
#else
    CALL(6, resource, 0, 96, CASE == 1 ? 1 : 2, NO_DATA);
    CALL(5, 0, 0, 48, 0, pixels);
    // The guest independently reads the four groups through its report import.
    report_call(&pixels[0]);
    report_call(&pixels[3]);
    report_call(&pixels[6]);
    const volatile unsigned *actual = pixels;
#if CASE == 1
#define PIXEL(index, expected) (actual[index] == 0xff0000ff)
#else
#define PIXEL(index, expected) (actual[index] == (expected))
#endif
    // These are guest loads from the actual return buffer. Each expected
    // varying word uses the analytic red row and green column values.
    if (!PIXEL(0, 0xffff106a) || !PIXEL(1, 0xffff306a) ||
        !PIXEL(2, 0xffff506a) || !PIXEL(3, 0xffff706a) ||
        !PIXEL(4, 0xffff1040) || !PIXEL(5, CASE == 18 ? 0xffff3041 : 0xffff3040) ||
        !PIXEL(6, 0xffff5040) || !PIXEL(7, 0xffff7040) ||
        !PIXEL(8, 0xffff1015) || !PIXEL(9, 0xffff3015) ||
        !PIXEL(10, 0xffff5015) || !PIXEL(11, 0xffff7015))
        return 93;
#undef PIXEL
    return report_call(&pixels[9]);
#endif
#elif CASE >= 12
    unsigned resource = CALL(7, 0, 0, CASE == 17 ? 120 : 128, 0, ADDRESS(texture_source));
#if CASE == 13 || CASE == 17
    return 90;
#elif CASE == 14
    CALL(8, resource, 2, 0, 0, NO_DATA);
    return 90;
#elif CASE == 15
    CALL(2, resource, 0, 4, 0, ADDRESS(update_words));
    return 90;
#elif CASE == 16
    unsigned pixels[3];
    __asm__ volatile("movl $0x11223344, %0" : "=m"(pixels[2]));
    CALL(8, resource, 1, 1, 0, NO_DATA);
    CALL(5, 0, 0, 8, 0, pixels);
    return report_call(pixels);
#else
    unsigned pixels[15];
    CALL(8, resource, 1, 0, 0, NO_DATA);
    CALL(5, 0, 0, 60, 0, pixels);
    report_call(&pixels[0]);
    report_call(&pixels[3]);
    report_call(&pixels[6]);
    report_call(&pixels[9]);
    return report_call(&pixels[12]);
#endif
#else
    unsigned words[3];
    unsigned resource = CALL(1, 0, 0, 20, 1, ADDRESS(initial_words));
#if CASE == 3
    CALL(3, resource, 0, 0, 0, NO_DATA);
    CALL(1, 0, 0, 20, 1, ADDRESS(other_words));
    CALL(4, resource, 4, 12, 0, NO_DATA);
#elif CASE == 4
    CALL(4, 0, 4, 12, 0, NO_DATA);
#elif CASE == 5
    CALL(4, resource, 12, 12, 0, NO_DATA);
#elif CASE == 6
    CALL(4, resource, 0xfffffffc, 12, 0, NO_DATA);
#elif CASE == 7
    CALL(4, resource, 4, 12, 0, NO_DATA);
    CALL(5, 0, 0, 12, 0, ADDRESS(initial_words));
#elif CASE == 8
    CALL(4, resource, 4, 12, 0, NO_DATA);
    CALL(4, resource, 4, 12, 0, NO_DATA);
#elif CASE == 9
    // A queued snapshot left at guest return is dropped without GPU work.
    CALL(4, resource, 4, 12, 0, NO_DATA);
    return 73;
#elif CASE == 11
    CALL(5, 0, 0, 12, 0, words);
#else
    CALL(4, resource, 4, 12, 0, NO_DATA);
    CALL(2, resource, 4, 12, 0, ADDRESS(update_words));
    CALL(5, 0, 0, 12, 0, words);
    report_call(words);

    CALL(4, resource, 4, 12, 0, NO_DATA);
    CALL(5, 0, 0, 12, 0, words);
    report_call(words);

    CALL(4, resource, 4, 12, 0, NO_DATA);
    CALL(3, resource, 0, 0, 0, NO_DATA);
    unsigned replacement = CALL(1, 0, 0, 20, 1, ADDRESS(other_words));
    CALL(5, 0, 0, 12, 0, words);
    report_call(words);

    CALL(4, replacement, 4, 12, 0, NO_DATA);
    CALL(5, 0, 0, 12, 0, words);
    CALL(3, replacement, 0, 0, 0, NO_DATA);
    return report_call(words);
#endif
    return 91;
#endif
}
