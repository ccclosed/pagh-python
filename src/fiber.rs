//! Stackful coroutines: the mechanism behind generator functions.
//!
//! The interpreter is a recursive tree walker, so a `yield` can happen at any
//! depth of Rust recursion (inside a loop, a `try`, a nested call). Suspending
//! there without unwinding means switching to a different stack and coming back
//! later, which is exactly a coroutine: this module owns the stack, the
//! register save area and the switch.
//!
//! # Why a stack switch and not a state machine
//!
//! A resumable state machine would have to encode "where am I" for every
//! statement kind — and a `yield` inside `for`/`while`/`try`/`finally`/`with`
//! would need a case for each. A stack switch keeps the whole interpreter
//! state where it already is (on the stack of the suspended fiber) and needs
//! one primitive instead of a second evaluator.
//!
//! # Contract
//!
//! * [`Fiber::switch`] saves the callee-saved registers and the stack pointer
//!   of the current context into `*save` and resumes `*load`. It is the only
//!   piece of assembly in the interpreter.
//! * A fiber's first entry lands in [`pagh_fiber_entry`] (assembly), which
//!   calls [`pagh_fiber_start`], which calls the registered entry function
//!   with the registered argument. The registration happens in
//!   [`Fiber::start`] immediately before the switch, so the entry sees it; the
//!   entry function must copy the argument out during its first instructions
//!   (the pointed-to storage belongs to the resumer's frame and dies when the
//!   fiber first yields).
//! * Stacks come from the interpreter's allocator and are recycled through a
//!   process-wide pool, because pagh never returns memory to the kernel: a
//!   script that creates generators in a loop must not leak a stack per
//!   generator.
//! * [`Fiber::headroom`] reports how much of the current fiber stack is left.
//!   The evaluator checks it before every Python call and raises
//!   `RecursionError` instead of running off the small stack — a debug
//!   assertion is not available here, and silent corruption is not acceptable.
//!
//! There are no threads in this ABI, so the pool and the pending-start slot are
//! plain `static mut` state, touched only with interrupts as they are.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr;

/// Bytes of stack per fiber: the same 64 KiB the kernel gives the process.
///
/// The interpreter's Rust frames are large (a Python call costs a few KiB of
/// Rust stack), so a smaller fiber would cut the usable recursion depth well
/// below what a plain function gets. [`Fiber::headroom`] is the second line of
/// defence for the cases this size still cannot cover.
pub const STACK_SIZE: usize = 64 * 1024;

/// Free bytes that must remain on a fiber stack before another Python call is
/// allowed to start.
pub const STACK_MARGIN: usize = 8 * 1024;

/// The process stack the kernel gives a U-mode task (see the ABI).
///
/// Only the interpreter's README quotes it; the code does not check it.
///
/// The interpreter does *not* check this one: its Rust frames cost a few KiB
/// per Python call, so roughly twelve nested calls fill it, and a check that
/// leaves room for the deepest single call would refuse recursion depths that
/// today still work (`tests/live/functions.py`'s `fib(10)` runs at the very
/// edge of the 64 KiB). The fiber guard below covers the stack this module
/// owns instead; the process stack's limit is documented in the README as a
/// known, pre-existing gap.
#[allow(dead_code)] // documentation constant: the README quotes it, no code reads it
pub const PROCESS_STACK: usize = 64 * 1024;

/// How many fibers may be alive at once. Suspended generators keep their stack
/// for as long as they live, so this is the worst-case footprint of the
/// feature: 8 x 64 KiB = 512 KiB of the 1 MiB heap. The cap is what turns
/// "too many live generators" into a Python `MemoryError` instead of an
/// allocator abort.
pub const MAX_LIVE: usize = 8;

/// The entry point a fiber runs. It must never return.
pub type FiberEntry = unsafe extern "C" fn(*mut u8) -> !;

