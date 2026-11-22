// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod independent_ref;

#[cfg(test)]
mod review {
    use super::independent_ref;
    use xenora_shader::{Architecture, compile};

    const FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/add.bin");

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
        let fixtures: [(&[u8], &str); 3] = [
            (include_bytes!("../../fixtures/rdna2/add.bin"), "add"),
            (
                include_bytes!("../../fixtures/rdna2/bitwise-branch.bin"),
                "bitwise-branch",
            ),
            (
                include_bytes!("../../fixtures/rdna2/float-exact.bin"),
                "float-exact",
            ),
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
            &[0xbf82_0001, 0xbeea_0380],              // Reserved VCC_LO instead of normal SGPR.
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
}
