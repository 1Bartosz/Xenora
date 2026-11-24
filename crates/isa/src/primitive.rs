//! Reusable integer semantic primitives selected by the generated registry dispatch.
//!
//! Each function implements one closed [`crate::semantics::Operation`] at one
//! signedness, with the condition-code rule the RDNA 2 guide gives for it
//! (section 5.3, "Scalar Condition Code", and the SOP2 pseudocode in 12.1).
//! Forms share a primitive only when their declared operation, signedness and
//! SCC effect are identical; the generator never selects one by mnemonic.

/// Unsigned `a + b + carry_in` and the carry-out of bit 31.
pub const fn add_carry_u32(a: u32, b: u32, carry_in: bool) -> (u32, bool) {
    let (partial, first) = a.overflowing_add(b);
    let (value, second) = partial.overflowing_add(carry_in as u32);
    (value, first || second)
}

/// Unsigned `a - b - borrow_in` and the borrow (`b + borrow_in > a`).
pub const fn sub_borrow_u32(a: u32, b: u32, borrow_in: bool) -> (u32, bool) {
    let (partial, first) = a.overflowing_sub(b);
    let (value, second) = partial.overflowing_sub(borrow_in as u32);
    (value, first || second)
}

/// Two's-complement `a + b` and whether the exact sum leaves the i32 range.
pub const fn add_overflow_i32(a: u32, b: u32) -> (u32, bool) {
    let (value, overflow) = (a as i32).overflowing_add(b as i32);
    (value as u32, overflow)
}

/// Two's-complement `a - b` and whether the exact difference leaves the i32 range.
pub const fn sub_overflow_i32(a: u32, b: u32) -> (u32, bool) {
    let (value, overflow) = (a as i32).overflowing_sub(b as i32);
    (value as u32, overflow)
}

/// Bitwise AND and whether the result is nonzero.
pub const fn and_nonzero_b32(a: u32, b: u32) -> (u32, bool) {
    let value = a & b;
    (value, value != 0)
}

/// Bitwise OR and whether the result is nonzero.
pub const fn or_nonzero_b32(a: u32, b: u32) -> (u32, bool) {
    let value = a | b;
    (value, value != 0)
}

/// Bitwise XOR and whether the result is nonzero.
pub const fn xor_nonzero_b32(a: u32, b: u32) -> (u32, bool) {
    let value = a ^ b;
    (value, value != 0)
}
