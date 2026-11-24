// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod independent_ref;

#[cfg(test)]
mod review {
    use super::independent_ref;
    use xenora_shader::{Architecture, compile};

    const FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/add.bin");

    const OR_FIXTURE: &[u8; 40] = include_bytes!("../../fixtures/rdna2/or.bin");
    const OR_INPUT: [u32; 7] = [
        0,
        1,
        u32::MAX,
        0x8055_00aa,
        0x7faa_ff55,
        0x8000_0000,
        0x0055_00aa,
    ];
    const OR_EXPECTED: [u32; 7] = [
        0x8055_00aa,
        0x8055_00ab,
        u32::MAX,
        0x8055_00aa,
        u32::MAX,
        0x8055_00aa,
        0x8055_00aa,
    ];

    #[test]
    fn vector_or_matches_known_answers_and_preserves_unmasked_bits() {
        verify(OR_FIXTURE, &OR_INPUT, Some(&OR_EXPECTED));
        let input: Vec<_> = (0..32).map(|bit| 1_u32 << bit).collect();
        let expected: Vec<_> = input
            .iter()
            .map(|word| 0x8055_00aa + (word & 0x7faa_ff55))
            .collect();
        verify(OR_FIXTURE, &input, Some(&expected));

        let mut renamed = *OR_FIXTURE;
        // Reuse v255 as both destination and the second source.
        renamed[20..24].copy_from_slice(&0x39ff_ff01_u32.to_le_bytes());
        renamed[29] = 255;
        verify(&renamed, &OR_INPUT, Some(&OR_EXPECTED));
    }