// ── the switch ─────────────────────────────────────────────────────────────
//
// Both implementations are the same shape: push the callee-saved registers,
// store the stack pointer into `*save`, load the stack pointer from `*load`,
// pop the registers and return into the resumed context.

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(
    r#"
    .text
    .globl pagh_fiber_switch
    .p2align 2
pagh_fiber_switch:
    addi sp, sp, -208
    sd   ra,   0(sp)
    sd   s0,   8(sp)
    sd   s1,  16(sp)
    sd   s2,  24(sp)
    sd   s3,  32(sp)
    sd   s4,  40(sp)
    sd   s5,  48(sp)
    sd   s6,  56(sp)
    sd   s7,  64(sp)
    sd   s8,  72(sp)
    sd   s9,  80(sp)
    sd   s10, 88(sp)
    sd   s11, 96(sp)
    fsd  fs0, 104(sp)
    fsd  fs1, 112(sp)
    fsd  fs2, 120(sp)
    fsd  fs3, 128(sp)
    fsd  fs4, 136(sp)
    fsd  fs5, 144(sp)
    fsd  fs6, 152(sp)
    fsd  fs7, 160(sp)
    fsd  fs8, 168(sp)
    fsd  fs9, 176(sp)
    fsd  fs10,184(sp)
    fsd  fs11,192(sp)
    sd   sp, 0(a0)
    ld   sp, 0(a1)
    ld   ra,   0(sp)
    ld   s0,   8(sp)
    ld   s1,  16(sp)
    ld   s2,  24(sp)
    ld   s3,  32(sp)
    ld   s4,  40(sp)
    ld   s5,  48(sp)
    ld   s6,  56(sp)
    ld   s7,  64(sp)
    ld   s8,  72(sp)
    ld   s9,  80(sp)
    ld   s10, 88(sp)
    ld   s11, 96(sp)
    fld  fs0, 104(sp)
    fld  fs1, 112(sp)
    fld  fs2, 120(sp)
    fld  fs3, 128(sp)
    fld  fs4, 136(sp)
    fld  fs5, 144(sp)
    fld  fs6, 152(sp)
    fld  fs7, 160(sp)
    fld  fs8, 168(sp)
    fld  fs9, 176(sp)
    fld  fs10,184(sp)
    fld  fs11,192(sp)
    addi sp, sp, 208
    ret

    .globl pagh_fiber_entry
    .p2align 2
pagh_fiber_entry:
    call pagh_fiber_start
1:  j 1b
"#
);

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
    .text
    .globl pagh_fiber_switch
    .p2align 4
pagh_fiber_switch:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rdi], rsp
    mov rsp, [rsi]
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

    .globl pagh_fiber_entry
    .p2align 4
pagh_fiber_entry:
    call pagh_fiber_start
1:  jmp 1b
"#
);

extern "C" {
    /// Save the current context in `*save`, resume `*load`.
    fn pagh_fiber_switch(save: *mut usize, load: *const usize);
    /// The assembly trampoline a fresh fiber starts in.
    fn pagh_fiber_entry();
}

/// Hand-off slot between [`Fiber::start`] and [`pagh_fiber_start`].
///
/// The switch is immediate and synchronous, and this ABI has one thread, so a
/// single slot is enough; the volatile accesses keep the store from being
/// reordered past the switch.
static mut PENDING_ENTRY: FiberEntry = unused_entry;
static mut PENDING_ARGUMENT: *mut u8 = ptr::null_mut();

/// Placeholder so the slot has a valid value before the first `start`.
unsafe extern "C" fn unused_entry(_argument: *mut u8) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Called by the assembly trampoline with the fiber stack already active.
#[no_mangle]
pub unsafe extern "C" fn pagh_fiber_start() -> ! {
    let entry = ptr::read_volatile(ptr::addr_of!(PENDING_ENTRY));
    let argument = ptr::read_volatile(ptr::addr_of!(PENDING_ARGUMENT));
    entry(argument)
}

/// Saved stacks, reused by later fibers.
static mut STACK_POOL: Vec<Vec<u8>> = Vec::new();
/// How many fibers exist (suspended or running) right now.
static mut LIVE: usize = 0;

fn live_count() -> usize {
    // SAFETY: single-threaded ABI; the counter is a plain word.
    unsafe { ptr::read_volatile(ptr::addr_of!(LIVE)) }
}

fn set_live(count: usize) {
    // SAFETY: as `live_count`.
    unsafe { ptr::write_volatile(ptr::addr_of_mut!(LIVE), count) }
}

/// A pooled stack if one is free, otherwise a fresh allocation. `None` means
/// "no fiber capacity left" and never an allocation failure: the caller checks
/// [`MAX_LIVE`] first, so the allocator never sees an unbounded request.
fn take_stack(from_pool: bool) -> Option<Vec<u8>> {
    // SAFETY: single-threaded ABI; the pool is only touched from here.
    unsafe {
        if from_pool {
            if let Some(stack) = (*ptr::addr_of_mut!(STACK_POOL)).pop() {
                return Some(stack);
            }
        }
        Some(alloc::vec![0u8; STACK_SIZE])
    }
}

fn give_stack(stack: Vec<u8>) {
    // SAFETY: as `take_stack`; nothing else can be running on this stack.
    unsafe {
        (*ptr::addr_of_mut!(STACK_POOL)).push(stack);
    }
}

/// How many fibers are alive right now.
///
/// The interpreter itself never needs the number (the cap is enforced in
/// [`Fiber::new`]); the host harness prints it when a check fails, because a
/// leaked fiber slot is otherwise invisible.
#[allow(dead_code)]
pub fn live_fibers() -> usize {
    live_count()
}

/// A stack with a prepared initial frame for `entry`.
pub struct Fiber {
    /// The suspended stack pointer; the context handle the switch uses.
    ctx: usize,
    /// Where [`Fiber::yield_to_caller`] writes the resumer's stack pointer.
    caller: *mut usize,
    /// Stack base, used by [`Fiber::headroom`].
    bottom: usize,
    stack: Option<Vec<u8>>,
    started: bool,
    /// Set by the fiber body when it is done; a finished fiber is never
    /// resumed again.
    pub finished: bool,
}

