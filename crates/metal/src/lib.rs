//! Bounded execution of trusted Metal compute kernels.

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
compile_error!("Xenora's Metal backend requires ARM64 macOS");

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
mod compute;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub use compute::{MetalError, NativeCompute, RunProfile};
