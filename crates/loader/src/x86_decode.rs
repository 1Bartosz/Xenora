// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared architectural decoding primitives for x86-64.
//!
//! The static PE probe, the software interpreter and the native DBT all use
//! these constants and helpers so that decoding limits agree across the
//! execution engines.

/// Architectural x86-64 instruction length limit, in bytes.
///
/// An instruction may not exceed 15 bytes (Intel SDM Vol. 2A, "Instruction
/// Length"). Sequences longer than this are not decodable and are rejected as
/// illegal instructions.
pub const MAX_INSTRUCTION_LEN: usize = 15;

/// Number of leading legacy segment prefix bytes (0x64 FS, 0x65 GS) at the
/// start of a code slice, capped at [`MAX_INSTRUCTION_LEN`].
///
/// The count includes the first byte when it is a segment prefix, so a run of
/// more than [`MAX_INSTRUCTION_LEN`] prefixes is reported as
/// [`MAX_INSTRUCTION_LEN`].
pub fn segment_prefix_len(code: &[u8]) -> usize {
    let mut len = match code.first() {
        Some(&0x64 | &0x65) => 1,
        _ => 0,
    };
    while len < MAX_INSTRUCTION_LEN && matches!(code.get(len), Some(&0x64 | &0x65)) {
        len += 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::{MAX_INSTRUCTION_LEN, segment_prefix_len};

    #[test]
    fn segment_prefix_len_counts_legacy_segment_prefixes() {
        assert_eq!(segment_prefix_len(&[]), 0);
        assert_eq!(segment_prefix_len(&[0x90]), 0);
        assert_eq!(segment_prefix_len(&[0x65, 0x90]), 1);
        assert_eq!(segment_prefix_len(&[0x64, 0x65, 0x31, 0xc0]), 2);
        assert_eq!(segment_prefix_len(&[0x65; 20]), MAX_INSTRUCTION_LEN);
        assert_eq!(segment_prefix_len(&[0x65; 15]), MAX_INSTRUCTION_LEN);
    }
}
