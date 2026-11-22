// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod review {
    use xenora_shader::{Architecture, compile};

    const FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/add.bin");

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
}
