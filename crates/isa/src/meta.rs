//! Complete gfx1030 instruction metadata generated from AMD's public RDNA 2 XML.
//!
//! Identification here establishes an instruction's form and length only. It is
//! not semantic support: [`crate::Instruction`] lists the forms Xenora executes.
//! The XML names literal, SDWA, DPP and NSA variants without saying when they
//! apply, so [`identify`] implements those length rules by hand from the RDNA 2
//! ISA reference guide (see docs/research/rdna2.md).

use std::fmt;

pub use crate::rdna2_meta::{FIELDS, FORMS, SOURCE_SHA256};

/// AMD base encoding of an instruction form.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[allow(missing_docs)]
pub enum EncodingFamily {
    Ds,
    Exp,
    Flat,
    Global,
    Mimg,
    Mtbuf,
    Mubuf,
    Scratch,
    Smem,
    Sop1,
    Sop2,
    Sopc,
    Sopk,
    Sopp,
    Vintrp,
    Vop1,
    Vop2,
    Vop3,
    Vop3Sdst,
    Vop3p,
    Vopc,
}

impl EncodingFamily {
    /// Every family, in table order.
    pub const ALL: [Self; 21] = [
        Self::Ds,
        Self::Exp,
        Self::Flat,
        Self::Global,
        Self::Mimg,
        Self::Mtbuf,
        Self::Mubuf,
        Self::Scratch,
        Self::Smem,
        Self::Sop1,
        Self::Sop2,
        Self::Sopc,
        Self::Sopk,
        Self::Sopp,
        Self::Vintrp,
        Self::Vop1,
        Self::Vop2,
        Self::Vop3,
        Self::Vop3Sdst,
        Self::Vop3p,
        Self::Vopc,
    ];

    /// Lower-case name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ds => "ds",
            Self::Exp => "exp",
            Self::Flat => "flat",
            Self::Global => "global",
            Self::Mimg => "mimg",
            Self::Mtbuf => "mtbuf",
            Self::Mubuf => "mubuf",
            Self::Scratch => "scratch",
            Self::Smem => "smem",
            Self::Sop1 => "sop1",
            Self::Sop2 => "sop2",
            Self::Sopc => "sopc",
            Self::Sopk => "sopk",
            Self::Sopp => "sopp",
            Self::Vintrp => "vintrp",
            Self::Vop1 => "vop1",
            Self::Vop2 => "vop2",
            Self::Vop3 => "vop3",
            Self::Vop3Sdst => "vop3-sdst",
            Self::Vop3p => "vop3p",
            Self::Vopc => "vopc",
        }
    }
}

/// Control-flow flags recorded by the XML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormFlags(pub u8);

impl FormFlags {
    #[allow(missing_docs)]
    pub const NONE: Self = Self(0);
    #[allow(missing_docs)]
    pub const BRANCH: Self = Self(1);
    #[allow(missing_docs)]
    pub const CONDITIONAL: Self = Self(2);
    #[allow(missing_docs)]
    pub const INDIRECT: Self = Self(4);
    #[allow(missing_docs)]
    pub const TERMINATOR: Self = Self(8);
    #[allow(missing_docs)]
    pub const IMMEDIATE: Self = Self(16);

    /// Whether every flag in `other` is set.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Extension words an instruction form may carry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Extensions(pub u8);

impl Extensions {
    #[allow(missing_docs)]
    pub const NONE: Self = Self(0);
    /// A source field may select a trailing 32-bit literal.
    pub const LITERAL: Self = Self(1);
    /// The form always carries a trailing 32-bit literal.
    pub const ALWAYS_LITERAL: Self = Self(2);
    #[allow(missing_docs)]
    pub const SDWA: Self = Self(4);
    #[allow(missing_docs)]
    pub const DPP8: Self = Self(8);
    #[allow(missing_docs)]
    pub const DPP16: Self = Self(16);
    /// MIMG non-sequential address dwords.
    pub const NSA: Self = Self(32);

    /// Whether every extension in `other` is allowed.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// One encoded field: pieces of (bit offset, bit count), least significant piece first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldLayout {
    #[allow(missing_docs)]
    pub name: &'static str,
    #[allow(missing_docs)]
    pub pieces: &'static [(u8, u8)],
}

impl FieldLayout {
    /// Extracts the field from the instruction's first two dwords.
    pub fn extract(&self, value: u64) -> u32 {
        let mut result = 0u64;
        let mut shift = 0;
        for &(offset, count) in self.pieces {
            result |= ((value >> offset) & ((1u64 << count) - 1)) << shift;
            shift += count;
        }
        result as u32
    }
}

/// An operand as described by the XML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub struct OperandInfo {
    pub field: &'static str,
    pub format: &'static str,
    pub kind: &'static str,
    pub bits: u16,
    pub input: bool,
    pub output: bool,
    pub implicit: bool,
    /// Whether the operand occupies microcode bits.
    pub encoded: bool,
}

