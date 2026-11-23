// SPDX-License-Identifier: GPL-3.0-or-later
use crate::cpu::{self, Fault, State};

pub(crate) const FIXTURE: &[u8] = include_bytes!("../fixtures/x2.exe");
const IMAGE_BASE: u64 = 0x1_4000_0000;
const FILE_ALIGNMENT: usize = 0x200;
const SECTION_ALIGNMENT: u32 = 0x1000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LoadError {
    Bounds,
    Header,
    Directory,
    Section,
    Entry,
}

/// Type alias for the handler closure stored in a service registry entry.
#[allow(clippy::type_complexity)]
pub(crate) type HandlerFn = Box<dyn FnMut(&State) -> Result<u64, cpu::FaultKind>>;

/// A registered service entry: validates arguments from the guest `State`,
/// then returns a result or a `FaultKind` on invalid input.
///
/// The service receives the full register state at the point of the UD2 trap.
/// By convention `state.gpr[0]` (EAX) holds the service identifier that was
/// used to select this entry; `state.gpr[1]` (RCX) carries the primary
/// argument.  Additional general-purpose registers are available but their
/// use must be defined per-service.
pub(crate) struct ServiceEntry {
    /// Human-readable name for diagnostics (printed by the CLI entrypoint).
    #[allow(dead_code)]
    pub(crate) name: &'static str,
    /// Handler invoked on each trap.  Returns the value to place in RAX, or a
    /// `FaultKind` to propagate as a fault (the trap instruction is not
    /// retired).
    pub(crate) handler: HandlerFn,
}

/// Typed service registry: maps service IDs (the EAX value at UD2) to
/// validated handlers.  IDs must be non-zero; zero is reserved.
///
/// # Design constraints
///
/// - Each entry is called at most once per execution (the caller may enforce
///   this via `submitted` guards in higher-level dispatch).
/// - Handler argument validation is the responsibility of the entry; the
///   registry only dispatches by ID.
/// - An unknown or zero ID produces `FaultKind::UnknownService`.
pub(crate) struct ServiceRegistry {
    entries: Vec<(u64, ServiceEntry)>,
}

impl ServiceRegistry {
    /// Construct an empty registry.
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Register a service under `id`.  Panics if `id` is 0 or already taken.
    pub(crate) fn register(&mut self, id: u64, entry: ServiceEntry) {
        assert!(id != 0, "service ID 0 is reserved");
        assert!(
            !self.entries.iter().any(|(k, _)| *k == id),
            "duplicate service ID {id}"
        );
        self.entries.push((id, entry));
    }

    /// Dispatch a trap: look up the ID in `state.gpr[0]` (EAX) and invoke the
    /// matching handler.  Returns `FaultKind::UnknownService` if no entry
    /// matches.
    fn dispatch(&mut self, state: &State) -> Result<u64, cpu::FaultKind> {
        let id = state.gpr[0] & 0xffff_ffff; // EAX
        for (k, entry) in &mut self.entries {
            if *k == id {
                return (entry.handler)(state);
            }
        }
        Err(cpu::FaultKind::UnknownService)
    }
}

#[derive(Debug)]
struct Code<'a> {
    base: u64,
    bytes: &'a [u8],
}

impl<'a> Code<'a> {
    fn fetch_from(&self, address: u64) -> Option<&'a [u8]> {
        let offset = usize::try_from(address.checked_sub(self.base)?).ok()?;
        self.bytes.get(offset..).filter(|bytes| !bytes.is_empty())
    }
}

#[derive(Debug)]
pub(crate) struct Image<'a> {
    code: Code<'a>,
    entry: u64,
    data_base: u64,
    pub(crate) data: [u8; 8],
}

fn bytes<const N: usize>(input: &[u8], offset: usize) -> Result<[u8; N], LoadError> {
    input
        .get(offset..offset.checked_add(N).ok_or(LoadError::Bounds)?)
        .and_then(|value| value.try_into().ok())
        .ok_or(LoadError::Bounds)
}

fn word(input: &[u8], offset: usize) -> Result<u16, LoadError> {
    Ok(u16::from_le_bytes(bytes(input, offset)?))
}

fn dword(input: &[u8], offset: usize) -> Result<u32, LoadError> {
    Ok(u32::from_le_bytes(bytes(input, offset)?))
}

fn section<'a>(
    header: &[u8],
    image: &'a [u8],
    name: [u8; 8],
    rva: u32,
    raw: usize,
    flags: u32,
) -> Result<&'a [u8], LoadError> {
    let size = dword(header, 8)? as usize;
    if bytes(header, 0)? != name
        || dword(header, 12)? != rva
        || dword(header, 16)? as usize != FILE_ALIGNMENT
        || dword(header, 20)? as usize != raw
        || bytes::<12>(header, 24)? != [0; 12]
        || dword(header, 36)? != flags
        || !(1..=64).contains(&size)
    {
        return Err(LoadError::Section);
    }
    let raw = image
        .get(raw..raw + FILE_ALIGNMENT)
        .ok_or(LoadError::Bounds)?;
    Ok(&raw[..size])
}

pub(crate) fn load(image: &[u8]) -> Result<Image<'_>, LoadError> {
    load_profile(image, IMAGE_BASE, 2).map(|(image, _)| image)
}

#[cfg(any(test, feature = "metal"))]
pub(crate) fn load_compute(image: &[u8]) -> Result<(Image<'_>, u64, &[u8; 48]), LoadError> {
    let base = 0x400000;
    let (image, read_only) = load_profile(image, base, 3)?;
    Ok((
        image,
        base + 3 * u64::from(SECTION_ALIGNMENT),
        read_only.try_into().map_err(|_| LoadError::Section)?,
    ))
}

