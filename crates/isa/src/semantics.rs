//! Typed vocabulary of the declarative semantic registry (`crates/isa/semantics/rdna2.toml`).
//!
//! The registry is data. `crates/isa/generate_semantics.py` validates it against
//! this closed vocabulary and the pinned AMD XML snapshot, then emits
//! `semantics_gen.rs`: the typed operation enums, their opcode dispatch and one
//! static [`FormSemantics`] row per form. Nothing here interprets metadata at run
//! time; the decoder and translator call the generated `const fn`s and `match`es.
//!
//! The reference interpreter (`crates/ref`) and the independent interpreter
//! (`tests/validation/independent_ref.rs`) must not use this module.

use crate::meta::EncodingFamily;

/// Version of the registry schema this crate was generated against.
pub const SCHEMA: u32 = 1;

/// Reusable semantic primitive selected by a form. Closed: the generator rejects
/// any other name, and each primitive has one implementation in `primitive.rs`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Integer addition.
    Add,
    /// Integer subtraction.
    Sub,
    /// Bitwise AND.
    And,
    /// Bitwise OR.
    Or,
    /// Bitwise exclusive OR.
    Xor,
}

/// How the operands are interpreted. `Bitwise` exactly when the operation is logical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signedness {
    /// Unsigned integers.
    Unsigned,
    /// Two's-complement integers.
    Signed,
    /// Untyped bits.
    Bitwise,
}

/// Extra input consumed by the operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CarryIn {
    /// No carry or borrow input.
    None,
    /// SCC as carry-in (add) or borrow-in (sub).
    Scc,
}

/// What a form writes to SCC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SccEffect {
    /// SCC is not written.
    Unchanged,
    /// Unsigned carry-out of the top bit.
    CarryOut,
    /// Unsigned borrow (subtrahend plus borrow-in exceeds the minuend).
    Borrow,
    /// Two's-complement overflow of the exact result.
    SignedOverflow,
    /// The result is not zero.
    Nonzero,
}

/// What a form writes to VCC or EXEC. Stage A forms leave both unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaskEffect {
    /// Not written.
    Unchanged,
}

/// Wave-size restriction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wave {
    /// Behaviour does not depend on the wave size.
    Any,
    /// Only Wave32 behaviour is declared; Wave64 stays unsupported.
    Wave32,
}

/// Shared operand rule. The decoder implements each rule once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperandRule {
    /// SOP2: SDST is s0-s105; SSRC0/SSRC1 are s0-s105, inline integers -16..64
    /// or one literal word. Everything else is rejected as unsupported.
    Sop2Sgpr,
}

/// Evidence a form needs before it counts as verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Evidence {
    /// `crates/ref` lockstep reference.
    Reference,
    /// `tests/validation/independent_ref.rs`.
    Independent,
    /// Native Metal known-answer runs.
    Metal,
    /// LLVM assembler/disassembler encoding agreement.
    LlvmEncoding,
}

/// Public document a [`SpecRef`] points at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Document {
    /// AMD RDNA 2 ISA reference guide (PDF).
    Guide,
    /// Pinned AMD machine-readable XML.
    Xml,
}

/// One public specification reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpecRef {
    /// Source document.
    pub document: Document,
    /// Section number, for example `"12.1"`.
    pub section: &'static str,
    /// Page in the document, 0 when not paginated.
    pub page: u16,
}

/// One validated registry row, emitted as a static table by the generator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormSemantics {
    /// XML instruction name, the stable form identity.
    pub name: &'static str,
    /// Encoding family (matches [`crate::meta::FORMS`]).
    pub family: EncodingFamily,
    /// Opcode within the family.
    pub opcode: u16,
    /// Primitive.
    pub operation: Operation,
    /// Operand interpretation.
    pub signedness: Signedness,
    /// Carry or borrow input.
    pub carry_in: CarryIn,
    /// Operand and result width in bits.
    pub width: u8,
    /// SCC write.
    pub scc: SccEffect,
    /// VCC write.
    pub vcc: MaskEffect,
    /// EXEC write.
    pub exec: MaskEffect,
    /// Wave-size restriction.
    pub wave: Wave,
    /// Operand rule.
    pub operands: OperandRule,
    /// Public references.
    pub spec: &'static [SpecRef],
    /// Required evidence.
    pub verify: &'static [Evidence],
}
