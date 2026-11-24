#!/usr/bin/env python3
"""Reference vectors for SSE and SSE2 floating point, from exact fractions.

  generate   write float_vectors.txt next to this file
  --check    recompute in memory and fail if the committed file differs

Every finite result is computed as an exact `fractions.Fraction` (square roots by
integer comparison) and then rounded by definition, so nothing here shares code
or method with crates/loader/src/softfloat.rs, and no host floating point is
used. Rules: Intel SDM Vol. 1, 4.8, 4.9 and 11.5, and the Vol. 2 operation
sections. Cases whose flags those sections leave open are not generated:
a denormal dividend over zero, flushing an exact denormal result, and the
precision flag of a denormal converted to an integer under DAZ.

Line: <operation> <mxcsr> <a> <b> <result> <mxcsr flags>, all values hex.
"""

import random
import sys
from fractions import Fraction
from math import isqrt
from pathlib import Path

OUT = Path(__file__).resolve().parent / "float_vectors.txt"
IE, DE, ZE, OE, UE, PE = 1, 2, 4, 8, 16, 32
NEAREST, DOWN, UP, ZERO = range(4)


class Fmt:
    def __init__(self, bits, frac):
        self.bits, self.frac = bits, frac
        self.ebits = bits - 1 - frac
        self.emax = 2 ** (self.ebits - 1) - 1
        self.emin = 1 - self.emax
        self.inf = (2 ** self.ebits - 1) << frac
        self.quiet = 1 << (frac - 1)
        self.top = 1 << (bits - 1)

    def mag(self, v):
        return v % self.top

    def neg(self, v):
        return v >= self.top

    def nan(self, v):
        return self.mag(v) > self.inf

    def snan(self, v):
        return self.nan(v) and not v & self.quiet

    def isinf(self, v):
        return self.mag(v) == self.inf

    def denormal(self, v):
        return 0 < self.mag(v) < (1 << self.frac)

    def value(self, v):
        m = self.mag(v)
        field, fraction = m >> self.frac, m % (1 << self.frac)
        if field == 0:
            x = Fraction(fraction) * Fraction(2) ** (self.emin - self.frac)
        else:
            x = (1 + Fraction(fraction, 1 << self.frac)) * Fraction(2) ** (field - self.emax)
        return -x if self.neg(v) else x

    def encode(self, sign, x):
        """Bits of an exactly representable non-negative x."""
        if x == 0:
            m = 0
        elif x < Fraction(2) ** self.emin:
            m = int(x / Fraction(2) ** (self.emin - self.frac))
        else:
            e = exponent(x)
            m = ((e + self.emax) << self.frac) + int((x / Fraction(2) ** e - 1) * (1 << self.frac))
        return m + (self.top if sign else 0)


S, D = Fmt(32, 23), Fmt(64, 52)


def exponent(x):
    """floor(log2 x) for x > 0."""
    e = x.numerator.bit_length() - x.denominator.bit_length()
    return e if Fraction(2) ** e <= x else e - 1


def pick(below, above, position, sign, mode, even):
    """Choose between neighbours; position is -1, 0 or 1 against the midpoint,
    or None when exactly `below`."""
    if position is None:
        return below
    if mode == NEAREST:
        if position == 0:
            return below if even else above
        return above if position > 0 else below
    if mode == ZERO:
        return below
    return above if (mode == UP) != sign else below


def quantize(n, rest_sign, quantum, sign, mode):
    """n quanta below the value; rest_sign as in pick."""
    return pick(n * quantum, (n + 1) * quantum, rest_sign, sign, mode, n % 2 == 0)


def locate(x, quantum):
    """Quanta below the positive fraction x, and its side of the midpoint."""
    q = x / quantum
    n = q.numerator // q.denominator
    rest = q - n
    if rest == 0:
        return n, None
    half = Fraction(1, 2)
    return n, (rest > half) - (rest < half)