    #[test]
    fn vector_or_rejects_invalid_operands_and_incomplete_streams() {
        for instruction in [
            0x3803_ff03_u32, // Uninitialized v3 in SRC0.
            0x3802_0701,     // Uninitialized v3 in VSRC1.
            0x3801_ff01,     // Write to synthetic index v0.
            0x3803_fe80,     // Inline SRC0 outside the supported subset.
        ] {
            let mut code = *OR_FIXTURE;
            code[20..24].copy_from_slice(&instruction.to_le_bytes());
            verify(&code, &OR_INPUT, None);
        }
        for end in 0..OR_FIXTURE.len() {
            verify(&OR_FIXTURE[..end], &OR_INPUT, None);
        }
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn vector_or_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let compiled = compile(Architecture::Gfx1030, OR_FIXTURE).unwrap();
        let input: Vec<_> = OR_INPUT.into_iter().cycle().take(131).collect();
        let expected: Vec<_> = OR_EXPECTED.into_iter().cycle().take(input.len()).collect();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    const ASHR_FIXTURE: &[u8; 40] = include_bytes!("../../fixtures/rdna2/ashr.bin");
    const ASHR_VALUE: i32 = i32::MIN + 1; // The fixture literal 0x8000_0001.

    fn arithmetic_shift_expected(value: i32, count: u32) -> u32 {
        // Floor division of the sign-extended value by 2^(count mod 32),
        // reduced modulo 2^32. Derived from the definition of an arithmetic
        // right shift; no host `>>` or production code is involved.
        i64::from(value)
            .div_euclid(1_i64 << (count % 32))
            .rem_euclid(1_i64 << 32) as u32
    }

    fn arithmetic_right_shift_words() -> (Vec<u32>, Vec<u32>) {
        let input: Vec<u32> = (0..64).chain([0x8000_0000, u32::MAX]).collect();
        let expected = input
            .iter()
            .map(|&count| arithmetic_shift_expected(ASHR_VALUE, count))
            .collect();
        (input, expected)
    }

    #[test]
    fn arithmetic_right_shift_preserves_sign_and_wraps_counts() {
        let (input, expected) = arithmetic_right_shift_words();
        assert_eq!(xenora_ref::execute(ASHR_FIXTURE, &input).unwrap(), expected);
        let compiled = compile(Architecture::Gfx1030, ASHR_FIXTURE).unwrap();
        assert!(compiled.ir.contains("ashr.i32 t1, t0"));
        assert!(
            compiled
                .msl
                .contains("as_type<uint>(as_type<int>(t1) >> (t0 & 31u))")
        );
    }

    #[test]
    fn arithmetic_right_shift_matches_explicit_checkpoints() {
        // Hand-derived from floor(-2147483647 / 2^(count mod 32)) mod 2^32.
        // Counts above 31 alias because the hardware masks the count.
        const CHECKPOINTS: &[(u32, u32)] = &[
            (0, 0x8000_0001),
            (1, 0xc000_0000),
            (2, 0xe000_0000),
            (3, 0xf000_0000),
            (4, 0xf800_0000),
            (7, 0xff00_0000),
            (8, 0xff80_0000),
            (15, 0xffff_0000),
            (16, 0xffff_8000),
            (23, 0xffff_ff00),
            (24, 0xffff_ff80),
            (30, 0xffff_fffe),
            (31, 0xffff_ffff),
            (32, 0x8000_0001),
            (33, 0xc000_0000),
            (63, 0xffff_ffff),
            (0x8000_0000, 0x8000_0001), // Masked to a count of zero.
            (u32::MAX, 0xffff_ffff),    // Masked to a count of 31.
        ];
        for &(count, expected) in CHECKPOINTS {
            assert_eq!(
                arithmetic_shift_expected(ASHR_VALUE, count),
                expected,
                "derivation for count {count:#x}"
            );
            let input = [count];
            assert_eq!(
                xenora_ref::execute(ASHR_FIXTURE, &input).unwrap(),
                [expected],
                "reference for count {count:#x}"
            );
            assert_eq!(
                independent_ref::execute(ASHR_FIXTURE, &input).unwrap(),
                [expected],
                "independent oracle for count {count:#x}"
            );
        }
        // A logical shift would have produced 0x4000_0000 for count one.
        assert_ne!(arithmetic_shift_expected(ASHR_VALUE, 1), 0x4000_0000);
    }

    #[test]
    fn arithmetic_right_shift_positive_control_and_renamed_destination() {
        let (input, _) = arithmetic_right_shift_words();
        let mut positive = *ASHR_FIXTURE;
        // Replace only the literal word at offset 0x10.
        positive[16..20].copy_from_slice(&0x7fff_ffff_u32.to_le_bytes());
        let expected: Vec<u32> = input
            .iter()
            .map(|&count| arithmetic_shift_expected(0x7fff_ffff, count))
            .collect();
        assert_eq!(expected[0], 0x7fff_ffff);
        assert_eq!(expected[1], 0x3fff_ffff);
        assert_eq!(expected[30], 1);
        assert_eq!(expected[31], 0);
        verify(&positive, &input, Some(&expected));

        // Keep the value source at v255 and move the result to v254 so the
        // destination no longer aliases the value input.
        let mut renamed = *ASHR_FIXTURE;
        renamed[20..24].copy_from_slice(&0x31fd_ff01_u32.to_le_bytes());
        renamed[29] = 0xfe; // buffer_store_dword v254
        let (_, expected) = arithmetic_right_shift_words();
        verify(&renamed, &input, Some(&expected));
    }

    #[test]
    fn arithmetic_right_shift_rejects_invalid_operands_and_incomplete_streams() {
        let (input, _) = arithmetic_right_shift_words();
        for instruction in [
            0x31ff_ff03_u32, // Uninitialized v3 as the count.
            0x31fe_0701,     // Uninitialized v3 as the value.
            0x3001_ff01,     // Write to synthetic index v0.
            0x31ff_ff80,     // Inline SRC0 outside the supported subset.
        ] {
            let mut code = *ASHR_FIXTURE;
            code[20..24].copy_from_slice(&instruction.to_le_bytes());
            verify(&code, &input, None);
        }
        for end in 0..ASHR_FIXTURE.len() {
            verify(&ASHR_FIXTURE[..end], &input, None);
        }
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn arithmetic_right_shift_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let (input, expected) = arithmetic_right_shift_words();
        let compiled = compile(Architecture::Gfx1030, ASHR_FIXTURE).unwrap();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    fn wave32_masked_ashr() -> Vec<u8> {
        // Preserve the original input load, wait and v255 literal.
        let mut code = ASHR_FIXTURE[..20].to_vec();
        code.extend(words(&[
            0xbeff_03c1, // s_mov_b32 exec_hi, -1
            0xbeeb_03c1, // s_mov_b32 vcc_hi, -1
            0xbeea_0380, // s_mov_b32 vcc_lo, 0
            0xbf88_0001, // s_cbranch_execz: initial low mask is full
            0xbf82_0001, // s_branch past the incorrect-path overwrite
            0x7ffe_0287, // v_mov_b32 v255, 7
            0xbefe_0380, // s_mov_b32 exec_lo, 0
            0x7ffe_0287, // inactive v_mov_b32 leaves v255 unchanged
            0xbf88_0001, // s_cbranch_execz: low half zero despite high half
            0xbeea_0381, // setting vcc_lo on this wrong path changes the result
            0xbefe_03c1, // s_mov_b32 exec_lo, -1
            0xbf86_0001, // s_cbranch_vccz: low half zero despite high half
            0x7ffe_0287, // v_mov_b32 v255, 7 if VCC_LO is nonzero
        ]));
        code.extend_from_slice(&ASHR_FIXTURE[20..]);
        code
    }

    #[test]
    fn wave32_exec_and_vcc_zero_checks_use_low_halves() {
        let input = [0, 1, 31, 32, 63];
        let mut code = wave32_masked_ashr();
        verify(
            &code,
            &input,
            Some(&[0x8000_0001, 0xc000_0000, u32::MAX, 0x8000_0001, u32::MAX]),
        );
        let compiled = compile(Architecture::Gfx1030, &code).unwrap();
        assert!(compiled.ir.contains("ashr.i32"));
        assert!(compiled.msl.contains(" >> (t0 & 31u)"));
        assert_eq!(compiled.ir.matches("taken=true").count(), 3);
        assert_eq!(compiled.ir.matches("taken=false").count(), 1);

        // Change only the VCC_LO initializer. The VCCZ branch falls through.
        code[28] = 0x81;
        verify(&code, &input, Some(&[7, 3, 0, 7, 0]));
    }

    #[test]
    fn wave32_rejects_invalid_branches_and_empty_exec_memory() {
        let code = wave32_masked_ashr();
        for (offset, encoding) in [
            (28, 0xbee9_0380u32), // Initialize s105 instead of VCC_LO.
            (52, 0xbf88_ffff),    // Backward EXECZ branch.
            (64, 0xbf86_0003),    // VCCZ branch into the second store word.
        ] {
            let mut rejected = code.clone();
            rejected[offset..offset + 4].copy_from_slice(&encoding.to_le_bytes());
            verify(&rejected, &[1], None);
        }
        // A reachable vector memory instruction with empty EXEC is unsupported.
        let mut rejected = code;
        rejected[48..52].copy_from_slice(&0xe030_2000_u32.to_le_bytes());
        rejected.splice(52..52, 0x8000_0100_u32.to_le_bytes());
        verify(&rejected, &[1], None);
    }

    fn partial_exec_ashr() -> Vec<u8> {
        // Original gfx1030 instructions checked with llvm-mc -show-encoding.
        // AMD's RDNA 2 ISA, section 6.2.2, predicates VGPR writes on EXEC.
        let mut code = ASHR_FIXTURE[..20].to_vec();
        code.extend(words(&[
            0x7e04_0287, // v_mov_b32 v2, 7
            0xbefe_03ff,
            0x8000_0001, // s_mov_b32 exec_lo, 0x80000001
            0xbf88_0001, // EXECZ would skip the move if the mask were empty
            0x7ffe_0302, // v_mov_b32 v255, v2
            0xd525_00ff,
            0x0202_03ff, // v_add_nc_u32_e64 v255, v255, v1
            0xbefe_03c1, // s_mov_b32 exec_lo, -1
        ]));
        code.extend_from_slice(&ASHR_FIXTURE[20..]);
        code
    }

    fn partial_exec_expected() -> [u32; 67] {
        let mut expected = [0xc000_0000; 67];
        for index in [0, 31, 32, 63, 64] {
            expected[index] = 4; // (7 + 1) arithmetic-shifted right one bit.
        }
        expected
    }

    #[test]
    fn partial_wave32_updates_only_the_selected_lanes() {
        verify(
            &partial_exec_ashr(),
            &[1; 67],
            Some(&partial_exec_expected()),
        );
    }

    #[test]
    fn partial_wave32_rejects_vector_memory_and_uninitialized_destinations() {
        let mut uninitialized = partial_exec_ashr();
        // Replace the masked move's destination with uninitialized v254.
        uninitialized[36..40].copy_from_slice(&0x7ffc_0302_u32.to_le_bytes());
        verify(&uninitialized, &[1; 33], None);

        let mut masked_memory = partial_exec_ashr();
        // Leave the partial mask in place for the final buffer store.
        masked_memory.drain(48..52);
        verify(&masked_memory, &[1; 33], None);
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn partial_wave32_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let compiled = compile(Architecture::Gfx1030, &partial_exec_ashr()).unwrap();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &[1; 67])
            .unwrap();
        assert_eq!(actual, partial_exec_expected());
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn wave32_masked_ashr_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let (input, expected) = arithmetic_right_shift_words();
        let compiled = compile(Architecture::Gfx1030, &wave32_masked_ashr()).unwrap();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    const SHIFT_FIXTURE: &[u8; 56] = include_bytes!("../../fixtures/rdna2/shifts.bin");
    const SHIFT_INPUT: [u32; 12] = [
        0,
        1,
        2,
        30,
        31,
        32,
        33,
        63,
        64,
        0x8000_0000,
        0x8000_0021,
        u32::MAX,
    ];
    const SHIFT_EXPECTED: [u32; 12] = [
        2,
        0x4000_0006,
        0x2000_000c,
        0xc000_0002,
        0x8000_0001,
        2,
        0x4000_0006,
        0x8000_0001,
        2,
        2,
        0x4000_0006,
        0x8000_0001,
    ];

    #[test]
    fn vector_shifts_match_known_answers_with_aliased_operands() {
        verify(SHIFT_FIXTURE, &SHIFT_INPUT, Some(&SHIFT_EXPECTED));
    }

    #[test]
    fn vector_shifts_reject_invalid_operands_and_incomplete_streams() {
        for (offset, instruction) in [
            (28, 0x2dff_ff03_u32), // Uninitialized count.
            (28, 0x2dfe_0701),     // Uninitialized value.
            (28, 0x2c01_ff01),     // Write to synthetic index.
            (28, 0x2dff_fe08),     // SGPR count.
            (32, 0x3402_0503),     // Uninitialized count.
            (32, 0x3402_0701),     // Uninitialized value.
            (32, 0x3400_0501),     // Write to synthetic index.
            (32, 0x3402_0480),     // Inline count.
        ] {
            let mut code = *SHIFT_FIXTURE;
            code[offset..offset + 4].copy_from_slice(&instruction.to_le_bytes());
            verify(&code, &SHIFT_INPUT, None);
        }
        for end in 28..SHIFT_FIXTURE.len() {
            verify(&SHIFT_FIXTURE[..end], &SHIFT_INPUT, None);
        }
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn vector_shifts_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let compiled = compile(Architecture::Gfx1030, SHIFT_FIXTURE).unwrap();
        let input: Vec<_> = SHIFT_INPUT.into_iter().cycle().take(131).collect();
        let expected: Vec<_> = SHIFT_EXPECTED
            .into_iter()
            .cycle()
            .take(input.len())
            .collect();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    fn words(code: &[u32]) -> Vec<u8> {
        code.iter().copied().flat_map(u32::to_le_bytes).collect()
    }

    fn verify(code: &[u8], input: &[u32], expected: Option<&[u32]>) {
        let compiled = compile(Architecture::Gfx1030, code);
        let reference = xenora_ref::execute(code, input);
        let independent = independent_ref::execute(code, input);
        assert_eq!(compiled.is_ok(), expected.is_some(), "{compiled:?}");
        assert_eq!(reference.as_deref().ok(), expected, "{reference:?}");
        assert_eq!(independent.as_deref().ok(), expected, "{independent:?}");
    }

    fn check(input: &[u32]) {
        let expected: Vec<_> = input
            .iter()
            .map(|&x| ((u64::from(x) + 0x1234_5678) % (1_u64 << 32)) as u32)
            .collect();
        assert_eq!(xenora_ref::execute(FIXTURE, input).unwrap(), expected);
    }

    #[test]
    fn independently_derived_addition_for_boundary_and_pseudorandom_inputs() {
        let wrap_at = 0xEDCB_A988;
        for base in [0, wrap_at - 0x8000, 0x7fff_8000, 0xffff_0000] {
            let input: Vec<_> = (base..=base + u32::from(u16::MAX)).collect();
            for chunk in input.chunks(4096) {
                check(chunk);
            }
        }
        let mut seed = 0xbfac_128d_u32;
        let input: Vec<_> = (0..4096)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed
            })
            .collect();
        check(&input);
    }

    #[test]
    fn compiler_and_reference_agree_on_acceptance_of_each_single_bit_mutation() {
        for bit in 0..FIXTURE.len() * 8 {
            let mut code = FIXTURE.to_vec();
            code[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(
                compile(Architecture::Gfx1030, &code).is_ok(),
                xenora_ref::execute(&code, &[42]).is_ok(),
                "acceptance differed at byte {}, bit {}",
                bit / 8,
                bit % 8,
            );
        }
    }

    #[test]
    fn scalar_compare_selects_both_forward_edges() {
        let fixture = include_bytes!("../../fixtures/rdna2/bitwise-branch.bin");
        let input = [0, 1, 0x00ff_00ff, 0xff00_ff00, u32::MAX, 0xedcb_a988];
        let expected = [
            0x1234_5678,
            0x1234_5679,
            0x12cb_5687,
            0x1234_5678,
            0x12cb_5687,
            0x12ff_56f0,
        ];
        assert_eq!(xenora_ref::execute(fixture, &input).unwrap(), expected);
        let taken = compile(Architecture::Gfx1030, fixture).unwrap();
        assert!(taken.msl.contains(" & "));
        assert!(taken.msl.contains(" ^ "));

        let mut equal = *fixture;
        assert_eq!(equal[17], 0x86); // Inline unsigned 6 in the SCC comparison.
        equal[17] = 0x87; // Compare 7 with 7 and fall through to v2 = 0.
        assert_eq!(xenora_ref::execute(&equal, &input).unwrap(), [0; 6]);
        let fallthrough = compile(Architecture::Gfx1030, &equal).unwrap();
        assert!(!fallthrough.msl.contains(" & "));
        assert!(!fallthrough.msl.contains(" ^ "));

        let mut into_literal = *fixture;
        into_literal[20] = 3; // PC + 4 + 3 * 4 = 36, the literal of the instruction at 32.
        assert!(compile(Architecture::Gfx1030, &into_literal).is_err());
        assert!(xenora_ref::execute(&into_literal, &input).is_err());
    }

    #[test]
    fn float_results_and_rejections_follow_the_exact_domain() {
        let fixture = include_bytes!("../../fixtures/rdna2/float-exact.bin");
        let input = [
            0,
            0x003f_ffff,
            0x0040_0000,
            0x0040_0001,
            0x007f_ffff,
            0x7f80_0000,
            0x7fc0_0001,
            u32::MAX,
        ];
        // Hand-derived: (1 + 5/2) * 5/2 = 35/4; (3/2 + 5/2) * 5/2 = 10.
        let expected = [
            0x410c_0000,
            0x410c_0000,
            0x4120_0000,
            0x4120_0000,
            0x4120_0000,
            0x410c_0000,
            0x4120_0000,
            0x4120_0000,
        ];
        assert_eq!(xenora_ref::execute(fixture, &input).unwrap(), expected);
        assert!(compile(Architecture::Gfx1030, fixture).is_ok());

        for rejected_literal in [1_u32, 0x3380_0000, 0x0080_0000, 0xff80_0000] {
            // Subnormal, midpoint rounding at 1, binary64 absorption, infinity.
            let mut changed = *fixture;
            changed[36..40].copy_from_slice(&rejected_literal.to_le_bytes());
            assert!(
                compile(Architecture::Gfx1030, &changed).is_err(),
                "{rejected_literal:08x}"
            );
            assert!(
                xenora_ref::execute(&changed, &[0]).is_err(),
                "{rejected_literal:08x}"
            );
        }
    }

    #[test]
    fn second_interpreter_agrees_with_reference_on_boundary_and_random_inputs() {
        let fixtures: [(&[u8], &str); 4] = [
            (include_bytes!("../../fixtures/rdna2/add.bin"), "add"),
            (
                include_bytes!("../../fixtures/rdna2/bitwise-branch.bin"),
                "bitwise-branch",
            ),
            (
                include_bytes!("../../fixtures/rdna2/float-exact.bin"),
                "float-exact",
            ),
            (include_bytes!("../../fixtures/rdna2/ashr.bin"), "ashr"),
        ];
        let mut inputs: Vec<u32> = vec![
            0,
            1,
            u32::MAX,
            0x8000_0000,
            0x7fff_ffff,
            0xedcb_a988,
            0x0040_0000,
            0x0040_0001,
            0x007f_ffff,
            0x0080_0000,
            0x00ff_00ff,
            0xff00_ff00,
        ];
        for base in [
            0xedcb_a988u32.wrapping_sub(64),
            0x0040_0000u32.wrapping_sub(64),
        ] {
            inputs.extend((0..128).map(|i| base.wrapping_add(i)));
        }
        let mut seed = 0x9e37_79b9u32;
        inputs.extend((0..3000).map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        }));
        for (code, name) in fixtures {
            let expected = xenora_ref::execute(code, &inputs).unwrap();
            let actual = independent_ref::execute(code, &inputs).unwrap();
            assert_eq!(actual, expected, "fixture {name}");
        }
    }

