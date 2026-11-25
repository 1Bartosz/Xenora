// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest-callable graphics command path exported from `xenora_graphics.dll`.
//!
//! The D3D12-style runtime in [`crate::graphics`] is complete, but a guest
//! program could not reach it: the only guest-reachable path was the rigid
//! `xenora_fixture.dll!SubmitGuestDraw` helper, which created its own
//! objects and issued one fixed three-vertex draw. This module exposes the
//! real command path to guest code: device, queue, allocator, command list,
//! buffer and render-target resources, state binding, draws, execution,
//! framebuffer readback and ordered destruction.
//!
//! Every export uses the Win64 calling convention and returns an `HRESULT`
//! in `RAX`. Object handles are opaque generation-checked `u64` values that
//! the guest passes back exactly as it received them; out-parameters are
//! guest pointers. A recoverable misuse is reported as a failure `HRESULT`,
//! never as a guest fault, so a bad argument cannot stop the guest.

use super::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ServicesState};
use crate::cpu::ExitReason;
use crate::graphics::{
    CommandListType, DXGI_FORMAT_R8G8B8A8_UNORM, GraphicsBackendId, GraphicsContext, GraphicsError,
    GraphicsHandle, IndexFormat, ResourceDesc, ResourceDimension, ResourceStates,
};
use crate::imports::{ImportRegistry, ServiceStatus, Win64Args};
use crate::memory::{GuestMemory, MemoryError};
use crate::runtime::HResult;
use std::sync::{Arc, MutexGuard};

const MODULE: &str = "xenora_graphics.dll";

/// The one supported vertex layout: one `float4` position per vertex.
const VERTEX_STRIDE: usize = 16;
/// The largest viewport the guest may ask for.
const MAX_VIEWPORT_DIMENSION: u32 = 4096;
/// Backend id of the deterministic software reference rasteriser.
const BACKEND_SOFTWARE_REFERENCE: u32 = 0;
/// Backend id of the native Metal backend.
const BACKEND_METAL_NATIVE: u32 = 1;

/// `S_OK`.
fn ok() -> u64 {
    HResult::OK.guest_return()
}

/// `E_INVALIDARG`.
fn invalid_arg() -> u64 {
    HResult::from_win32(ERROR_INVALID_PARAMETER).guest_return()
}

/// `E_ACCESSDENIED`.
fn access_denied() -> u64 {
    HResult::from_win32(ERROR_ACCESS_DENIED).guest_return()
}

/// The `HRESULT` a runtime error maps to.
fn hresult(error: &GraphicsError) -> u64 {
    match error {
        GraphicsError::InvalidDeviceHandle
        | GraphicsError::InvalidQueueHandle
        | GraphicsError::InvalidAllocatorHandle
        | GraphicsError::InvalidCommandListHandle
        | GraphicsError::InvalidResourceHandle => invalid_arg(),
        GraphicsError::ResourceLifetimeViolation => access_denied(),
        _ => HResult::FAIL.guest_return(),
    }
}

/// Whether an error means the handle the guest passed does not name an
/// object of the expected kind.
fn is_invalid_handle(error: &GraphicsError) -> bool {
    matches!(
        error,
        GraphicsError::InvalidDeviceHandle
            | GraphicsError::InvalidQueueHandle
            | GraphicsError::InvalidAllocatorHandle
            | GraphicsError::InvalidCommandListHandle
            | GraphicsError::InvalidResourceHandle
    )
}

/// The handle values a guest argument may name: the value itself, and, when
/// the value does not name an object, the handle stored at that address.
/// The second form lets a guest pass a pointer to a handle slot.
fn handle_candidates(raw: u64, memory: &GuestMemory) -> Vec<u64> {
    let mut candidates = vec![raw];
    if raw != 0
        && let Ok(stored) = memory.read_u64(raw)
        && stored != raw
    {
        candidates.push(stored);
    }
    candidates
}

