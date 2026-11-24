// SPDX-License-Identifier: GPL-3.0-or-later

//! Process startup for trusted, original PE32+ fixtures.

use crate::dbt::DbtMetrics;
use crate::{
    CpuState, DbtEngine, ExecutionResult, ImportRegistry, LoadedImage, MemoryError,
    MemoryPermissions, ParsedPe, PeError, RelocationReport, apply_relocations,
    execute_tls_callbacks, parse_tls_directory, setup_tls,
};

const STACK_SIZE: usize = 64 * 1024;
const JIT_CAPACITY: usize = 1024 * 1024;

/// Guest addresses reserved by the fixture harness.
pub struct StartupLayout {
    pub image_base: u64,
    /// End of the fixture stack. Must be 16-byte aligned.
    pub stack_top: u64,
    pub tls_base: u64,
}

pub struct StartupResult {
    pub image: LoadedImage,
    pub relocations: RelocationReport,
    pub bound_imports: usize,
    pub tls_callbacks: usize,
    pub execution: ExecutionResult,
    pub dbt_metrics: DbtMetrics,
}

/// Completed startup phases and the transition to the guest entry.
pub enum StartupEvent {
    Parsed {
        sections: usize,
        entry_rva: u32,
    },
    Mapped {
        entry_va: u64,
    },
    Relocated {
        applied_dir64: usize,
    },
    ImportsBound {
        count: usize,
    },
    TlsCompleted {
        callbacks: usize,
        native_steps: usize,
    },
    Entering {
        step_limit: usize,
    },
}

/// Map, rebase and bind a trusted synthetic fixture, then run its TLS callbacks
/// and entry point. Candidate inspection uses the static probe separately.
pub fn run_synthetic_startup(
    bytes: &[u8],
    layout: StartupLayout,
    imports: ImportRegistry,
    entry_budget: usize,
) -> Result<StartupResult, Box<dyn std::error::Error>> {
    run_synthetic_startup_with_observer(bytes, layout, imports, entry_budget, |_| {})
}

/// Run a trusted synthetic fixture and report milestones in execution order.
pub fn run_synthetic_startup_with_observer(
    bytes: &[u8],
    layout: StartupLayout,
    mut imports: ImportRegistry,
    entry_budget: usize,
    mut observe: impl FnMut(StartupEvent),
) -> Result<StartupResult, Box<dyn std::error::Error>> {
    if !layout.stack_top.is_multiple_of(16) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "fixture stack top must be 16-byte aligned",
        )
        .into());
    }
    let pe = ParsedPe::parse(bytes)?;
    observe(StartupEvent::Parsed {
        sections: pe.sections.len(),
        entry_rva: pe.entry_point_rva,
    });
    let mut image = pe.load_at_base(bytes, layout.image_base)?;
    let memory = &mut image.memory;
    memory
        .fetch_instruction_byte(image.entry_point)
        .map_err(|_| PeError::InvalidEntryPoint)?;
    observe(StartupEvent::Mapped {
        entry_va: image.entry_point,
    });

    let relocations = apply_relocations(&pe, bytes, layout.image_base, memory)?;
    observe(StartupEvent::Relocated {
        applied_dir64: relocations.applied_dir64,
    });
    let bound_imports = imports.bind_imports(&pe, bytes, layout.image_base, memory)?;
    observe(StartupEvent::ImportsBound {
        count: bound_imports,
    });
    let tls = parse_tls_directory(bytes, &pe)?;
    if let Some(tls) = &tls {
        setup_tls(
            tls,
            layout.image_base,
            pe.image_base,
            memory,
            layout.tls_base,
        )?;
    }

    let stack_base = layout
        .stack_top
        .checked_sub(STACK_SIZE as u64)
        .ok_or(MemoryError::Overflow)?;
    memory.map_region(stack_base, STACK_SIZE, MemoryPermissions::READ_WRITE)?;
    let mut engine = DbtEngine::new(JIT_CAPACITY)?;
    engine.set_import_registry(imports);

    let mut tls_cpu = CpuState::new(image.entry_point, layout.stack_top);
    let mut tls_callbacks = 0;
    if let Some(tls) = &tls {
        tls_callbacks = execute_tls_callbacks(
            &tls.callbacks,
            layout.image_base,
            pe.image_base,
            layout.stack_top,
            &mut tls_cpu,
            memory,
            &mut engine,
        )?;
    }

    observe(StartupEvent::TlsCompleted {
        callbacks: tls_callbacks,
        native_steps: engine.metrics.instructions_executed,
    });
    observe(StartupEvent::Entering {
        step_limit: entry_budget,
    });

    // Model function entry with the Win64 32-byte home area and return slot.
    // The synthetic entry point terminates with HLT.
    let mut entry_cpu = CpuState::new(image.entry_point, layout.stack_top - 40);
    let execution = engine.run(&mut entry_cpu, memory, entry_budget);
    Ok(StartupResult {
        image,
        relocations,
        bound_imports,
        tls_callbacks,
        execution,
        dbt_metrics: engine.metrics,
    })
}