/// One instruction in one base encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub struct Form {
    pub name: &'static str,
    pub family: EncodingFamily,
    pub opcode: u16,
    /// Identifying bits over the first two dwords (little-endian, first dword low).
    pub mask: u64,
    pub bits: u64,
    /// Base length before literal, SDWA, DPP or NSA words.
    pub bytes: u8,
    pub flags: FormFlags,
    pub extensions: Extensions,
    pub group: &'static str,
    pub operands: &'static [OperandInfo],
}

impl Form {
    /// The base encoding's field layouts.
    pub fn fields(&self) -> &'static [FieldLayout] {
        FIELDS
            .iter()
            .find(|(family, _)| *family == self.family)
            .map(|(_, fields)| *fields)
            .unwrap_or(&[])
    }

    /// The named field's layout, if the encoding has one.
    pub fn field(&self, name: &str) -> Option<&'static FieldLayout> {
        self.fields().iter().find(|field| field.name == name)
    }
}

/// Which extension words an identified instruction carries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Trailer {
    /// A 32-bit literal follows the base encoding.
    pub literal: bool,
    /// SDWA, DPP8 or DPP16 control dword (VOP1, VOP2, VOPC only).
    pub vector_control: Option<VectorControl>,
    /// Extra MIMG address dwords.
    pub nsa_dwords: u8,
}

/// The dword that replaces SRC0 in a VOP1, VOP2 or VOPC instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum VectorControl {
    Sdwa,
    Dpp8 { fetch_inactive: bool },
    Dpp16,
}

/// An identified instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Identified {
    /// The matched form.
    pub form: &'static Form,
    /// First two dwords; the second is zero for 32-bit encodings.
    pub raw: u64,
    /// Total length in bytes, including extension words.
    pub len: usize,
    /// Extension words present.
    pub trailer: Trailer,
}

impl Identified {
    /// Extracts a named field of the base encoding.
    pub fn field(&self, name: &str) -> Option<u32> {
        self.form.field(name).map(|field| field.extract(self.raw))
    }
}

/// Why an instruction could not be identified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifyError {
    /// Fewer bytes remain than the encoding or its extension words need.
    Truncated {
        /// Bytes required from the instruction start.
        needed: usize,
    },
    /// No gfx1030 form matches; the bits are reserved or belong to another architecture.
    Unassigned,
    /// The bits select an extension the form does not allow.
    InvalidExtension(&'static str),
}

impl fmt::Display for IdentifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed } => write!(formatter, "truncated: needs {needed} bytes"),
            Self::Unassigned => formatter.write_str("no gfx1030 instruction has this encoding"),
            Self::InvalidExtension(what) => write!(formatter, "invalid extension: {what}"),
        }
    }
}

impl std::error::Error for IdentifyError {}

/// Source operand value selecting a trailing literal (XML `SRC_LITERAL`).
pub const SRC_LITERAL: u32 = 255;
/// VOP1/VOP2/VOPC SRC0 value selecting an SDWA control dword.
pub const SRC_SDWA: u32 = 0xf9;
/// VOP1/VOP2/VOPC SRC0 value selecting a DPP16 control dword.
pub const SRC_DPP16: u32 = 0xfa;
/// VOP1/VOP2/VOPC SRC0 value selecting a DPP8 control dword.
pub const SRC_DPP8: u32 = 0xe9;
/// DPP8 with fetch-inactive set.
pub const SRC_DPP8_FI: u32 = 0xea;

fn read_dword(bytes: &[u8], index: usize) -> Option<u32> {
    let start = index.checked_mul(4)?;
    let chunk = bytes.get(start..start.checked_add(4)?)?;
    Some(u32::from_le_bytes(chunk.try_into().ok()?))
}

/// Finds the form whose identifying bits match.
///
/// Masks never overlap ambiguously; the generator rejects such tables.
pub fn lookup(raw: u64) -> Option<&'static Form> {
    FORMS.iter().find(|form| raw & form.mask == form.bits)
}