impl Fiber {
    /// Allocate a fiber, or `None` when [`MAX_LIVE`] fibers already exist.
    pub fn new() -> Option<Box<Fiber>> {
        if live_count() >= MAX_LIVE {
            return None;
        }
        let mut stack = take_stack(true)?;
        let bottom = stack.as_ptr() as usize;
        let ctx = prepare_initial_frame(&mut stack);
        set_live(live_count() + 1);
        Some(Box::new(Fiber {
            ctx,
            caller: ptr::null_mut(),
            bottom,
            stack: Some(stack),
            started: false,
            finished: false,
        }))
    }

    /// Enter the fiber for the first time on `entry(argument)`.
    ///
    /// `argument` must stay readable until the fiber first yields (the entry
    /// function is expected to copy out of it immediately).
    ///
    /// # Safety
    /// The caller must not hold a borrow of any data the fiber body touches,
    /// and must not resume the same fiber twice.
    pub unsafe fn start(&mut self, entry: FiberEntry, argument: *mut u8) -> bool {
        if self.started || self.finished {
            return false;
        }
        ptr::write_volatile(ptr::addr_of_mut!(PENDING_ENTRY), entry);
        ptr::write_volatile(ptr::addr_of_mut!(PENDING_ARGUMENT), argument);
        self.started = true;
        let mut caller_sp = 0usize;
        self.caller = &mut caller_sp as *mut usize;
        pagh_fiber_switch(&mut caller_sp as *mut usize, &self.ctx as *const usize);
        true
    }

    /// Resume a suspended fiber.
    ///
    /// # Safety
    /// Only legal while the fiber is suspended (not finished, not running).
    pub unsafe fn resume(&mut self) {
        let mut caller_sp = 0usize;
        self.caller = &mut caller_sp as *mut usize;
        pagh_fiber_switch(&mut caller_sp as *mut usize, &self.ctx as *const usize);
    }

    /// Suspend the *currently running* fiber, returning to its resumer.
    ///
    /// # Safety
    /// Must be called from inside this fiber's own stack.
    pub unsafe fn yield_to_caller(&mut self) {
        let caller = self.caller;
        if caller.is_null() {
            return;
        }
        pagh_fiber_switch(&mut self.ctx as *mut usize, caller as *const usize);
    }

    /// Whether the fiber ever ran.
    pub fn started(&self) -> bool {
        self.started
    }

    /// Bytes of stack left below the current position. Only meaningful while
    /// this fiber is the running one.
    pub fn headroom(&self) -> usize {
        current_sp().saturating_sub(self.bottom)
    }
}

impl Drop for Fiber {
    fn drop(&mut self) {
        if let Some(stack) = self.stack.take() {
            give_stack(stack);
        }
        set_live(live_count().saturating_sub(1));
    }
}

/// The stack pointer of the running context.
pub fn current_sp() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        let sp: usize;
        // SAFETY: reading a register has no side effects.
        unsafe { core::arch::asm!("mv {}, sp", out(reg) sp, options(nomem, nostack)) };
        return sp;
    }
    #[cfg(target_arch = "x86_64")]
    {
        let sp: usize;
        // SAFETY: reading a register has no side effects.
        unsafe { core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack)) };
        return sp;
    }
    #[allow(unreachable_code)]
    0
}

/// Build the frame a fresh fiber starts in and return its stack pointer.
///
/// The layout mirrors what `pagh_fiber_switch` pushes: the callee-saved
/// registers in the order the switch pops them, with the entry trampoline as
/// the return address. After the switch's `ret`, the fiber runs in
/// `pagh_fiber_entry` with a 16-byte aligned stack pointer, which is the ABI's
/// contract for a function entry.
#[cfg(target_arch = "riscv64")]
fn prepare_initial_frame(stack: &mut [u8]) -> usize {
    // 26 slots: ra, s0..s11, fs0..fs11.
    const FRAME: usize = 208;
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !15;
    let sp = top - FRAME;
    // SAFETY: `sp..top` is inside the allocation and 8-byte aligned.
    unsafe {
        let slots = sp as *mut usize;
        for index in 0..(FRAME / 8) {
            ptr::write(slots.add(index), 0);
        }
        ptr::write(slots, pagh_fiber_entry as *const () as usize); // ra
    }
    sp
}

#[cfg(target_arch = "x86_64")]
fn prepare_initial_frame(stack: &mut [u8]) -> usize {
    // 7 slots: r15, r14, r13, r12, rbx, rbp and the return address.
    const FRAME: usize = 56;
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !15;
    let sp = top - FRAME;
    // SAFETY: `sp..top` is inside the allocation and 8-byte aligned.
    unsafe {
        let slots = sp as *mut usize;
        for index in 0..(FRAME / 8) {
            ptr::write(slots.add(index), 0);
        }
        ptr::write(slots.add(6), pagh_fiber_entry as *const () as usize); // return address
    }
    sp
}

#[cfg(not(any(target_arch = "riscv64", target_arch = "x86_64")))]
fn prepare_initial_frame(_stack: &mut [u8]) -> usize {
    unimplemented!("fibers need an architecture-specific context switch")
}