    #[test]
    fn binary32_boundaries_follow_exact_significand_arithmetic() {
        const ADD: u32 = 0x0602_0702;
        const MUL: u32 = 0x1002_0702;
        let input = [0, 0x0040_0000, u32::MAX];
        let one = 0x3f80_0000;
        let min_normal = 0x0080_0000;
        let max_normal = 0x7f7f_ffff;
        for (op, a, b, expected) in [
            (ADD, one, 0x3400_0000, Some(0x3f80_0001)), // 1 + 2^-23
            (ADD, 0x3f7f_ffff, 0x3380_0000, Some(one)), // (1 - 2^-24) + 2^-24
            (ADD, min_normal, min_normal, Some(0x0100_0000)), // 2^-125
            (ADD, one, 0x3380_0000, None),              // Midpoint 1 + 2^-24
            (ADD, one, min_normal, None), // Absorbed in binary64 if computed in host FP
            (ADD, max_normal, max_normal, None), // Overflow
            (MUL, 0x3fc0_0000, 0x3fc0_0000, Some(0x4010_0000)), // (3/2)^2 = 9/4
            (MUL, 0x7e80_0000, min_normal, Some(one)), // 2^126 × 2^-126
            (MUL, max_normal, one, Some(max_normal)),
            (MUL, 0x4580_0800, 0x4580_0800, None), // 4097² = 16,785,409, an odd integer
            (MUL, min_normal, 0x3f00_0000, None),  // 2^-127 is subnormal
            (MUL, max_normal, 0x4000_0000, None),  // Overflow
            (ADD, 0, one, None),                   // Positive zero
            (ADD, one, 0x8000_0000, None),         // Negative zero
            (ADD, 0xbf80_0000, 0x4000_0000, None), // Result normal, operand negative
            (ADD, 0x4000_0000, 0xbf80_0000, None), // Same result, other operand negative
            (ADD, one, 1, None),                   // Smallest positive subnormal
            (MUL, 1, 0x7e80_0000, None), // Subnormal operand would yield exact normal 2^-23
            (MUL, 0x7e80_0000, 1, None),
            (ADD, one, 0x7f80_0000, None), // Infinity
            (MUL, 0xff80_0000, one, None), // Negative infinity
            (ADD, one, 0x7fc0_0001, None), // Quiet NaN with payload
            (MUL, 0x7f80_0001, one, None), // Signaling NaN with payload
        ] {
            // Force literal moves into v2 and v3. VOP2 writes v1, the existing
            // store source. The exact words were cross-checked with gfx1030 llvm-mc.
            let mut code = words(&[0x7e04_02ff, a, 0x7e06_02ff, b, op]);
            code.extend_from_slice(&FIXTURE[28..]);
            let expected = expected.map(|word| [word; 3]);
            verify(
                &code,
                &input,
                expected.as_ref().map(|values| values.as_slice()),
            );
        }
    }