/// Identifies the instruction at the start of `bytes` and computes its length.
pub fn identify(bytes: &[u8]) -> Result<Identified, IdentifyError> {
    let first = read_dword(bytes, 0).ok_or(IdentifyError::Truncated { needed: 4 })?;
    let second = read_dword(bytes, 1);
    let raw_guess = u64::from(first) | (u64::from(second.unwrap_or(0)) << 32);
    // A 32-bit form never inspects the second dword; a 64-bit form needs it.
    let form = FORMS
        .iter()
        .find(|form| (form.bytes == 4 || second.is_some()) && raw_guess & form.mask == form.bits)
        .or_else(|| {
            // Distinguish a truncated 64-bit instruction from unassigned bits.
            FORMS.iter().find(|form| {
                form.bytes == 8
                    && u64::from(first) & form.mask & 0xffff_ffff == form.bits & 0xffff_ffff
            })
        })
        .ok_or(IdentifyError::Unassigned)?;
    if form.bytes == 8 && second.is_none() {
        return Err(IdentifyError::Truncated { needed: 8 });
    }
    let raw = if form.bytes == 4 {
        u64::from(first)
    } else {
        raw_guess
    };
    let mut trailer = Trailer::default();
    let source = |name: &str| form.field(name).map(|field| field.extract(raw));
    match form.family {
        EncodingFamily::Sop1 | EncodingFamily::Sop2 | EncodingFamily::Sopc => {
            trailer.literal = [source("SSRC0"), source("SSRC1")]
                .into_iter()
                .flatten()
                .any(|value| value == SRC_LITERAL);
        }
        EncodingFamily::Vop1 | EncodingFamily::Vop2 | EncodingFamily::Vopc => {
            match source("SRC0") {
                Some(SRC_LITERAL) => trailer.literal = true,
                Some(SRC_SDWA) => trailer.vector_control = Some(VectorControl::Sdwa),
                Some(SRC_DPP16) => trailer.vector_control = Some(VectorControl::Dpp16),
                Some(SRC_DPP8) => {
                    trailer.vector_control = Some(VectorControl::Dpp8 {
                        fetch_inactive: false,
                    })
                }
                Some(SRC_DPP8_FI) => {
                    trailer.vector_control = Some(VectorControl::Dpp8 {
                        fetch_inactive: true,
                    })
                }
                _ => {}
            }
        }
        EncodingFamily::Vop3 | EncodingFamily::Vop3Sdst | EncodingFamily::Vop3p => {
            // An attribute field (XML OPR_ATTR) is not an operand selector: in the
            // VOP3 interpolation forms SRC0 holds {attr_word, attr_chan, attr}
            // bit-fields (AMD RDNA 2 ISA guide, section 10.4.2), so 255 there
            // does not select a literal.
            trailer.literal = ["SRC0", "SRC1", "SRC2"]
                .into_iter()
                .filter(|name| {
                    !form
                        .operands
                        .iter()
                        .any(|operand| operand.field == *name && operand.kind == "OPR_ATTR")
                })
                .filter_map(source)
                .any(|value| value == SRC_LITERAL);
        }
        EncodingFamily::Mimg => {
            trailer.nsa_dwords = source("NSA").unwrap_or(0) as u8;
        }
        _ => {}
    }
    if form.extensions.contains(Extensions::ALWAYS_LITERAL) {
        trailer.literal = true;
    } else if trailer.literal && !form.extensions.contains(Extensions::LITERAL) {
        return Err(IdentifyError::InvalidExtension("literal"));
    }
    match trailer.vector_control {
        Some(VectorControl::Sdwa) if !form.extensions.contains(Extensions::SDWA) => {
            return Err(IdentifyError::InvalidExtension("sdwa"));
        }
        Some(VectorControl::Dpp8 { .. }) if !form.extensions.contains(Extensions::DPP8) => {
            return Err(IdentifyError::InvalidExtension("dpp8"));
        }
        Some(VectorControl::Dpp16) if !form.extensions.contains(Extensions::DPP16) => {
            return Err(IdentifyError::InvalidExtension("dpp16"));
        }
        _ => {}
    }
    if trailer.nsa_dwords > 0 && !form.extensions.contains(Extensions::NSA) {
        return Err(IdentifyError::InvalidExtension("nsa"));
    }
    let len = usize::from(form.bytes)
        + 4 * usize::from(trailer.literal)
        + 4 * usize::from(trailer.vector_control.is_some())
        + 4 * usize::from(trailer.nsa_dwords);
    if bytes.len() < len {
        return Err(IdentifyError::Truncated { needed: len });
    }
    Ok(Identified {
        form,
        raw,
        len,
        trailer,
    })
}

/// Number of forms per encoding family in the metadata.
pub fn inventory() -> Vec<(EncodingFamily, usize)> {
    EncodingFamily::ALL
        .iter()
        .map(|&family| {
            (
                family,
                FORMS.iter().filter(|form| form.family == family).count(),
            )
        })
        .collect()
}
