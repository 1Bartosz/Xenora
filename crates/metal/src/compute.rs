use std::ffi::c_void;
use std::fmt;
use std::mem::size_of_val;
use std::ptr::{NonNull, copy_nonoverlapping};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBuffer, MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLCompileOptions, MTLComputeCommandEncoder, MTLComputePipelineState,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLGPUFamily, MTLLibrary,
    MTLMathFloatingPointFunctions, MTLMathMode, MTLResourceOptions, MTLShaderValidation, MTLSize,
};

const MAX_THREADS_PER_GROUP: usize = 64;
const MAX_GROUPS: usize = 64;
const MAX_GRID_WORDS: usize = MAX_GROUPS * MAX_THREADS_PER_GROUP;
const MAX_SOURCE_BYTES: usize = 64 * 1024;

/// Failure at a named operation in the native Metal backend.
#[derive(Debug)]
pub struct MetalError {
    pub operation: &'static str,
    pub detail: String,
}

impl MetalError {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for MetalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for MetalError {}

/// A native Metal device and its command queue for the Phase 01 fixture ABI.
pub struct NativeCompute {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl NativeCompute {
    /// Open the default Apple GPU and create a retained-resource command queue.
    ///
    /// # Errors
    ///
    /// Returns an error if discovery, the capability check, or queue creation fails.
    pub fn new() -> Result<Self, MetalError> {
        autoreleasepool(|_| {
            let device = MTLCreateSystemDefaultDevice().ok_or_else(|| {
                MetalError::new("MTLCreateSystemDefaultDevice", "no Metal device")
            })?;
            // Apple family 7 documents nonuniform threadgroups, which this ABI needs.
            if !device.supportsFamily(MTLGPUFamily::Apple7) {
                return Err(MetalError::new(
                    "supportsFamily",
                    "Apple family 7 capabilities are required",
                ));
            }
            let queue = device
                .newCommandQueue()
                .ok_or_else(|| MetalError::new("newCommandQueue", "queue creation failed"))?;
            Ok(Self { device, queue })
        })
    }

    /// Report the selected device's registered name.
    pub fn device_name(&self) -> String {
        autoreleasepool(|_| self.device.name().to_string())
    }

    /// Run trusted MSL with read-only words at buffer 0 and output words at buffer 1.
    ///
    /// The kernel must use one invocation per word and restrict its memory accesses
    /// to these bindings and ranges. Run this inside a child process with a wall
    /// deadline. The backend caps resources and dispatch size but does not verify
    /// the MSL, its control flow, or its termination.
    ///
    /// # Errors
    ///
    /// Returns an error for out-of-policy input sizes, compilation, resource setup,
    /// dispatch or completion failure.
    pub fn run_words(
        &self,
        source: &str,
        entry: &str,
        input: &[u32],
    ) -> Result<Vec<u32>, MetalError> {
        if !(1..=MAX_GRID_WORDS).contains(&input.len()) {
            return Err(MetalError::new(
                "dispatch bounds",
                format!("expected 1..={MAX_GRID_WORDS} words, got {}", input.len()),
            ));
        }
        if source.is_empty() || source.len() > MAX_SOURCE_BYTES || entry.is_empty() {
            return Err(MetalError::new(
                "source bounds",
                "expected a nonempty entry and source of at most 64 KiB",
            ));
        }

        autoreleasepool(|_| self.run(source, entry, input))
    }