fn load_profile(
    image: &[u8],
    base: u64,
    section_count: u16,
) -> Result<(Image<'_>, &[u8]), LoadError> {
    let pages = usize::from(section_count) + 1;
    if image.len() != pages * FILE_ALIGNMENT {
        return Err(LoadError::Bounds);
    }
    if bytes::<2>(image, 0)? != *b"MZ" {
        return Err(LoadError::Header);
    }
    let pe = dword(image, 0x3c)? as usize;
    // The section table must fit in the header page before any offsets are used.
    let table_end = 264 + 40 * usize::from(section_count);
    if pe < 0x40 || !pe.is_multiple_of(8) || pe > FILE_ALIGNMENT - table_end {
        return Err(LoadError::Header);
    }
    let coff = &image[pe..pe + 24];
    if bytes::<4>(coff, 0)? != *b"PE\0\0"
        || word(coff, 4)? != 0x8664
        || word(coff, 6)? != section_count
        || bytes::<8>(coff, 12)? != [0; 8]
        || word(coff, 20)? != 240
        || word(coff, 22)? != 0x23
    {
        return Err(LoadError::Header);
    }
    let optional = &image[pe + 24..pe + 264];
    if word(optional, 0)? != 0x20b
        || dword(optional, 4)? as usize != FILE_ALIGNMENT
        || dword(optional, 8)? as usize != (pages - 2) * FILE_ALIGNMENT
        || dword(optional, 12)? != 0
        || dword(optional, 20)? != SECTION_ALIGNMENT
        || u64::from_le_bytes(bytes(optional, 24)?) != base
        || dword(optional, 32)? != SECTION_ALIGNMENT
        || dword(optional, 36)? as usize != FILE_ALIGNMENT
        || dword(optional, 52)? != 0
        || dword(optional, 56)? != (u32::from(section_count) + 1) * SECTION_ALIGNMENT
        || dword(optional, 60)? as usize != FILE_ALIGNMENT
        || word(optional, 68)? != 3
        || word(optional, 70)? != 0
        || dword(optional, 104)? != 0
    {
        return Err(LoadError::Header);
    }
    if dword(optional, 108)? != 16 || optional[112..].iter().any(|&byte| byte != 0) {
        return Err(LoadError::Directory);
    }
    let table = &image[pe + 264..pe + table_end];
    // The profile requires distinct aligned raw pages and distinct aligned RVAs.
    let code = section(
        &table[..40],
        image,
        *b".text\0\0\0",
        SECTION_ALIGNMENT,
        FILE_ALIGNMENT,
        0x6000_0020,
    )?;
    let data = section(
        &table[40..80],
        image,
        *b".data\0\0\0",
        2 * SECTION_ALIGNMENT,
        2 * FILE_ALIGNMENT,
        0xc000_0040,
    )?;
    let entry_rva = dword(optional, 16)?;
    // This profile allows entry only at the start of the non-empty code region.
    if entry_rva != SECTION_ALIGNMENT {
        return Err(LoadError::Entry);
    }
    let read_only = if section_count == 3 {
        section(
            &table[80..],
            image,
            *b".rdata\0\0",
            3 * SECTION_ALIGNMENT,
            3 * FILE_ALIGNMENT,
            0x4000_0040,
        )?
    } else {
        &[]
    };
    Ok((
        Image {
            code: Code {
                base: base + u64::from(SECTION_ALIGNMENT),
                bytes: code,
            },
            entry: base + u64::from(entry_rva),
            data_base: base + 2 * u64::from(SECTION_ALIGNMENT),
            data: data.try_into().map_err(|_| LoadError::Section)?,
        },
        read_only,
    ))
}

impl Image<'_> {
    pub(crate) fn initial(&self, argument: u64) -> State {
        let mut state = State {
            gpr: [0; 16],
            rip: self.entry,
            rflags: 2,
        };
        state.gpr[1] = argument;
        state.gpr[7] = self.data_base;
        state
    }

    pub(crate) fn run(
        &mut self,
        state: &mut State,
        budget: usize,
        retired: impl FnMut(&State),
    ) -> Result<(), Fault> {
        self.run_with_trap(
            state,
            budget,
            &mut |state| {
                if state.gpr[0] != 1 {
                    return Err(cpu::FaultKind::UnknownService);
                }
                Ok(state.gpr[1].wrapping_add(1))
            },
            retired,
        )
    }

    /// Run with a typed service registry.  Each UD2 trap dispatches through
    /// `registry` by EAX value.  An unknown ID or handler error faults with
    /// the trap instruction unretired.
    pub(crate) fn run_with_registry(
        &mut self,
        state: &mut State,
        budget: usize,
        registry: &mut ServiceRegistry,
        retired: impl FnMut(&State),
    ) -> Result<(), Fault> {
        self.run_with_trap(
            state,
            budget,
            &mut |state| registry.dispatch(state),
            retired,
        )
    }

    pub(crate) fn run_with_trap(
        &mut self,
        state: &mut State,
        budget: usize,
        trap: &mut impl FnMut(&State) -> Result<u64, cpu::FaultKind>,
        retired: impl FnMut(&State),
    ) -> Result<(), Fault> {
        let code = self
            .code
            .fetch_from(state.rip)
            .filter(|_| state.rip == self.entry)
            .ok_or(Fault {
                kind: cpu::FaultKind::CodeRange,
                rip: state.rip,
            })?;
        cpu::execute(
            code,
            state,
            self.data_base,
            &mut self.data,
            budget,
            Some(trap),
            retired,
        )
    }
}

#[cfg(test)]
mod tests;
