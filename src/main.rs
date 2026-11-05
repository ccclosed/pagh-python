//! `pythonc` — a native Python interpreter for the pagh-riscv native ABI.
//!
//! This binary is the freestanding entry point. The interpreter lives in the
//! other modules and is also compiled by `hostcheck/` with a `std` platform, so
//! the language core can be exercised on the host before booting a kernel.

#![no_std]
#![no_main]

extern crate alloc;

mod platform;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    platform::write_stdout("pythonc: interpreter core is being built\n");
    0
}