    #[test]
    fn vector_register_renaming_and_in_place_add_preserve_known_answers() {
        let input = [0, 1, 0xedcb_a988, 0x8000_0000, u32::MAX];
        let expected = [0x1234_5678, 0x1234_5679, 0, 0x9234_5678, 0x1234_5677];
        let renamed = words(&[
            0xe030_2000,
            0x8000_ff00, // Load v255 instead of v1.
            0xbf8c_3f70,
            0x7f00_02ff,
            0x1234_5678, // Move the literal to v128.
            0xd525_00ff,
            0x0203_01ff, // Add v255, v255, v128.
            0x7ffc_03ff, // Copy v255 to v254 before storing.
            0xe070_2000,
            0x8001_fe00, // Store v254.
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        verify(FIXTURE, &input, Some(&expected));
        verify(&renamed, &input, Some(&expected));
    }

    #[test]
    fn scalar_register_copy_and_literal_comparison_select_both_edges() {
        let input = [0, u32::MAX];
        for (literal, expected) in [(0xffff_fff0, [1, 1]), (0xffff_fff1, [2, 2])] {
            let mut code = words(&[
                0xbee9_03d0, // s_mov_b32 s105, -16
                0xbe89_0369, // s_mov_b32 s9, s105
                0xbf06_ff09,
                literal,     // s_cmp_eq_u32 s9, lit(<word>)
                0xbf84_0002, // Skip the next move and branch if unequal.
                0x7e02_0281, // v_mov_b32_e32 v1, 1
                0xbf82_0001, // Skip the unequal case's move.
                0x7e02_0282, // v_mov_b32_e32 v1, 2
            ]);
            code.extend_from_slice(&FIXTURE[28..]);
            verify(&code, &input, Some(&expected));
            for end in 0..code.len() {
                verify(&code[..end], &input, None);
            }
        }
    }

    #[test]
    fn skipped_code_still_has_to_obey_the_fixture_layout_and_bindings() {
        for bad_prefix in [
            &[0xbf82_0001u32, 0xbf81_0000][..],       // Early termination.
            &[0xbf82_0001, 0xbeec_0380],              // Privileged TTMP0 destination.
            &[0xbf82_0002, 0xe030_3000, 0x8000_0100], // Unsupported OFFEN with IDXEN.
            &[0xbf82_0002, 0xe030_2000, 0x8000_0101], // v1 instead of v0 as index.
            &[0xbf82_0002, 0xe030_2000, 0x8001_0100], // Load the wrong buffer.
            &[0xbf82_0002, 0xe070_2000, 0x8000_0100], // Store to the input buffer.
        ] {
            let mut code = words(bad_prefix);
            code.extend_from_slice(FIXTURE);
            verify(&code, &[1], None);
        }
        // A well-formed but unreachable move need not have an initialized source.
        let mut skipped_move = words(&[0xbf82_0001, 0x7e02_03ff]);
        skipped_move.extend_from_slice(FIXTURE);
        verify(&skipped_move, &[1], Some(&[0x1234_5679]));
    }

    #[test]
    fn reachable_vector_writes_respect_pending_loads_and_the_index_register() {
        let mut pending = FIXTURE.to_vec();
        pending.splice(8..8, 0x7e02_0281_u32.to_le_bytes()); // Move 1 to pending v1.
        verify(&pending, &[1], None);
        let mut overwrites_index = FIXTURE.to_vec();
        overwrites_index[20] = 0; // Change the VOP3 add's destination from v1 to v0.
        verify(&overwrites_index, &[1], None);
    }

    #[test]
    fn rejects_overlapping_reads_even_if_the_later_read_would_supply_the_output() {
        let code = words(&[
            0xe030_2000,
            0x8000_0100, // Load into v1 without waiting.
            0xe030_2000,
            0x8000_0200, // Load into v2 while v1 is pending.
            0xbf8c_3f70,
            0xe070_2000,
            0x8001_0200, // Store v2, leaving v1 unused.
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        verify(&code, &[0x8765_4321], None);
    }

    #[test]
    fn rejects_a_second_store_and_requires_waiting_for_a_following_load() {
        for (at, extra) in [
            (36, &FIXTURE[28..36]), // Second store before the original store wait.
            (40, &FIXTURE[28..40]), // Second store and wait after a completed output.
            (40, &FIXTURE[..8]),    // Load after the completed output, without a load wait.
        ] {
            let mut code = FIXTURE.to_vec();
            code.splice(at..at, extra.iter().copied());
            verify(&code, &[1], None);
        }
    }

    #[test]
    fn distinguishes_descriptor_and_ordinary_scalar_registers() {
        for (mov, expected) in [
            (0xbe87_0381, None), // s_mov_b32 s7, 1 writes a reserved descriptor.
            (0xbe88_0381, Some([0x1234_5679])), // s8 sits at the boundary.
            (0xbe89_0381, Some([0x1234_5679])), // s9 is an ordinary scalar.
        ] {
            let mut code = words(&[mov]);
            code.extend_from_slice(FIXTURE);
            verify(&code, &[1], expected.as_ref().map(|words| words.as_slice()));
        }
    }

    const SCALAR_ALU_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/scalar-alu.bin");
    // SOP2 opcodes of the supported subset (RDNA 2 XML opcode table).
    const SOP2_OPCODES: [u32; 9] = [0, 1, 2, 3, 4, 5, 14, 16, 18];
    const SCALAR_VALUES: [u32; 10] = [
        0,
        1,
        2,
        0x7fff_ffff,
        0x8000_0000,
        0x8000_0001,
        0xffff_fffe,
        u32::MAX,
        0x1234_5678,
        0xedcb_a988,
    ];

    /// ENCODING 0b10 in bits 30-31, OP 23-29, SDST 16-22, SSRC1 8-15, SSRC0 0-7.
    fn sop2(opcode: u32, dst: u32, src0: u32, src1: u32) -> u32 {
        0x8000_0000 | opcode << 23 | dst << 16 | src1 << 8 | src0
    }

    /// D and SCC from the operation's definition, with exact integers: additions
    /// carry out of bit 31, subtractions borrow, bitwise results set SCC iff nonzero.
    fn sop2_expected(opcode: u32, a: u32, b: u32, carry: bool) -> (u32, bool) {
        let (a, b) = (u64::from(a), u64::from(b));
        let c = u64::from(carry && matches!(opcode, 4 | 5));
        let (exact, scc) = match opcode {
            0 | 4 => (a + b + c, a + b + c >= 1 << 32),
            1 | 5 => (((1 << 32) + a - b - c) % (1 << 32), b + c > a),
            // Signed: the exact result of the sign-extended operands must fit in
            // [-2^31, 2^31), else SCC = overflow (RDNA 2 ISA guide 5.3 and 12.1).
            2 | 3 => {
                let signed = |value: u64| i64::from(value as u32 as i32);
                let exact = if opcode == 2 {
                    signed(a) + signed(b)
                } else {
                    signed(a) - signed(b)
                };
                let overflow = !(-(1_i64 << 31)..(1_i64 << 31)).contains(&exact);
                (exact.rem_euclid(1 << 32) as u64, overflow)
            }
            14 => (a & b, a & b != 0),
            16 => (a | b, a | b != 0),
            _ => (a ^ b, a ^ b != 0),
        };
        ((exact % (1 << 32)) as u32, scc)
    }

    /// s8 = a, s9 = b, SCC = `carry` (if set), then `alu` (with an optional literal),
    /// then v2 = s<result>; lanes are XORed with their input only when SCC is set.
    fn sop2_program(alu: &[u32], result: u32, a: u32, b: u32, carry: Option<bool>) -> Vec<u8> {
        let mut words = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
        words.extend([0xbe88_03ff, a, 0xbe89_03ff, b]);
        if let Some(carry) = carry {
            // s_cmp_eq_u32 0, 0 sets SCC; s_cmp_eq_u32 0, 1 clears it.
            words.push(if carry { 0xbf06_8080 } else { 0xbf06_8180 });
        }
        words.extend(alu);
        words.extend([0x7e04_0200 | result, 0xbf84_0001, 0x3a04_0501]);
        words.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    fn lanes(value: u32, scc: bool) -> [u32; 2] {
        [value, if scc { !value } else { value }]
    }

    /// Every supported opcode over boundary operands, with SCC both clear and set
    /// before the carry-consuming forms.
    fn sop2_vectors() -> Vec<(Vec<u8>, [u32; 2])> {
        let mut vectors = Vec::new();
        for opcode in SOP2_OPCODES {
            for a in SCALAR_VALUES {
                for b in SCALAR_VALUES {
                    for carry in [false, true] {
                        let (value, scc) = sop2_expected(opcode, a, b, carry);
                        let code = sop2_program(&[sop2(opcode, 10, 8, 9)], 10, a, b, Some(carry));
                        vectors.push((code, lanes(value, scc)));
                    }
                }
            }
        }
        vectors
    }

    #[test]
    fn scalar_alu_fixture_chains_carry_borrow_and_bitwise_scc() {
        let input = [0, 1, u32::MAX, 0x0000_000e, 0x1234_5678];
        let expected: Vec<_> = input.iter().map(|word| word ^ 0x0000_000e).collect();
        verify(SCALAR_ALU_FIXTURE, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, SCALAR_ALU_FIXTURE).unwrap();
        // The scalar results reach the kernel as constants; the branch is resolved.
        assert!(compiled.disassembly.contains("s_subb_u32 s11, s9, s8"));
        assert!(compiled.ir.contains("const.u32"), "{}", compiled.ir);
        // Clearing the final AND's SCC-zero edge (s15 = s14 & 15 = 0) is observable:
        // with 31 instead of 15 the result is 0x10, SCC = 1, and v2 = 0.
        let mut nonzero = SCALAR_ALU_FIXTURE.to_vec();
        assert_eq!(nonzero[0x39], 0x8f);
        nonzero[0x39] = 0x9f;
        verify(&nonzero, &input, Some(&[0; 5]));
    }

    /// The translator's own folding: v2 = D as a constant, and the SCC-zero edge
    /// is taken exactly when SCC is clear.
    fn assert_translated(code: &[u8], expected: &[u32; 2]) {
        let ir = compile(Architecture::Gfx1030, code).unwrap().ir;
        assert!(
            ir.contains(&format!("const.u32 0x{:08x}", expected[0])),
            "{ir}"
        );
        let taken = expected[0] == expected[1];
        assert!(ir.contains(&format!("taken={taken}")), "{ir}");
    }

    /// Aliased destinations, literals in either source and inline integers.
    fn sop2_operand_cases() -> Vec<(Vec<u8>, [u32; 2])> {
        let (a, b) = (0xffff_fff0, 0x0000_0011);
        let mut cases = Vec::new();
        for opcode in SOP2_OPCODES {
            for (dst, src0, src1) in [(8, 8, 9), (9, 8, 9), (8, 8, 8), (9, 9, 9)] {
                let x = if src0 == 9 { b } else { a };
                let y = if src1 == 8 { a } else { b };
                let (value, scc) = sop2_expected(opcode, x, y, true);
                let code = sop2_program(&[sop2(opcode, dst, src0, src1)], dst, a, b, Some(true));
                cases.push((code, lanes(value, scc)));
            }
            let (value, scc) = sop2_expected(opcode, a, b, true);
            cases.push((
                sop2_program(&[sop2(opcode, 10, 8, 255), b], 10, a, 7, Some(true)),
                lanes(value, scc),
            ));
            let (value, scc) = sop2_expected(opcode, b, a, true);
            cases.push((
                sop2_program(&[sop2(opcode, 10, 255, 8), b], 10, a, 7, Some(true)),
                lanes(value, scc),
            ));
            for (selector, inline) in [(0x80, 0), (0xc0, 64), (0xc1, u32::MAX), (0xd0, 0xffff_fff0)]
            {
                let (value, scc) = sop2_expected(opcode, a, inline, true);
                let code = sop2_program(&[sop2(opcode, 10, 8, selector)], 10, a, b, Some(true));
                cases.push((code, lanes(value, scc)));
            }
        }
        cases
    }

    #[test]
    fn scalar_alu_matches_definitions_on_boundary_operands() {
        let vectors = sop2_vectors();
        assert_eq!(vectors.len(), 9 * 10 * 10 * 2);
        for (code, expected) in &vectors {
            verify(code, &[0, u32::MAX], Some(expected));
            assert_translated(code, expected);
        }
    }

    #[test]
    fn signed_add_and_subtract_set_scc_on_overflow_not_carry() {
        // (opcode, S0, S1, D, SCC), worked by hand from RDNA 2 ISA guide 12.1.
        for (opcode, a, b, value, scc) in [
            (2, 0x7fff_ffff, 1, 0x8000_0000, true), // MAX + 1: overflow, no carry
            (2, u32::MAX, 1, 0, false),             // -1 + 1: carry, no overflow
            (2, 0x8000_0000, u32::MAX, 0x7fff_ffff, true), // MIN + -1: overflow
            (2, 0x8000_0000, 0x7fff_ffff, u32::MAX, false), // MIN + MAX = -1
            (3, 0x8000_0000, 1, 0x7fff_ffff, true), // MIN - 1: overflow
            (3, 0, 1, u32::MAX, false),             // 0 - 1: borrow, no overflow
            (3, 0x7fff_ffff, u32::MAX, 0x8000_0000, true), // MAX - -1: overflow
            (3, 0x8000_0000, 0x8000_0000, 0, false), // MIN - MIN = 0
        ] {
            let code = sop2_program(&[sop2(opcode, 10, 8, 9)], 10, a, b, Some(false));
            let expected = lanes(value, scc);
            assert_eq!(sop2_expected(opcode, a, b, false), (value, scc));
            verify(&code, &[0, u32::MAX], Some(&expected));
            assert_translated(&code, &expected);
        }
        for (code, expected) in signed_immediate_overflows() {
            verify(&code, &[0, u32::MAX], Some(&expected));
            assert_translated(&code, &expected);
        }
    }

    /// Overflow through an inline and a literal source, observed by the branch.
    fn signed_immediate_overflows() -> Vec<(Vec<u8>, [u32; 2])> {
        vec![
            // s_add_i32 s10, s8, 1 with s8 = MAX: 0x80000000, overflow.
            (
                sop2_program(&[sop2(2, 10, 8, 0x81)], 10, 0x7fff_ffff, 0, Some(false)),
                lanes(0x8000_0000, true),
            ),
            // s_sub_i32 s10, s8, 0x12345678 with s8 = MIN: 0x6dcba988, overflow.
            (
                sop2_program(
                    &[sop2(3, 10, 8, 255), 0x1234_5678],
                    10,
                    0x8000_0000,
                    0,
                    Some(false),
                ),
                lanes(0x6dcb_a988, true),
            ),
            // s_add_i32 s10, 0x40000000, s8 with s8 = 0x40000000: 0x80000000, overflow.
            (
                sop2_program(
                    &[sop2(2, 10, 255, 8), 0x4000_0000],
                    10,
                    0x4000_0000,
                    0,
                    Some(false),
                ),
                lanes(0x8000_0000, true),
            ),
            // s_sub_i32 s10, -16, s8 with s8 = MAX: 0x7ffffff1, overflow.
            (
                sop2_program(&[sop2(3, 10, 0xd0, 8)], 10, 0x7fff_ffff, 0, Some(false)),
                lanes(0x7fff_fff1, true),
            ),
        ]
    }

    #[test]
    fn scalar_alu_aliasing_inline_and_literal_operands() {
        let cases = sop2_operand_cases();
        assert_eq!(cases.len(), 9 * 10);
        for (code, expected) in &cases {
            verify(code, &[0, u32::MAX], Some(expected));
            assert_translated(code, expected);
        }
    }

    /// All three implementations reject `code`, and at byte `offset`.
    fn rejected_at(code: &[u8], offset: usize) {
        rejected_at_with(code, offset, &[0, 1]);
    }

    fn rejected_at_with(code: &[u8], offset: usize, input: &[u32]) {
        let error = compile(Architecture::Gfx1030, code).unwrap_err();
        let at = match error {
            xenora_shader::CompileError::Verify { offset, .. } => offset,
            xenora_shader::CompileError::Decode(ref decode) => decode.offset,
            ref other => panic!("{other:?}"),
        };
        assert_eq!(at, offset, "{error:?}");
        assert_eq!(xenora_ref::execute(code, input).unwrap_err().offset, offset);
        let independent = independent_ref::execute(code, input).unwrap_err();
        assert!(
            independent.starts_with(&format!("{offset:#x}:")),
            "{independent}"
        );
    }

    #[test]
    fn scalar_alu_rejects_forms_outside_the_subset() {
        // load 8, wait 4, two literal moves 8 each, s_cmp 4: the SOP2 word is at 0x20.
        for opcode in [6, 15, 17, 19] {
            // s_min_i32 and the 64-bit bitwise forms.
            rejected_at(
                &sop2_program(&[sop2(opcode, 10, 8, 9)], 10, 1, 2, Some(false)),
                0x20,
            );
        }
        for opcode in [4, 5] {
            // Without the s_cmp, the carry-in reads SCC before anything set it.
            rejected_at(
                &sop2_program(&[sop2(opcode, 10, 8, 9)], 10, 1, 2, None),
                0x1c,
            );
        }
        for (alu, result) in [
            (&[sop2(0, 4, 8, 9)][..], 9),    // Resource descriptor destination.
            (&[sop2(0, 106, 8, 9)][..], 9),  // VCC_LO destination.
            (&[sop2(0, 10, 8, 11)][..], 9),  // Uninitialized s11.
            (&[sop2(0, 10, 106, 9)][..], 9), // VCC_LO source.
            (&[sop2(0, 10, 255, 255), 0x1234_5678][..], 9), // Two literal sources.
        ] {
            // v2 reads s9, which is valid, so only the SOP2 word can fail.
            rejected_at(&sop2_program(alu, result, 1, 2, Some(false)), 0x20);
        }
        // v_mov_b32 from VCC_LO stays outside the subset.
        let mut code = sop2_program(&[sop2(0, 10, 8, 9)], 10, 1, 2, Some(false));
        assert_eq!(code[0x24..0x28], 0x7e04_020a_u32.to_le_bytes());
        code[0x24] = 106;
        rejected_at(&code, 0x24);
    }

    #[test]
    fn sgpr_moves_need_known_values_even_for_inactive_lanes() {
        let words: [u32; 11] = [
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70, // load v1, wait
            0x7e04_0280, // v_mov_b32 v2, 0
            0xbefe_0382, // s_mov_b32 exec_lo, 2
            0x7e04_0208, // v_mov_b32 v2, s8 (s8 never written)
            0xbefe_03c1, // s_mov_b32 exec_lo, -1
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ];
        let code: Vec<u8> = words[..11]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        // One lane, inactive under EXEC_LO = 2: the interpreters must not skip the check.
        rejected_at_with(&code, 0x14, &[0]);
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn scalar_alu_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let metal = xenora_metal::NativeCompute::new().unwrap();
        let run = |code: &[u8], input: &[u32]| {
            let compiled = compile(Architecture::Gfx1030, code).unwrap();
            metal
                .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, input)
                .unwrap()
        };
        let input: Vec<u32> = (0..64).map(|lane| lane * 0x0101_0101).collect();
        let expected: Vec<_> = input.iter().map(|word| word ^ 0x0000_000e).collect();
        assert_eq!(run(SCALAR_ALU_FIXTURE, &input), expected);
        let mut vectors = sop2_vectors();
        vectors.extend(sop2_operand_cases());
        vectors.extend(signed_immediate_overflows());
        for (code, expected) in &vectors {
            assert_eq!(&run(code, &[0, u32::MAX]), expected);
        }
        eprintln!(
            "{} SOP2 programs matched on native Metal",
            vectors.len() + 1
        );
    }
}
