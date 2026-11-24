// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent guest rendering for one session (Gate 3, #266).
//!
//! The embedded `frame-loop.exe` fixture calls `xenora.dll!frame_submit(index)`
//! with index 0, 1, 2, ... and halts when the call returns 0. The guest runs on
//! its own thread for the session's lifetime. Pause, resume and stop act at the
//! frame-submit boundary: a paused guest waits inside that call with its
//! architectural state untouched, and a stop request makes the call return 0 so
//! the guest leaves its loop and halts.

use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use xenora_loader::startup::{StartupLayout, run_synthetic_startup};
use xenora_loader::{ExitReason, ImportRegistry, Register};
use xenora_metal::NativeCompute;
use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/pe/frame-loop.exe");
pub const WIDTH: usize = 4;
pub const HEIGHT: usize = 3;
pub const FRAME_BYTES: usize = WIDTH * HEIGHT * 4;
pub const MAX_FRAMES: u32 = 100_000;
/// Guest instructions per loop iteration (mov, call, inc, test, jnz) plus slack.
const STEPS_PER_FRAME: usize = 8;
/// `x9-constant-ps.s` exports (1, 0, 0, 1): opaque red. Everything else is the clear colour.
const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [19, 43, 93, 251];

pub const RUNNING: u32 = 1;
pub const PAUSED: u32 = 2;
pub const FINISHED: u32 = 3;
pub const STOPPED: u32 = 4;
pub const FAILED: u32 = 5;

#[derive(Default)]
struct State {
    status: u32,
    paused: bool,
    stop: bool,
    limit: u32,
    completed: u32,
    /// Every `frame_submit` call the guest made, including the one answered with 0.
    calls: u64,
    latest: Option<(u32, Vec<u8>)>,
}

#[derive(Default)]
pub struct FrameLoop {
    state: Mutex<State>,
    wake: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

pub struct Snapshot {
    pub status: u32,
    pub completed: u32,
    pub latest: Option<(u32, Vec<u8>)>,
}

impl FrameLoop {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn start(limit: u32) -> Arc<Self> {
        let shared = Arc::new(Self::default());
        {
            let mut state = shared.lock();
            state.status = RUNNING;
            state.limit = limit;
        }
        let guest = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("xenora-guest".into())
            .spawn(move || guest.run())
            .ok();
        if handle.is_none() {
            shared.lock().status = FAILED;
        }
        *shared.thread.lock().unwrap_or_else(|p| p.into_inner()) = handle;
        shared
    }

    /// Pause or resume. Returns false unless the loop is running or paused.
    pub fn set_paused(&self, paused: bool) -> bool {
        let mut state = self.lock();
        if !matches!(state.status, RUNNING | PAUSED) {
            return false;
        }
        state.paused = paused;
        state.status = if paused { PAUSED } else { RUNNING };
        self.wake.notify_all();
        true
    }

    /// Ask the guest to stop at its next frame submission and wait for its thread.
    pub fn stop(&self) {
        {
            let mut state = self.lock();
            state.stop = true;
            self.wake.notify_all();
        }
        if let Some(handle) = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = handle.join();
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.lock();
        Snapshot {
            status: state.status,
            completed: state.completed,
            latest: state.latest.clone(),
        }
    }

    fn run(self: Arc<Self>) {
        let outcome = self.execute();
        let mut state = self.lock();
        state.status = match outcome {
            Err(_) => FAILED,
            Ok(()) if state.stop && state.completed < state.limit => STOPPED,
            Ok(()) => FINISHED,
        };
    }

    fn execute(self: &Arc<Self>) -> Result<(), String> {
        let shader = compile_graphics(
            Architecture::Gfx1030,
            include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
            include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
        )
        .map_err(|e| format!("shader translation: {e}"))?;
        let gpu = NativeCompute::new().map_err(|e| format!("Metal device: {e}"))?;
        // The session's GPU resources live on the guest thread, where the import runs.
        GPU.with(|slot| *slot.borrow_mut() = Some((gpu, shader.msl)));

        let shared = Arc::clone(self);
        let mut imports = ImportRegistry::new();
        imports.register("xenora.dll", "frame_submit", move |args, _, memory| {
            let index = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
            shared
                .submit(index)
                .ok_or(ExitReason::IllegalInstruction(0xff))
        });
        let limit = self.lock().limit as usize;
        let startup = run_synthetic_startup(
            FIXTURE,
            StartupLayout {
                image_base: 0x1_4000_0000,
                stack_top: 0x2_0001_0000,
                tls_base: 0x3_0000_0000,
            },
            imports,
            (limit + 1) * STEPS_PER_FRAME + 16,
        )
        .map_err(|e| format!("startup: {e}"));
        GPU.with(|slot| slot.borrow_mut().take());
        let startup = startup?;
        let calls = self.lock().calls;
        let rbx = startup.execution.final_state.get(Register::Rbx);
        // The guest increments RBX after every call, so it must equal the calls we served.
        if startup.execution.exit_reason != ExitReason::Halt || rbx != calls {
            return Err(format!(
                "guest ended with {:?}, rbx={rbx}, calls={calls}",
                startup.execution.exit_reason
            ));
        }
        Ok(())
    }

    /// Handle one `frame_submit`: wait while paused, then render and verify the
    /// frame. `None` fails the guest; `Some(0)` tells it to stop.
    fn submit(&self, index: u64) -> Option<u64> {
        let mut state = self.lock();
        state.calls += 1;
        while state.paused && !state.stop {
            state = self.wake.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        if state.stop || state.completed >= state.limit {
            return Some(0);
        }
        if index != u64::from(state.completed) {
            return None; // frame identities must arrive in order
        }
        drop(state);
        let pixels = render(index as u32)?;
        let mut state = self.lock();
        state.completed += 1;
        state.latest = Some((index as u32, pixels));
        Some(u64::from(state.completed < state.limit && !state.stop))
    }
}

thread_local! {
    static GPU: RefCell<Option<(NativeCompute, String)>> = const { RefCell::new(None) };
}

/// Frame `index` is a red strip in column `index % 4` on the clear colour.
///
/// The triangle (x0, -10), (x0 + 0.7, 0), (x0, 10) in clip space, with x0 the
/// column's left edge, covers that column's pixel centres in all three rows and
/// none of its neighbours'.
fn render(index: u32) -> Option<Vec<u8>> {
    let column = (index as usize % WIDTH) as f32;
    let x0 = -1.0 + column * (2.0 / WIDTH as f32);
    let pixels = GPU.with(|slot| {
        let slot = slot.borrow();
        let (gpu, msl) = slot.as_ref()?;
        gpu.render_rgba8(
            msl,
            [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
            &[
                [x0, -10.0, 0.5, 1.0],
                [x0 + 0.7, 0.0, 0.5, 1.0],
                [x0, 10.0, 0.5, 1.0],
            ],
            &[[0.0; 4]; 3],
            [WIDTH, HEIGHT],
            CLEAR,
        )
        .ok()
    })?;
    (pixels == expected(index)).then_some(pixels)
}

/// The expected frame for `index`, from the strip rule alone.
pub fn expected(index: u32) -> Vec<u8> {
    (0..WIDTH * HEIGHT)
        .flat_map(|pixel| {
            if pixel % WIDTH == index as usize % WIDTH {
                RED
            } else {
                CLEAR
            }
        })
        .collect()
}