/// Run `op` with the handle the guest passed, reporting the error of the
/// last candidate when none of them names an object.
fn with_handle<T>(
    raw: u64,
    memory: &GuestMemory,
    missing: GraphicsError,
    mut op: impl FnMut(GraphicsHandle) -> Result<T, GraphicsError>,
) -> Result<T, GraphicsError> {
    let mut last = missing;
    for candidate in handle_candidates(raw, memory) {
        match op(GraphicsHandle::from_u64(candidate)) {
            Ok(value) => return Ok(value),
            Err(error) => {
                if is_invalid_handle(&error) {
                    last = error;
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(last)
}

/// The shared graphics runtime, locked for one call.
fn lock_graphics(
    state: &Arc<ServicesState>,
) -> Result<MutexGuard<'_, GraphicsContext>, ExitReason> {
    state
        .graphics
        .lock()
        .map_err(|_| ExitReason::MemoryFault(MemoryError::Overflow))
}

/// Create a device and write its handle to `*ppDevice`.
fn xenora_create_device(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let out = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    if out == 0 {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let device = match graphics.create_device() {
        Ok(device) => device,
        Err(error) => return Ok(hresult(&error)),
    };
    if memory.write_u64(out, device.to_u64()).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Create a command queue for a device.
fn xenora_create_command_queue(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let device = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let list_type = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let out = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
    if list_type != 0 || out == 0 {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let queue = match with_handle(
        device,
        memory,
        GraphicsError::InvalidDeviceHandle,
        |handle| graphics.create_command_queue(handle, CommandListType::Direct),
    ) {
        Ok(queue) => queue,
        Err(error) => return Ok(hresult(&error)),
    };
    if memory.write_u64(out, queue.to_u64()).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Create a command allocator for a device.
fn xenora_create_command_allocator(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let device = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let list_type = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let out = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
    if list_type != 0 || out == 0 {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let allocator = match with_handle(
        device,
        memory,
        GraphicsError::InvalidDeviceHandle,
        |handle| graphics.create_command_allocator(handle, CommandListType::Direct),
    ) {
        Ok(allocator) => allocator,
        Err(error) => return Ok(hresult(&error)),
    };
    if memory.write_u64(out, allocator.to_u64()).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Create a recording command list.
fn xenora_create_command_list(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let device = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let allocator = args.get(1, memory).map_err(ExitReason::MemoryFault)?;
    let list_type = args.get(2, memory).map_err(ExitReason::MemoryFault)? as u32;
    let out = args.get(3, memory).map_err(ExitReason::MemoryFault)?;
    if list_type != 0 || out == 0 {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let list = match with_handle(
        device,
        memory,
        GraphicsError::InvalidDeviceHandle,
        |handle| {
            with_handle(
                allocator,
                memory,
                GraphicsError::InvalidAllocatorHandle,
                |allocator_handle| {
                    graphics.create_command_list(handle, allocator_handle, CommandListType::Direct)
                },
            )
        },
    ) {
        Ok(list) => list,
        Err(error) => return Ok(hresult(&error)),
    };
    if memory.write_u64(out, list.to_u64()).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Create a buffer whose contents are the guest bytes at `guestAddr`.
fn xenora_create_buffer(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let device = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let guest_addr = args.get(1, memory).map_err(ExitReason::MemoryFault)?;
    let byte_length = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
    let out = args.get(3, memory).map_err(ExitReason::MemoryFault)?;
    if out == 0 || guest_addr == 0 || byte_length == 0 {
        return Ok(invalid_arg());
    }
    let desc = ResourceDesc {
        dimension: ResourceDimension::Buffer,
        width: byte_length,
        height: 1,
        depth_or_array_size: 1,
        mip_levels: 1,
        format_rgba8: false,
    };
    let mut graphics = lock_graphics(state)?;
    let resource = match with_handle(
        device,
        memory,
        GraphicsError::InvalidDeviceHandle,
        |handle| {
            graphics.create_committed_resource(
                handle,
                desc.clone(),
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                Some(guest_addr),
            )
        },
    ) {
        Ok(resource) => resource,
        Err(error) => return Ok(hresult(&error)),
    };
    if memory.write_u64(out, resource.to_u64()).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Create an `R8G8B8A8_UNORM` two-dimensional render target.
fn xenora_create_render_target(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let device = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let width = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let height = args.get(2, memory).map_err(ExitReason::MemoryFault)? as u32;
    let out = args.get(3, memory).map_err(ExitReason::MemoryFault)?;
    if out == 0 || width == 0 || height == 0 {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let resource = match with_handle(
        device,
        memory,
        GraphicsError::InvalidDeviceHandle,
        |handle| {
            graphics.create_committed_texture2d(
                handle,
                width,
                height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                ResourceStates::RENDER_TARGET,
                None,
            )
        },
    ) {
        Ok(resource) => resource,
        Err(error) => return Ok(hresult(&error)),
    };
    if memory.write_u64(out, resource.to_u64()).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Bind a vertex buffer at `slot`.
fn xenora_set_vertex_buffer(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let slot = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let resource = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
    let offset = args.get(3, memory).map_err(ExitReason::MemoryFault)? as usize;
    if slot != 0 {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| {
            with_handle(
                resource,
                memory,
                GraphicsError::InvalidResourceHandle,
                |resource_handle| {
                    graphics.cmd_set_vertex_buffer(
                        list_handle,
                        resource_handle,
                        offset,
                        VERTEX_STRIDE,
                    )
                },
            )
        },
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Bind an index buffer.
fn xenora_set_index_buffer(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let resource = args.get(1, memory).map_err(ExitReason::MemoryFault)?;
    let offset = args.get(2, memory).map_err(ExitReason::MemoryFault)? as usize;
    let format = args.get(3, memory).map_err(ExitReason::MemoryFault)? as u32;
    if IndexFormat::from_dxgi(format).is_none() {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| {
            with_handle(
                resource,
                memory,
                GraphicsError::InvalidResourceHandle,
                |resource_handle| {
                    graphics.cmd_set_index_buffer(list_handle, resource_handle, offset, format)
                },
            )
        },
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Bind the colour target and the clear colour.
fn xenora_set_render_target(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let resource = args.get(1, memory).map_err(ExitReason::MemoryFault)?;
    let clear = args.get(2, memory).map_err(ExitReason::MemoryFault)? as u32;
    let clear_color = [
        (clear & 0xFF) as u8,
        ((clear >> 8) & 0xFF) as u8,
        ((clear >> 16) & 0xFF) as u8,
        ((clear >> 24) & 0xFF) as u8,
    ];
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| {
            if resource == 0 {
                return graphics.cmd_set_render_targets(list_handle, None, clear_color);
            }
            with_handle(
                resource,
                memory,
                GraphicsError::InvalidResourceHandle,
                |resource_handle| {
                    graphics.cmd_set_render_targets(list_handle, Some(resource_handle), clear_color)
                },
            )
        },
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Set the viewport.
fn xenora_set_viewport(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let width = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let height = args.get(2, memory).map_err(ExitReason::MemoryFault)? as u32;
    if !(1..=MAX_VIEWPORT_DIMENSION).contains(&width)
        || !(1..=MAX_VIEWPORT_DIMENSION).contains(&height)
    {
        return Ok(invalid_arg());
    }
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| graphics.cmd_set_viewport(list_handle, width as usize, height as usize),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Record a non-indexed draw.
fn xenora_draw(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let vertex_count = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let instance_count = args.get(2, memory).map_err(ExitReason::MemoryFault)? as u32;
    let start_vertex = args.get(3, memory).map_err(ExitReason::MemoryFault)? as u32;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| {
            graphics.cmd_draw_instanced(list_handle, vertex_count, instance_count, start_vertex, 0)
        },
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Record an indexed draw. `start_index_base` packs the start index in its
/// low 32 bits and a sign-extended 16-bit base vertex in its high 32 bits.
fn xenora_draw_indexed(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let index_count = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
    let instance_count = args.get(2, memory).map_err(ExitReason::MemoryFault)? as u32;
    let start_index_base = args.get(3, memory).map_err(ExitReason::MemoryFault)?;
    let start_index = start_index_base as u32;
    let base_vertex = i32::from(((start_index_base >> 32) as u16) as i16);
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| {
            graphics.cmd_draw_indexed_instanced(
                list_handle,
                index_count,
                instance_count,
                start_index,
                base_vertex,
                0,
            )
        },
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// End recording so the list can be executed.
fn xenora_close_command_list(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| graphics.close_command_list(list_handle),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Execute a closed command list on a queue.
fn xenora_execute_command_lists(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let queue = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let list = args.get(1, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        queue,
        memory,
        GraphicsError::InvalidQueueHandle,
        |queue_handle| {
            with_handle(
                list,
                memory,
                GraphicsError::InvalidCommandListHandle,
                |list_handle| graphics.execute_command_list(queue_handle, list_handle, memory),
            )
        },
    );
    match result {
        Ok(_) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Copy the contents of a resource into guest memory.
fn xenora_readback_resource(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let resource = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let guest_addr = args.get(1, memory).map_err(ExitReason::MemoryFault)?;
    let byte_length = args.get(2, memory).map_err(ExitReason::MemoryFault)? as usize;
    if guest_addr == 0 {
        return Ok(invalid_arg());
    }
    let graphics = lock_graphics(state)?;
    let bytes = match with_handle(
        resource,
        memory,
        GraphicsError::InvalidResourceHandle,
        |resource_handle| {
            let bytes = graphics.resource_bytes(resource_handle)?;
            Ok(bytes.to_vec())
        },
    ) {
        Ok(bytes) => bytes,
        Err(error) => return Ok(hresult(&error)),
    };
    let count = byte_length.min(bytes.len());
    if count == 0 {
        return Ok(invalid_arg());
    }
    if memory.write_bytes(guest_addr, &bytes[..count]).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Write the backend, width, height and draw count of the last frame.
fn xenora_get_last_frame_info(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let out = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    if out == 0 {
        return Ok(invalid_arg());
    }
    let graphics = lock_graphics(state)?;
    let presentation = match graphics.last_presentation.lock() {
        Ok(presentation) => presentation,
        Err(_) => return Ok(HResult::FAIL.guest_return()),
    };
    let backend = match presentation.backend {
        Some(GraphicsBackendId::MetalNative) => BACKEND_METAL_NATIVE,
        Some(GraphicsBackendId::SoftwareReference) | None => BACKEND_SOFTWARE_REFERENCE,
    };
    let width = u32::try_from(presentation.width).unwrap_or(u32::MAX);
    let height = u32::try_from(presentation.height).unwrap_or(u32::MAX);
    let draw_count = u32::try_from(presentation.draw_count).unwrap_or(u32::MAX);
    drop(presentation);
    let mut record = [0u8; 16];
    record[0..4].copy_from_slice(&backend.to_le_bytes());
    record[4..8].copy_from_slice(&width.to_le_bytes());
    record[8..12].copy_from_slice(&height.to_le_bytes());
    record[12..16].copy_from_slice(&draw_count.to_le_bytes());
    if memory.write_bytes(out, &record).is_err() {
        return Ok(invalid_arg());
    }
    Ok(ok())
}

/// Destroy a resource.
fn xenora_destroy_resource(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let resource = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        resource,
        memory,
        GraphicsError::InvalidResourceHandle,
        |resource_handle| graphics.destroy_resource(resource_handle),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Destroy a command list.
fn xenora_destroy_command_list(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let list = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        list,
        memory,
        GraphicsError::InvalidCommandListHandle,
        |list_handle| graphics.destroy_command_list(list_handle),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Destroy a command allocator.
fn xenora_destroy_command_allocator(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let allocator = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        allocator,
        memory,
        GraphicsError::InvalidAllocatorHandle,
        |allocator_handle| graphics.destroy_command_allocator(allocator_handle),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Destroy a command queue.
fn xenora_destroy_command_queue(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let queue = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        queue,
        memory,
        GraphicsError::InvalidQueueHandle,
        |queue_handle| graphics.destroy_command_queue(queue_handle),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

/// Destroy a device.
fn xenora_destroy_device(
    state: &Arc<ServicesState>,
    args: &Win64Args,
    memory: &mut GuestMemory,
) -> Result<u64, ExitReason> {
    let device = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
    let mut graphics = lock_graphics(state)?;
    let result = with_handle(
        device,
        memory,
        GraphicsError::InvalidDeviceHandle,
        |device_handle| graphics.destroy_device(device_handle),
    );
    match result {
        Ok(()) => Ok(ok()),
        Err(error) => Ok(hresult(&error)),
    }
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    macro_rules! export {
        ($name:literal, $service:path) => {{
            let state = Arc::clone(state);
            registry.register(MODULE, $name, move |args, _cpu, memory| {
                $service(&state, args, memory)
            });
            registry.set_status(MODULE, $name, ServiceStatus::Implemented);
        }};
    }

    export!("XenoraCreateDevice", xenora_create_device);
    export!("XenoraCreateCommandQueue", xenora_create_command_queue);
    export!(
        "XenoraCreateCommandAllocator",
        xenora_create_command_allocator
    );
    export!("XenoraCreateCommandList", xenora_create_command_list);
    export!("XenoraCreateBuffer", xenora_create_buffer);
    export!("XenoraCreateRenderTarget", xenora_create_render_target);
    export!("XenoraSetVertexBuffer", xenora_set_vertex_buffer);
    export!("XenoraSetIndexBuffer", xenora_set_index_buffer);
    export!("XenoraSetRenderTarget", xenora_set_render_target);
    export!("XenoraSetViewport", xenora_set_viewport);
    export!("XenoraDraw", xenora_draw);
    export!("XenoraDrawIndexed", xenora_draw_indexed);
    export!("XenoraCloseCommandList", xenora_close_command_list);
    export!("XenoraExecuteCommandLists", xenora_execute_command_lists);
    export!("XenoraReadbackResource", xenora_readback_resource);
    export!("XenoraGetLastFrameInfo", xenora_get_last_frame_info);
    export!("XenoraDestroyResource", xenora_destroy_resource);
    export!("XenoraDestroyCommandList", xenora_destroy_command_list);
    export!(
        "XenoraDestroyCommandAllocator",
        xenora_destroy_command_allocator
    );
    export!("XenoraDestroyCommandQueue", xenora_destroy_command_queue);
    export!("XenoraDestroyDevice", xenora_destroy_device);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    const E_INVALIDARG_HRESULT: u64 = 0x8007_0057;
    const E_ACCESSDENIED_HRESULT: u64 = 0x8007_0005;

    const VERTEX_ADDRESS: u64 = 0x0001_0000;
    const READBACK_ADDRESS: u64 = 0x0002_0000;
    const HANDLE_SLOT: u64 = 0x0003_0000;

    fn state() -> Arc<ServicesState> {
        Arc::new(ServicesState::new(Arc::new(MockClock::new(0))))
    }

    fn memory() -> GuestMemory {
        let mut memory = GuestMemory::new();
        memory
            .map_region(VERTEX_ADDRESS, 0x1000, MemoryPermissions::READ_WRITE)
            .expect("vertex region");
        memory
            .map_region(READBACK_ADDRESS, 0x1000, MemoryPermissions::READ_WRITE)
            .expect("readback region");
        memory
            .map_region(HANDLE_SLOT, 0x1000, MemoryPermissions::READ_WRITE)
            .expect("handle slot region");
        memory
    }

    fn call(
        state: &Arc<ServicesState>,
        rcx: u64,
        rdx: u64,
        r8: u64,
        r9: u64,
        memory: &mut GuestMemory,
    ) -> u64 {
        let args = Win64Args {
            rcx,
            rdx,
            r8,
            r9,
            rsp: 0,
        };
        xenora_create_device(state, &args, memory).expect("service call succeeds")
    }

    /// The three vertex positions of a triangle that covers a 4 by 3 viewport.
    fn covering_vertices() -> Vec<u8> {
        let positions: [[f32; 4]; 3] = [
            [-3.0, 11.0 / 3.0, 0.0, 1.0],
            [-3.0, -7.0, 0.0, 1.0],
            [5.0, 11.0 / 3.0, 0.0, 1.0],
        ];
        let mut bytes = Vec::new();
        for position in positions {
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn invalid_device_handle_is_rejected() {
        let state = state();
        let mut memory = memory();
        let invalid = 0xDEAD_BEEF_u64;
        let result = {
            let args = Win64Args {
                rcx: invalid,
                rdx: 0,
                r8: HANDLE_SLOT,
                r9: 0,
                rsp: 0,
            };
            xenora_create_command_queue(&state, &args, &mut memory).expect("service call succeeds")
        };
        assert_eq!(result, E_INVALIDARG_HRESULT);
    }

    #[test]
    fn zero_out_pointer_is_rejected() {
        let state = state();
        let mut memory = memory();
        let args = Win64Args {
            rcx: 0,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        let result =
            xenora_create_device(&state, &args, &mut memory).expect("service call succeeds");
        assert_eq!(result, E_INVALIDARG_HRESULT);
    }

    #[test]
    fn zero_viewport_width_is_rejected() {
        let state = state();
        let mut memory = memory();
        let args = Win64Args {
            rcx: 1,
            rdx: 0,
            r8: 3,
            r9: 0,
            rsp: 0,
        };
        let result =
            xenora_set_viewport(&state, &args, &mut memory).expect("service call succeeds");
        assert_eq!(result, E_INVALIDARG_HRESULT);
    }

    #[test]
    fn device_outlives_its_queue() {
        let state = state();
        let mut memory = memory();
        let args = Win64Args {
            rcx: HANDLE_SLOT,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_device(&state, &args, &mut memory).expect("service call succeeds"),
            ok()
        );
        let device = memory.read_u64(HANDLE_SLOT).expect("device handle");
        let queue_args = Win64Args {
            rcx: device,
            rdx: 0,
            r8: HANDLE_SLOT + 8,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_command_queue(&state, &queue_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let queue = memory.read_u64(HANDLE_SLOT + 8).expect("queue handle");

        let destroy_device_args = Win64Args {
            rcx: device,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_destroy_device(&state, &destroy_device_args, &mut memory)
                .expect("service call succeeds"),
            E_ACCESSDENIED_HRESULT
        );

        let destroy_queue_args = Win64Args {
            rcx: queue,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_destroy_command_queue(&state, &destroy_queue_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        assert_eq!(
            xenora_destroy_device(&state, &destroy_device_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
    }

    /// Run one guest draw through the software reference backend and return
    /// the service state, the guest memory and the render target handle.
    fn executed_draw() -> (Arc<ServicesState>, GuestMemory, u64) {
        let state = state();
        let mut memory = memory();
        memory
            .write_bytes(VERTEX_ADDRESS, &covering_vertices())
            .expect("vertex data");

        // Device.
        let args = Win64Args {
            rcx: HANDLE_SLOT,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_device(&state, &args, &mut memory).expect("service call succeeds"),
            ok()
        );
        let device = memory.read_u64(HANDLE_SLOT).expect("device handle");

        // Command queue and allocator.
        let queue_args = Win64Args {
            rcx: device,
            rdx: 0,
            r8: HANDLE_SLOT + 8,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_command_queue(&state, &queue_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let queue = memory.read_u64(HANDLE_SLOT + 8).expect("queue handle");
        let allocator_args = Win64Args {
            rcx: device,
            rdx: 0,
            r8: HANDLE_SLOT + 16,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_command_allocator(&state, &allocator_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let allocator = memory.read_u64(HANDLE_SLOT + 16).expect("allocator handle");

        // Command list.
        let list_args = Win64Args {
            rcx: device,
            rdx: allocator,
            r8: 0,
            r9: HANDLE_SLOT + 24,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_command_list(&state, &list_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let list = memory.read_u64(HANDLE_SLOT + 24).expect("list handle");

        // A 4 by 3 render target and a vertex buffer over the guest data.
        let target_args = Win64Args {
            rcx: device,
            rdx: 4,
            r8: 3,
            r9: HANDLE_SLOT + 32,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_render_target(&state, &target_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let target = memory.read_u64(HANDLE_SLOT + 32).expect("target handle");
        let buffer_args = Win64Args {
            rcx: device,
            rdx: VERTEX_ADDRESS,
            r8: 48,
            r9: HANDLE_SLOT + 40,
            rsp: 0,
        };
        assert_eq!(
            xenora_create_buffer(&state, &buffer_args, &mut memory).expect("service call succeeds"),
            ok()
        );
        let buffer = memory.read_u64(HANDLE_SLOT + 40).expect("buffer handle");

        // Record the draw.
        let set_target_args = Win64Args {
            rcx: list,
            rdx: target,
            r8: 0xFF00_0000,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_set_render_target(&state, &set_target_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let viewport_args = Win64Args {
            rcx: list,
            rdx: 4,
            r8: 3,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_set_viewport(&state, &viewport_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let vertex_args = Win64Args {
            rcx: list,
            rdx: 0,
            r8: buffer,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_set_vertex_buffer(&state, &vertex_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let draw_args = Win64Args {
            rcx: list,
            rdx: 3,
            r8: 1,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_draw(&state, &draw_args, &mut memory).expect("service call succeeds"),
            ok()
        );
        let close_args = Win64Args {
            rcx: list,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_close_command_list(&state, &close_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );

        // Execute.
        let execute_args = Win64Args {
            rcx: queue,
            rdx: list,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_execute_command_lists(&state, &execute_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        (state, memory, target)
    }

    #[test]
    fn software_reference_round_trip() {
        let (state, mut memory, target) = executed_draw();
        let readback_args = Win64Args {
            rcx: target,
            rdx: READBACK_ADDRESS,
            r8: 48,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_readback_resource(&state, &readback_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let mut pixels = [0u8; 48];
        memory
            .read_bytes(READBACK_ADDRESS, &mut pixels)
            .expect("readback bytes");
        assert_eq!(pixels.len(), 4 * 3 * 4);
        for pixel in pixels.as_chunks::<4>().0 {
            assert_eq!(*pixel, [255u8, 0, 0, 255]);
        }
    }

    #[test]
    fn last_frame_info_reports_the_executed_draw() {
        let (state, mut memory, _target) = executed_draw();
        let info_args = Win64Args {
            rcx: HANDLE_SLOT + 48,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0,
        };
        assert_eq!(
            xenora_get_last_frame_info(&state, &info_args, &mut memory)
                .expect("service call succeeds"),
            ok()
        );
        let mut info = [0u8; 16];
        memory
            .read_bytes(HANDLE_SLOT + 48, &mut info)
            .expect("frame info");
        assert_eq!(
            u32::from_le_bytes(info[0..4].try_into().expect("backend")),
            0
        );
        assert_eq!(u32::from_le_bytes(info[4..8].try_into().expect("width")), 4);
        assert_eq!(
            u32::from_le_bytes(info[8..12].try_into().expect("height")),
            3
        );
        assert_eq!(
            u32::from_le_bytes(info[12..16].try_into().expect("draw count")),
            1
        );
    }

    #[test]
    fn service_calls_report_their_hresult() {
        let state = state();
        let mut memory = memory();
        // A call that only exercises the argument plumbing still returns a
        // status rather than faulting the guest.
        assert_eq!(call(&state, 0, 0, 0, 0, &mut memory), E_INVALIDARG_HRESULT);
    }
}