    fn run(&self, source: &str, entry: &str, input: &[u32]) -> Result<Vec<u32>, MetalError> {
        let options = MTLCompileOptions::new();
        if !options.respondsToSelector(sel!(setMathMode:))
            || !options.respondsToSelector(sel!(setMathFloatingPointFunctions:))
        {
            return Err(MetalError::new(
                "MTLCompileOptions",
                "this Metal runtime lacks explicit safe and precise math options",
            ));
        }
        options.setMathMode(MTLMathMode::Safe);
        options.setMathFloatingPointFunctions(MTLMathFloatingPointFunctions::Precise);
        let library = self
            .device
            .newLibraryWithSource_options_error(&NSString::from_str(source), Some(&options))
            .map_err(|error| {
                MetalError::new(
                    "newLibraryWithSource",
                    error.localizedDescription().to_string(),
                )
            })?;
        let function = library
            .newFunctionWithName(&NSString::from_str(entry))
            .ok_or_else(|| {
                MetalError::new("newFunctionWithName", format!("missing entry {entry}"))
            })?;
        let pipeline = self
            .device
            .newComputePipelineStateWithFunction_error(&function)
            .map_err(|error| {
                MetalError::new(
                    "newComputePipelineStateWithFunction",
                    error.localizedDescription().to_string(),
                )
            })?;
        verify_shader_validation(&pipeline)?;

        let width = MAX_THREADS_PER_GROUP
            .min(input.len())
            .min(self.device.maxThreadsPerThreadgroup().width)
            .min(pipeline.maxTotalThreadsPerThreadgroup());
        if width == 0 || input.len().div_ceil(width) > MAX_GROUPS {
            return Err(MetalError::new(
                "dispatch bounds",
                "device or pipeline would require too many groups",
            ));
        }

        let byte_len = size_of_val(input);
        if byte_len > self.device.maxBufferLength() {
            return Err(MetalError::new(
                "buffer bounds",
                "fixture exceeds device buffer limit",
            ));
        }
        let input_buffer = self.new_buffer(input)?;
        let mut words = vec![0xA5A5_A5A5; input.len()];
        let output_buffer = self.new_buffer(&words)?;
        let command = self
            .queue
            .commandBuffer()
            .ok_or_else(|| MetalError::new("commandBuffer", "command buffer creation failed"))?;
        let encoder = command.computeCommandEncoder().ok_or_else(|| {
            MetalError::new("computeCommandEncoder", "compute encoder creation failed")
        })?;
        encoder.setComputePipelineState(&pipeline);
        // SAFETY: These live shared buffers belong to the same device and are
        // initialized before encoding. Both offsets are zero. The kernel's
        // documented ABI binds indices 0 and 1 as arrays of u32.
        unsafe {
            encoder.setBuffer_offset_atIndex(Some(&input_buffer), 0, 0);
            encoder.setBuffer_offset_atIndex(Some(&output_buffer), 0, 1);
        }
        encoder.dispatchThreads_threadsPerThreadgroup(size(input.len()), size(width));
        encoder.endEncoding();
        command.commit();
        command.waitUntilCompleted();

        if command.status() != MTLCommandBufferStatus::Completed {
            let error = command.error().map_or_else(
                || format!("unexpected command status {:?}", command.status()),
                |error| error.localizedDescription().to_string(),
            );
            return Err(MetalError::new("waitUntilCompleted", error));
        }
        // SAFETY: Metal copied the initialized words into distinct shared storage.
        // The GPU finished before the CPU touches that storage. Both ranges are
        // valid for byte_len bytes and don't overlap. A byte copy requires no
        // additional alignment and all u32 bit patterns are valid.
        unsafe {
            copy_nonoverlapping(
                output_buffer.contents().cast::<u8>().as_ptr(),
                words.as_mut_ptr().cast::<u8>(),
                byte_len,
            );
        }
        Ok(words)
    }

    fn new_buffer(
        &self,
        words: &[u32],
    ) -> Result<Retained<ProtocolObject<dyn MTLBuffer>>, MetalError> {
        let bytes = NonNull::from(words).cast::<c_void>();
        // SAFETY: Metal reads exactly size_of_val(words) bytes from the live,
        // initialized slice and makes its own copy within this call.
        unsafe {
            self.device.newBufferWithBytes_length_options(
                bytes,
                size_of_val(words),
                MTLResourceOptions::StorageModeShared,
            )
        }
        .ok_or_else(|| MetalError::new("newBufferWithBytes", "shared buffer allocation failed"))
    }
}

fn verify_shader_validation(
    pipeline: &ProtocolObject<dyn MTLComputePipelineState>,
) -> Result<(), MetalError> {
    if std::env::var("MTL_SHADER_VALIDATION").as_deref() != Ok("1") {
        return Ok(());
    }

    if !pipeline.respondsToSelector(sel!(shaderValidation)) {
        return Err(MetalError::new(
            "shaderValidation",
            "the runtime lacks the pipeline state query",
        ));
    }
    // SAFETY: The selector exists and this newly created pipeline is private
    // to the current thread. The non-atomic property is read before encoding.
    if unsafe { pipeline.shaderValidation() } != MTLShaderValidation::Enabled {
        return Err(MetalError::new(
            "shaderValidation",
            "shader validation requested but inactive for the pipeline",
        ));
    }
    Ok(())
}

fn size(width: usize) -> MTLSize {
    MTLSize {
        width,
        height: 1,
        depth: 1,
    }
}