def locate_root(x, quantum):
    """As locate, for the square root of x."""
    q = x / (quantum * quantum)
    n = isqrt(q.numerator // q.denominator)
    if n * n == q:
        return n, None
    middle = (Fraction(n) + Fraction(1, 2)) ** 2
    return n, (q > middle) - (q < middle)


def finish(fmt, sign, e, locator, control):
    """Round a positive exact value with floor(log2) = e, given its locator."""
    mode, ftz, flags = control["mode"], control["ftz"], 0
    n, side = locator(Fraction(2) ** (e - fmt.frac))
    unbounded = quantize(n, side, Fraction(2) ** (e - fmt.frac), sign, mode)
    tiny = unbounded < Fraction(2) ** fmt.emin
    quantum = Fraction(2) ** (max(e, fmt.emin) - fmt.frac)
    n, side = locator(quantum)
    result = quantize(n, side, quantum, sign, mode)
    if tiny and ftz:
        if side is None:
            return None  # flushing an exact denormal is left open
        return fmt.encode(sign, Fraction(0)), UE | PE
    if side is not None:
        flags |= PE | (UE if tiny else 0)
    if result >= Fraction(2) ** (fmt.emax + 1):
        infinite = mode == NEAREST or (mode == UP and not sign) or (mode == DOWN and sign)
        return (fmt.inf if infinite else fmt.inf - 1) + (fmt.top if sign else 0), OE | PE
    return fmt.encode(sign, result), flags


def rounded(fmt, x, control):
    sign = x < 0
    x = abs(x)
    return finish(fmt, sign, exponent(x), lambda q: locate(x, q), control)


def default(fmt):
    return fmt.top + fmt.inf + fmt.quiet


def prepare(fmt, v, control):
    if control["daz"] and fmt.denormal(v):
        return fmt.top if fmt.neg(v) else 0
    return v


def binary(op, fmt, a, b, control):
    a, b = prepare(fmt, a, control), prepare(fmt, b, control)
    de = DE if fmt.denormal(a) or fmt.denormal(b) else 0
    if op in ("min", "max"):
        if fmt.nan(a) or fmt.nan(b):
            return b, IE
        order = compare(fmt, a, b)
        return (a if order == (-1 if op == "min" else 1) else b), de
    if fmt.nan(a) or fmt.nan(b):
        chosen = a if fmt.nan(a) else b
        return chosen | fmt.quiet, IE if fmt.snan(a) or fmt.snan(b) else 0
    if op == "sub":
        b ^= fmt.top
    sa, sb = fmt.neg(a), fmt.neg(b)
    sign = sa != sb
    signed = lambda m: m + (fmt.top if sign else 0)
    if op in ("add", "sub"):
        if fmt.isinf(a) and fmt.isinf(b) and sign:
            return default(fmt), IE
        if fmt.isinf(a):
            return a, de
        if fmt.isinf(b):
            return b, de
        x = fmt.value(a) + fmt.value(b)
        if x == 0:
            both = fmt.mag(a) == 0 and not sign
            return (fmt.top if (sa if both else control["mode"] == DOWN) else 0), de
    elif op == "mul":
        if fmt.isinf(a) or fmt.isinf(b):
            if fmt.mag(a) == 0 or fmt.mag(b) == 0:
                return default(fmt), IE
            return signed(fmt.inf), de
        x = fmt.value(a) * fmt.value(b)
        if x == 0:
            return signed(0), de
    else:
        if (fmt.isinf(a) and fmt.isinf(b)) or (fmt.mag(a) == 0 and fmt.mag(b) == 0):
            return default(fmt), IE
        if fmt.isinf(a):
            return signed(fmt.inf), de
        if fmt.isinf(b):
            return signed(0), de
        if fmt.mag(b) == 0:
            return None if de else (signed(fmt.inf), ZE)
        x = fmt.value(a) / fmt.value(b)
        if x == 0:
            return signed(0), de
    out = rounded(fmt, x, control)
    return out and (out[0], out[1] | de)


def root(fmt, v, control):
    v = prepare(fmt, v, control)
    if fmt.nan(v):
        return v | fmt.quiet, IE if fmt.snan(v) else 0
    if fmt.mag(v) == 0:
        return v, 0
    if fmt.neg(v):
        return default(fmt), IE
    if fmt.isinf(v):
        return v, 0
    x = fmt.value(v)
    bits, flags = finish(fmt, False, exponent(x) // 2, lambda q: locate_root(x, q), control)
    return bits, flags | (DE if fmt.denormal(v) else 0)


def compare(fmt, a, b):
    def key(v):
        if fmt.isinf(v):
            return Fraction(2) ** (fmt.emax + 2) * (-1 if fmt.neg(v) else 1)
        return fmt.value(v)
    return (key(a) > key(b)) - (key(a) < key(b))


def ordering(fmt, a, b, signaling, control):
    """(-1, 0, 1 or None for unordered), flags."""
    a, b = prepare(fmt, a, control), prepare(fmt, b, control)
    if fmt.nan(a) or fmt.nan(b):
        return None, IE if signaling or fmt.snan(a) or fmt.snan(b) else 0
    return compare(fmt, a, b), DE if fmt.denormal(a) or fmt.denormal(b) else 0


def predicate(code, fmt, a, b, control):
    order, flags = ordering(fmt, a, b, code in (1, 2, 5, 6), control)
    truth = [order == 0, order == -1, order in (-1, 0), order is None,
             order != 0, order != -1, order not in (-1, 0), order is not None][code]
    return (2 ** fmt.bits - 1 if truth else 0), flags


def comparison(fmt, a, b, signaling, control):
    """RFLAGS as ZF=4, PF=2, CF=1."""
    order, flags = ordering(fmt, a, b, signaling, control)
    return {None: 7, 1: 0, -1: 1, 0: 4}[order], flags


def widen(src, dst, v, control):
    v = prepare(src, v, control)
    sign = dst.top if src.neg(v) else 0
    if src.nan(v):
        payload = Fraction(src.mag(v) - src.inf) * Fraction(2) ** (dst.frac - src.frac)
        return sign + dst.inf + (int(payload) | dst.quiet), IE if src.snan(v) else 0
    if src.isinf(v):
        return sign + dst.inf, 0
    x = src.value(v)
    if x == 0:
        return sign, 0
    out = rounded(dst, x, control)
    return out and (out[0], out[1] | (DE if src.denormal(v) else 0))


def to_int(fmt, v, bits, truncate, control):
    if control["daz"] and fmt.denormal(v):
        return None
    indefinite = 2 ** (bits - 1), IE
    if fmt.nan(v) or fmt.isinf(v):
        return indefinite
    x = fmt.value(v)
    n, side = locate(abs(x), Fraction(1))
    r = int(quantize(n, side, Fraction(1), x < 0, ZERO if truncate else control["mode"]))
    r = -r if x < 0 else r
    if not -(2 ** (bits - 1)) <= r < 2 ** (bits - 1):
        return indefinite
    return r % 2 ** bits, 0 if side is None else PE


def from_int(fmt, v, bits, control):
    x = v - 2 ** bits if v >= 2 ** (bits - 1) else v
    return rounded(fmt, Fraction(x), control) if x else (0, 0)


def operands(fmt, rng):
    one = fmt.emax << fmt.frac
    least = 1 << fmt.frac
    edge = [0, 1, 2, least - 1, least, least + 1, 2 * least, one - 1, one, one + 1, one + least,
            one + fmt.quiet, one + (fmt.frac << fmt.frac), one + (fmt.frac << fmt.frac) + 1,
            fmt.inf - 1, fmt.inf - least, fmt.inf, fmt.inf + fmt.quiet, fmt.inf + fmt.quiet + 5,
            fmt.inf + 1, fmt.inf + fmt.quiet - 1]
    choice = rng.randrange(6)
    if choice < 2:
        v = rng.choice(edge)
    elif choice == 2:
        v = rng.randrange(least)  # denormal
    elif choice == 3:  # near one, few significant bits
        v = one + (rng.randrange(-40, 41) << fmt.frac) + (rng.randrange(16) << (fmt.frac - 4))
    elif choice == 4:  # near the smallest and largest normals
        v = rng.choice([least * rng.randrange(1, fmt.frac + 4), fmt.inf - least * rng.randrange(1, 4)]) + rng.randrange(least)
    else:
        v = rng.randrange(fmt.top)
    v = min(v, fmt.top - 1)
    return v + (fmt.top if rng.random() < 0.5 else 0)


def related(fmt, a, rng):
    """A second operand, often within a few exponents of the first."""
    if rng.random() < 0.5 or fmt.mag(a) >= fmt.inf:
        return operands(fmt, rng)
    m = fmt.mag(a) + (rng.randrange(-fmt.frac - 3, fmt.frac + 4) << fmt.frac) + rng.randrange(-3, 4)
    return max(0, min(m, fmt.inf)) + (fmt.top if rng.random() < 0.5 else 0)


def integers(bits, rng):
    c = rng.randrange(4)
    if c == 0:
        return rng.choice([0, 1, 2 ** (bits - 1), 2 ** (bits - 1) - 1, 2 ** bits - 1])
    if c == 1:
        return (2 ** rng.randrange(bits) + rng.randrange(-2, 3)) % 2 ** bits
    return rng.randrange(2 ** bits) >> rng.randrange(bits)


def near_int(fmt, bits, rng):
    """Values around halves, integers and the integer range limits."""
    if rng.random() < 0.3:
        return operands(fmt, rng)
    base = rng.choice([Fraction(rng.randrange(-9, 10), 2), Fraction(rng.randrange(-2 ** 24, 2 ** 24), 4),
                       Fraction(2 ** (bits - 1)), Fraction(-(2 ** (bits - 1))), Fraction(rng.randrange(1, 2 ** 20)) * 2 ** rng.randrange(40)])
    v = fmt.top if base == 0 and rng.random() < 0.5 else 0
    if base:
        v = rounded(fmt, base, {"mode": NEAREST, "ftz": False})[0]
    m = max(0, fmt.mag(v) + rng.randrange(-2, 3))
    return m + (fmt.top if fmt.neg(v) else 0)


def build():
    rng = random.Random(0x58454E4F5241)
    lines = []

    def emit(name, make, compute, count):
        done = 0
        while done < count:
            mode = rng.randrange(4)
            control = {"mode": mode, "daz": rng.random() < 0.2, "ftz": rng.random() < 0.2}
            mxcsr = 0x1F80 | mode << 13 | control["daz"] << 6 | control["ftz"] << 15
            a, b = make()
            out = compute(a, b, control)
            if out is None:
                continue
            lines.append(f"{name} {mxcsr:x} {a:x} {b:x} {out[0]:x} {out[1]:x}\n")
            done += 1

    def direct(name, mode, a, b, out):
        if out is not None:
            lines.append(f"{name} {0x1F80 | mode << 13:x} {a:x} {b:x} {out[0]:x} {out[1]:x}\n")

    for fmt, s in ((S, "ss"), (D, "sd")):
        # Every pair of special operands, for every two-operand rule.
        one, least = fmt.emax << fmt.frac, 1 << fmt.frac
        special = [0, fmt.top, 1, one, fmt.top + one, fmt.inf - 1, fmt.inf, fmt.top + fmt.inf,
                   fmt.inf + fmt.quiet + 5, fmt.top + fmt.inf + 3]
        for a in special:
            for b in special:
                for op in ("add", "sub", "mul", "div", "min", "max"):
                    mode = rng.randrange(4)
                    control = {"mode": mode, "daz": False, "ftz": False}
                    direct(op + s, mode, a, b, binary(op, fmt, a, b, control))
        # Products within a rounding step of the smallest normal: (1 + t)(1 - t)
        # times 2^emin. Tininess is judged after rounding, so some are not tiny.
        root_of_least = fmt.emin // 2
        for shift in range(fmt.frac // 2 - 3, fmt.frac // 2 + 6):
            t = 1 << (fmt.frac - shift)
            a = ((root_of_least + fmt.emax) << fmt.frac) + t
            rest = fmt.emin - root_of_least
            # 1 - t is in the binade below one, with twice the resolution.
            b = ((rest - 1 + fmt.emax) << fmt.frac) + (least - 2 * t)
            for mode in range(4):
                for x, y in ((a, b), (a + fmt.top, b)):
                    control = {"mode": mode, "daz": False, "ftz": False}
                    direct("mul" + s, mode, x, y, binary("mul", fmt, x, y, control))
    # The same neighbourhood for narrowing: 2^-126 (1 - 2^-k).
    for k in range(22, 30):
        b = D.encode(False, Fraction(2) ** S.emin * (1 - Fraction(1, 2 ** k)))
        for mode in range(4):
            for x in (b, b + D.top):
                direct("cvtsd2ss", mode, 0, x, widen(D, S, x, {"mode": mode, "daz": False, "ftz": False}))

    for fmt, s in ((S, "ss"), (D, "sd")):
        def pair(fmt=fmt):
            a = operands(fmt, rng)
            return a, related(fmt, a, rng)
        for op in ("add", "sub", "mul", "div", "min", "max"):
            emit(op + s, pair, lambda a, b, c, op=op, fmt=fmt: binary(op, fmt, a, b, c),
                 260 if op in ("add", "sub", "mul", "div") else 100)
        emit("sqrt" + s, lambda fmt=fmt: (0, operands(fmt, rng)), lambda a, b, c, fmt=fmt: root(fmt, b, c), 200)
        for code in range(8):
            emit(f"cmp{s}{code}", pair, lambda a, b, c, code=code, fmt=fmt: predicate(code, fmt, a, b, c), 40)
        emit("comi" + s, pair, lambda a, b, c, fmt=fmt: comparison(fmt, a, b, True, c), 80)
        emit("ucomi" + s, pair, lambda a, b, c, fmt=fmt: comparison(fmt, a, b, False, c), 80)
        for bits, w in ((32, "d"), (64, "q")):
            for name, truncate in (("cvt", False), ("cvtt", True)):
                emit(f"{name}{s}2si{w}", lambda fmt=fmt, bits=bits: (0, near_int(fmt, bits, rng)),
                     lambda a, b, c, fmt=fmt, bits=bits, t=truncate: to_int(fmt, b, bits, t, c), 110)
            emit(f"cvtsi2{s}{w}", lambda bits=bits: (0, integers(bits, rng)),
                 lambda a, b, c, fmt=fmt, bits=bits: from_int(fmt, b, bits, c), 80)
    emit("cvtss2sd", lambda: (0, operands(S, rng)), lambda a, b, c: widen(S, D, b, c), 120)

    def narrowing():
        if rng.random() < 0.4:
            return 0, operands(D, rng)
        # A binary32 neighbourhood, with bits beyond its precision.
        v = operands(S, rng)
        x = widen(S, D, v, {"mode": NEAREST, "daz": False, "ftz": False})[0]
        return 0, x ^ (rng.choice([0, 1, 1 << 28, (1 << 28) + 1, (1 << 29) - 1]) if D.mag(x) else 0)
    emit("cvtsd2ss", narrowing, lambda a, b, c: widen(D, S, b, c), 260)
    return "".join(lines)


def main(argv):
    if len(argv) != 2 or argv[1] not in ("generate", "--check"):
        sys.exit("usage: float_reference.py generate|--check")
    text = build()
    if argv[1] == "generate":
        OUT.write_text(text)
    elif OUT.read_text() != text:
        sys.exit("float_vectors.txt is out of date")
    print(f"{text.count(chr(10))} vectors")


if __name__ == "__main__":
    main(sys.argv)
