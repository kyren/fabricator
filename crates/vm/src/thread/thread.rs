use gc_arena::{
    Collect, Gc, Lock, Mutation, RefLock,
    collect::{DynCollect, dyn_collect},
};

use crate::{
    callback::Callback,
    closure::{Closure, SharedValue},
    error::{Error, RuntimeError},
    instructions,
    interpreter::Context,
    thread::{
        dispatch,
        error::{BacktraceFrame, CallError, ClosureBacktraceFrame},
        stack::Stack,
        vec_end_slice::VecEndSlice,
    },
    value::{Function, Value},
};

use super::error::VmError;

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Thread<'gc>(Gc<'gc, ThreadInner<'gc>>);

pub type ThreadInner<'gc> = RefLock<ThreadState<'gc>>;

#[derive(Collect)]
#[collect(no_drop)]
pub struct ThreadState<'gc> {
    frames: Vec<Frame<'gc>>,
    registers: RegisterVec<'gc>,
    stack: Vec<Value<'gc>>,
    stack_frame_boundaries: Vec<usize>,
    this: Vec<Value<'gc>>,
    heap: Vec<OwnedHeapVar<'gc>>,
    hook: Option<Box<dyn Hook<'gc>>>,
}

impl<'gc> Thread<'gc> {
    pub fn new(mc: &Mutation<'gc>) -> Thread<'gc> {
        Thread(Gc::new(
            mc,
            RefLock::new(ThreadState {
                frames: Vec::new(),
                registers: RegisterVec::default(),
                stack: Vec::new(),
                stack_frame_boundaries: Vec::new(),
                this: Vec::new(),
                heap: Vec::new(),
                hook: None,
            }),
        ))
    }

    #[inline]
    pub fn from_inner(inner: Gc<'gc, ThreadInner<'gc>>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, ThreadInner<'gc>> {
        self.0
    }

    pub fn set_hook(self, mc: &Mutation<'gc>, hook: impl Hook<'gc> + Collect<'gc> + 'gc) {
        self.0.borrow_mut(mc).hook = Some(Box::new(hook));
    }

    pub fn clear_hook(self, mc: &Mutation<'gc>) {
        self.0.borrow_mut(mc).hook = None;
    }

    /// Run a function on this `Thread` and discard all return values.
    pub fn run(
        self,
        ctx: Context<'gc>,
        function: impl Into<Function<'gc>>,
    ) -> Result<(), CallError> {
        self.exec(ctx, |mut exec| exec.call(ctx, function))
    }

    /// Run a function on this `Thread` with the given value of `self` and discard all return
    /// values.
    pub fn run_with(
        self,
        ctx: Context<'gc>,
        function: impl Into<Function<'gc>>,
        this: impl Into<Value<'gc>>,
    ) -> Result<(), CallError> {
        self.exec(ctx, |mut exec| exec.with_this(this).call(ctx, function))
    }

    /// Create a top-level [`Execution`] context outside of a callback.
    pub fn exec<R>(self, ctx: Context<'gc>, f: impl FnOnce(Execution<'gc, '_>) -> R) -> R {
        self.enter_state(&ctx, |state| {
            let ret = f(Execution {
                thread: state,
                stack_bottom: 0,
                this_bottom: 0,
            });
            // Guard against `Execution` object not being dropped.
            state.this.clear();
            ret
        })
    }

    fn enter_state<R>(self, mc: &Mutation<'gc>, f: impl FnOnce(&mut ThreadState<'gc>) -> R) -> R {
        let mut thread = self.0.try_borrow_mut(mc).expect("thread locked");
        assert!(
            thread.frames.is_empty()
                && thread.registers.len() == 0
                && thread.stack.is_empty()
                && thread.stack_frame_boundaries.is_empty()
                && thread.this.is_empty()
                && thread.heap.is_empty(),
            "cannot enter thread state, thread is poisoned"
        );

        let ret = f(&mut *thread);

        thread.registers.clear();
        thread.stack.clear();

        assert!(thread.frames.is_empty());
        assert!(thread.stack_frame_boundaries.is_empty());
        assert!(thread.this.is_empty());
        assert!(thread.heap.is_empty());

        ret
    }
}

/// An execution context for some `Thread`.
///
/// This type is passed to all callbacks to allow them to manipulate the call stack and call
/// functions code using the calling `Thread`.
pub struct Execution<'gc, 'a> {
    thread: &'a mut ThreadState<'gc>,
    stack_bottom: usize,
    this_bottom: usize,
}

impl<'gc, 'a> Drop for Execution<'gc, 'a> {
    fn drop(&mut self) {
        self.thread.this.truncate(self.this_bottom);
    }
}

impl<'gc, 'a> Execution<'gc, 'a> {
    /// Return a slice of the current call stack containing callback arguments and returns.
    #[inline]
    pub fn stack(&mut self) -> Stack<'gc, '_> {
        Stack::new(&mut self.thread.stack, self.stack_bottom)
    }

    /// Return a new execution context with a stack starting at the new provided bottom value.
    #[track_caller]
    #[inline]
    pub fn with_stack_bottom(&mut self, stack_bottom: usize) -> Execution<'gc, '_> {
        assert!(self.thread.stack.len() >= self.stack_bottom + stack_bottom);
        Execution {
            thread: self.thread,
            stack_bottom: self.stack_bottom + stack_bottom,
            this_bottom: self.this_bottom,
        }
    }

    /// Return the current number of *explicitly set* values on the `self` stack.
    ///
    /// There is always implicitly an unlimited number of `ctx.globals()` present below the last
    /// explicit `self` value.
    ///
    /// You can add `1` to this value to get indexes for all of the explicitly set `self` values as
    /// well as one copy of the implicit `ctx.globals()` at the bottom.
    #[inline]
    pub fn this_depth(&self) -> usize {
        self.thread.this.len()
    }

    /// Return the nth `self` value.
    ///
    /// The 0th `self` value is the topmost one, the 1th `self` value is the current value of
    /// `other`, etc.
    ///
    /// Any value out of range will always return `ctx.globals()`.
    #[inline]
    pub fn this(&self, ctx: Context<'gc>, nth: usize) -> Value<'gc> {
        self.thread
            .this
            .iter()
            .copied()
            .rev()
            .nth(nth)
            .unwrap_or(ctx.globals().into())
    }

    /// Return a new execution context with a new `self` value pushed from the one provided.
    ///
    /// On drop, the `self` stack will be reset to its previous state.
    #[inline]
    pub fn with_this(&mut self, this: impl Into<Value<'gc>>) -> Execution<'gc, '_> {
        let this_bottom = self.thread.this.len();
        self.thread.this.push(this.into());
        Execution {
            thread: self.thread,
            stack_bottom: self.stack_bottom,
            this_bottom,
        }
    }

    /// Return a new, unmodified `Execution` which borrows from this one.
    #[inline]
    pub fn reborrow(&mut self) -> Execution<'gc, '_> {
        Execution {
            thread: self.thread,
            stack_bottom: self.stack_bottom,
            this_bottom: self.this_bottom,
        }
    }

    /// Within a callback, call the given closure using the parent `Thread`.
    ///
    /// Arguments to the closure will be taken from the stack and returns placed back into the
    /// stack.
    #[inline]
    pub fn call_closure(
        &mut self,
        ctx: Context<'gc>,
        closure: Closure<'gc>,
    ) -> Result<(), VmError<'gc>> {
        self.thread.call_closure(ctx, closure, self.stack_bottom)
    }

    #[inline]
    pub fn call_callback(
        &mut self,
        ctx: Context<'gc>,
        callback: Callback<'gc>,
    ) -> Result<(), RuntimeError> {
        self.thread
            .call_callback(ctx, callback, self.stack_bottom, callback.this())
    }

    /// Call a `Function` within a callback.
    ///
    /// Arguments to the function will be taken from the stack and returns placed back into the
    /// stack.
    ///
    /// Closure and callback errors are converted into `CallError` in a smart way appropriate for
    /// calling a function from within a callback on its calling thread. If the provided function is
    /// a callback that errors and the returned `RuntimeError` wraps a `CallError`, then the inner
    /// `CallError` will be returned. If the provided function is a closure which errors and the
    /// returned `VmError` contains a `CallError`, then the inner `CallError` will be returned
    /// with an inner VM backtrace if present, or the outer VM backtrace if not present. In this
    /// way, callbacks that call functions using `Execution::call` will not add extra layers
    /// of `CallError`, only the *innermost* errors and backtraces will be returned, and since
    /// execution took place on the same `Thread`, the backtrace will already show all outer
    /// callbacks.
    #[inline]
    pub fn call(
        &mut self,
        ctx: Context<'gc>,
        function: impl Into<Function<'gc>>,
    ) -> Result<(), CallError> {
        match function.into() {
            Function::Closure(closure) => {
                if let Err(vm_err) = self.call_closure(ctx, closure) {
                    if let Error::Runtime(rte) = &vm_err.error {
                        if let Some(call_err) = rte.downcast_ref::<CallError>() {
                            return Err(match call_err {
                                CallError::Runtime(runtime_error) => CallError::Vm {
                                    error: runtime_error.clone().into(),
                                    backtrace: vm_err
                                        .backtrace
                                        .into_iter()
                                        .map(|f| f.to_extern())
                                        .collect(),
                                },
                                CallError::Vm { .. } => call_err.clone(),
                            });
                        }
                    }

                    Err(CallError::Vm {
                        error: vm_err.error.into_extern(),
                        backtrace: vm_err
                            .backtrace
                            .into_iter()
                            .map(|f| f.to_extern())
                            .collect(),
                    })
                } else {
                    Ok(())
                }
            }
            Function::Callback(callback) => {
                let res = self.call_callback(ctx, callback);
                if let Err(err) = res {
                    if let Some(call_err) = err.downcast_ref::<CallError>() {
                        Err(call_err.clone())
                    } else {
                        Err(CallError::Runtime(err))
                    }
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Returns the current execution frame depth.
    ///
    /// Every function call, both normal script closures and Rust callbacks, increase the frame
    /// depth by 1.
    ///
    /// This will always be at least 1 for the callback currently executing.
    #[inline]
    pub fn frame_depth(&self) -> usize {
        self.thread.frames.len()
    }

    /// Return a descriptor for this frame or an upper frame.
    ///
    /// The index 0 will return *this* frame, which will always be a callback frame.
    ///
    /// Any higher index will return upper frames, starting with the immediate caller and ending
    /// with the top-level executing frame.
    ///
    /// # Panics
    ///
    /// Panics if given an index that is larger than the return value of [`Execution::frame_depth`].
    #[track_caller]
    #[inline]
    pub fn upper_frame(&self, index: usize) -> BacktraceFrame<'gc> {
        assert!(index < self.thread.frames.len());
        self.thread.frames[self.thread.frames.len() - 1 - index].backtrace_frame()
    }
}

/// A backtrace context for some `Thread`, provided to execution hooks.
pub struct Backtrace<'gc, 'a> {
    frames: &'a [Frame<'gc>],
}

impl<'gc, 'a> Backtrace<'gc, 'a> {
    /// Returns the current execution frame depth.
    ///
    /// Every function call, both normal script closures and Rust callbacks, increase the frame
    /// depth by 1.
    ///
    /// This will always be at least 1 for the callback currently executing.
    #[inline]
    pub fn frame_depth(&self) -> usize {
        self.frames.len()
    }

    /// Return a descriptor for this frame or an upper frame.
    ///
    /// The index 0 will return *this* frame, which will always be a callback frame.
    ///
    /// Any higher index will return upper frames, starting with the immediate caller and ending
    /// with the top-level executing frame.
    ///
    /// # Panics
    ///
    /// Panics if given an index that is larger than the return value of [`Execution::frame_depth`].
    #[track_caller]
    #[inline]
    pub fn frame(&self, index: usize) -> BacktraceFrame<'gc> {
        assert!(index < self.frames.len());
        self.frames[self.frames.len() - 1 - index].backtrace_frame()
    }
}

pub trait Hook<'gc>: 'gc + DynCollect<'gc> {
    /// Hook that is called whenever a [`Closure`] or [`Callback`] is called.
    ///
    /// At the time of call, the frame for the callee will be newly pushed onto the frame stack, so
    /// calling `frames.upper_frame(0)` will return the function that has just been called.
    fn on_call(
        &mut self,
        _ctx: Context<'gc>,
        _backtrace: Backtrace<'gc, '_>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    /// Hook that is called whenever a closure or callback returns.
    ///
    /// At the time of call, the frame for the returning function will still be on the frame stack,
    /// so calling `frames.upper_frame(0)` will return the function that has just returned.
    ///
    /// This function will be called unconditionally whenever a frame is popped, *even* when the
    /// returning script frame is unwinding due to a script error.
    ///
    /// This thread hook *cannot* generate synthetic runtime errors because it is too confusing: if
    /// it were allowed to generate an error and did so, the same hook still must be called after
    /// this repeatedly for every upper unwinding frame.
    fn on_return(&mut self, _ctx: Context<'gc>, _backtrace: Backtrace<'gc, '_>) {}

    /// Hook that is called periodically during execution of VM instructions.
    ///
    /// Whenever a `Thread` starts running the VM, this hook will be called. Its return value
    /// determines how many more VM instructions can run before the hook is called again. A value of
    /// `0` indicates that the hook should be disabled for this VM run.
    ///
    /// The `instruction_count` parameter indicates how many instructions were executed
    /// since the last call to the hook. When the `Thread` first begins running the VM, the
    /// `instruction_count` will always be `0`. If the this first call does not return `0`,
    /// then the hook will always be called at least *once* more before the VM finishes. Thus,
    /// `instruction_count` may be less than the requested amount returned by the last call to the
    /// hook. The hook will never be called with an `instruction_count` greater than the pause value
    /// returned by the last call to the hook.
    ///
    /// A "VM run" here means running the VM for a closure up until the next function call or
    /// return. For every one of these runs, this hook (assuming it is not disabled) will be called
    /// exactly enough to keep track of all the VM instructions that were executed that run.
    fn on_step(
        &mut self,
        _ctx: Context<'gc>,
        _instruction_count: u32,
    ) -> Result<u32, RuntimeError> {
        Ok(0)
    }
}

dyn_collect!(dyn Hook<'gc>);

#[derive(Debug, Collect)]
#[collect(no_drop)]
pub(super) enum OwnedHeapVar<'gc> {
    // We lie here, if a "heap" variable is only uniquely referenced by the closure that owns it, we
    // don't bother to actually allocate it on the heap.
    //
    // Once a closure is created that must share this value, it will be moved to the heap as a
    // `OwnedHeapVar::Shared` value so that it can be shared across closures.
    Unique(Value<'gc>),
    Shared(SharedValue<'gc>),
}

impl<'gc> OwnedHeapVar<'gc> {
    #[inline]
    pub(super) fn unique(value: Value<'gc>) -> Self {
        Self::Unique(value)
    }

    #[inline]
    pub(super) fn get(&self) -> Value<'gc> {
        match self {
            OwnedHeapVar::Unique(v) => *v,
            OwnedHeapVar::Shared(v) => v.get(),
        }
    }

    #[inline]
    pub(super) fn set(&mut self, mc: &Mutation<'gc>, value: Value<'gc>) {
        match self {
            OwnedHeapVar::Unique(v) => *v = value,
            OwnedHeapVar::Shared(v) => v.set(mc, value),
        }
    }

    #[inline]
    pub(super) fn make_shared(&mut self, mc: &Mutation<'gc>) -> SharedValue<'gc> {
        match *self {
            OwnedHeapVar::Unique(v) => {
                let gc = Gc::new(mc, Lock::new(v));
                *self = OwnedHeapVar::Shared(gc);
                gc
            }
            OwnedHeapVar::Shared(v) => v,
        }
    }
}

#[derive(Collect)]
#[collect(no_drop)]
struct ClosureFrame<'gc> {
    closure: Closure<'gc>,
    register_bottom: usize,
    stack_bottom: usize,
    stack_frame_boundaries_bottom: usize,
    this_bottom: usize,
    heap_bottom: usize,
    dispatcher: instructions::Dispatcher<'gc>,
}

#[derive(Collect)]
#[collect(no_drop)]
enum Frame<'gc> {
    Closure(ClosureFrame<'gc>),
    Callback(Callback<'gc>),
}

impl<'gc> Frame<'gc> {
    fn backtrace_frame(&self) -> BacktraceFrame<'gc> {
        match self {
            Frame::Closure(script_frame) => BacktraceFrame::Closure(ClosureBacktraceFrame {
                closure: script_frame.closure,
                instruction: script_frame.dispatcher.instruction_index(),
            }),
            &Frame::Callback(callback) => BacktraceFrame::Callback(callback),
        }
    }
}

impl<'gc> ThreadState<'gc> {
    // Call a closure with arguments starting at `stack_bottom`.
    fn call_closure(
        &mut self,
        ctx: Context<'gc>,
        closure: Closure<'gc>,
        stack_bottom: usize,
    ) -> Result<(), VmError<'gc>> {
        let bottom_frame = self.frames.len();

        self.frames.push({
            let register_bottom = self.registers.len();
            // We only need to preserve the registers that the prototype claims to use.
            self.registers
                .set_len(register_bottom + closure.prototype().used_registers());

            let stack_frame_boundaries_bottom = self.stack_frame_boundaries.len();

            // Push the closure's bound `self` value, if it has one.
            let this_bottom = self.this.len();
            if let Some(this) = closure.this() {
                self.this.push(this)
            }

            let heap_bottom = self.heap.len();
            self.heap
                .resize_with(heap_bottom + closure.prototype().owned_heap(), || {
                    OwnedHeapVar::unique(Value::Undefined)
                });

            Frame::Closure(ClosureFrame {
                closure: closure,
                register_bottom,
                stack_bottom,
                stack_frame_boundaries_bottom,
                this_bottom,
                heap_bottom,
                dispatcher: instructions::Dispatcher::new(closure.prototype().bytecode()),
            })
        });

        fn unwind_closure_frame<'gc>(this: &mut ThreadState<'gc>, ctx: Context<'gc>) {
            if let Some(hook) = &mut this.hook {
                hook.on_return(
                    ctx,
                    Backtrace {
                        frames: &this.frames,
                    },
                );
            }

            match this.frames.pop().unwrap() {
                Frame::Closure(closure_frame) => {
                    this.registers.set_len(closure_frame.register_bottom);
                    this.stack.truncate(closure_frame.stack_bottom);
                    this.stack_frame_boundaries
                        .truncate(closure_frame.stack_frame_boundaries_bottom);
                    this.this.truncate(closure_frame.this_bottom);
                    this.heap.truncate(closure_frame.heap_bottom);
                }
                _ => panic!("not a closure frame"),
            }
        }

        fn vm_error<'gc>(thread: &ThreadState<'gc>, err: impl Into<Error<'gc>>) -> VmError<'gc> {
            VmError {
                error: err.into(),
                backtrace: thread.frames.iter().map(|f| f.backtrace_frame()).collect(),
            }
            .into()
        }

        if let Some(hook) = &mut self.hook {
            if let Err(err) = hook.on_call(
                ctx,
                Backtrace {
                    frames: &self.frames,
                },
            ) {
                let err = vm_error(self, err);
                unwind_closure_frame(self, ctx);
                return Err(err);
            }
        }

        let err = 'step: loop {
            let Frame::Closure(frame) = self.frames.last_mut().unwrap() else {
                unreachable!()
            };

            let registers = self.registers.frame(frame.register_bottom);
            let stack = VecEndSlice::new(&mut self.stack, frame.stack_bottom);
            let stack_frame_boundaries = VecEndSlice::new(
                &mut self.stack_frame_boundaries,
                frame.stack_frame_boundaries_bottom,
            );
            let this = VecEndSlice::new(&mut self.this, frame.this_bottom);
            let heap = &mut self.heap[frame.heap_bottom..];
            let mut dispatch = dispatch::Dispatch::new(
                ctx,
                frame.closure,
                registers,
                stack,
                stack_frame_boundaries,
                this,
                heap,
            );

            let next = if let Some(hook) = &mut self.hook {
                let mut remaining_insts = match hook.on_step(ctx, 0) {
                    Ok(next_remaining) => next_remaining,
                    Err(err) => break 'step err.into(),
                };

                loop {
                    // if `Hook::on_step` returns 0, this indicates that the step hook is disabled
                    // for this VM run.
                    if remaining_insts == 0 {
                        match frame.dispatcher.dispatch_loop(&mut dispatch) {
                            Ok(next) => break next,
                            Err(err) => break 'step err,
                        }
                    }

                    if let Some((mut res, remain)) = frame
                        .dispatcher
                        .dispatch_count(&mut dispatch, remaining_insts)
                    {
                        // The `on_step` hook here takes priority over a script error, because the
                        // contract is that `on_step` should not lose any VM instructions under
                        // any circumstances.
                        //
                        // If the hook succeeds, we throw away the next requested step count because
                        // the VM is pausing.
                        if let Err(err) = hook.on_step(ctx, remaining_insts - remain) {
                            res = Err(err.into());
                        }

                        match res {
                            Ok(next) => break next,
                            Err(err) => break 'step err,
                        }
                    } else {
                        match hook.on_step(ctx, remaining_insts) {
                            Ok(next_remaining) => {
                                remaining_insts = next_remaining;
                            }
                            Err(err) => break 'step err.into(),
                        }
                    }
                }
            } else {
                match frame.dispatcher.dispatch_loop(&mut dispatch) {
                    Ok(next) => next,
                    Err(err) => break err,
                }
            };

            match next {
                dispatch::Next::Call {
                    function,
                    args_bottom,
                    this,
                } => {
                    match function {
                        Function::Closure(closure) => {
                            let register_bottom = self.registers.len();
                            // We only need to preserve the registers that the prototype claims
                            // to use.
                            self.registers
                                .set_len(register_bottom + closure.prototype().used_registers());

                            let stack_bottom = frame.stack_bottom + args_bottom;

                            let stack_frame_boundaries_bottom = self.stack_frame_boundaries.len();

                            // Push the closure's bound `self` value or the provided `self` if
                            // either is defined.
                            let this_bottom = self.this.len();
                            if let Some(this) = closure.this().or(this) {
                                self.this.push(this)
                            }

                            let heap_bottom = self.heap.len();
                            self.heap.resize_with(
                                heap_bottom + closure.prototype().owned_heap(),
                                || OwnedHeapVar::unique(Value::Undefined),
                            );

                            self.frames.push(Frame::Closure(ClosureFrame {
                                closure,
                                register_bottom,
                                stack_bottom,
                                stack_frame_boundaries_bottom,
                                this_bottom,
                                heap_bottom,
                                dispatcher: instructions::Dispatcher::new(
                                    closure.prototype().bytecode(),
                                ),
                            }));

                            if let Some(hook) = &mut self.hook {
                                if let Err(err) = hook.on_call(
                                    ctx,
                                    Backtrace {
                                        frames: &self.frames,
                                    },
                                ) {
                                    break err.into();
                                }
                            }
                        }
                        Function::Callback(callback) => {
                            let stack_bottom = frame.stack_bottom + args_bottom;
                            let this = callback.this().or(this);

                            if let Err(err) = self.call_callback(ctx, callback, stack_bottom, this)
                            {
                                break err.into();
                            }
                        }
                    }
                }
                dispatch::Next::Return { returns_bottom } => {
                    // Truncate the register vec on return.
                    self.registers.set_len(frame.register_bottom);

                    // Drain everything on the stack up until the returns.
                    self.stack
                        .drain(frame.stack_bottom..frame.stack_bottom + returns_bottom);

                    // Clear any unpopped stack frames.
                    self.stack_frame_boundaries
                        .truncate(frame.stack_frame_boundaries_bottom);

                    // Clear any unpopped `self` values.
                    self.this.truncate(frame.this_bottom);

                    // Clear the heap values for this frame.
                    self.heap.truncate(frame.heap_bottom);

                    if let Some(hook) = &mut self.hook {
                        hook.on_return(
                            ctx,
                            Backtrace {
                                frames: &self.frames,
                            },
                        );
                    }

                    // Pop the returning frame.
                    self.frames.pop();

                    // If we have returned from our initial frame, then we can stop executing.
                    if self.frames.len() == bottom_frame {
                        return Ok(());
                    }
                }
            }
        };

        let err = vm_error(self, err);

        while self.frames.len() > bottom_frame {
            unwind_closure_frame(self, ctx);
        }

        Err(err)
    }

    fn call_callback(
        &mut self,
        ctx: Context<'gc>,
        callback: Callback<'gc>,
        stack_bottom: usize,
        with_this: Option<Value<'gc>>,
    ) -> Result<(), RuntimeError> {
        let this_bottom = self.this.len();
        self.frames.push(Frame::Callback(callback));

        if let Some(hook) = &mut self.hook {
            if let Err(err) = hook.on_call(
                ctx,
                Backtrace {
                    frames: &self.frames,
                },
            ) {
                hook.on_return(
                    ctx,
                    Backtrace {
                        frames: &self.frames,
                    },
                );
                // Pop the callback frame.
                assert!(matches!(self.frames.pop(), Some(Frame::Callback(_))));
                return Err(err);
            }
        }

        let mut exec = Execution {
            thread: self,
            stack_bottom,
            this_bottom,
        };
        let ret = callback.function().call(
            ctx,
            if let Some(this) = with_this {
                exec.with_this(this)
            } else {
                exec.reborrow()
            },
        );
        drop(exec);

        if let Some(hook) = &mut self.hook {
            hook.on_return(
                ctx,
                Backtrace {
                    frames: &self.frames,
                },
            );
        }

        // Guard against the `Execution` not being dropped.
        self.this.truncate(this_bottom);

        // Pop the callback frame.
        assert!(matches!(self.frames.pop(), Some(Frame::Callback(_))));

        ret
    }
}

/// The register vector.
///
/// For speed, we want the slice of registers that the VM operates on to be exactly 256 wide to
/// avoid bounds checks, and we want to resize the register vector as little as absolutely possible
/// between calls and returns to avoid having to memset the entire register frame. We use this type
/// to accomplish this.
#[derive(Default, Collect)]
#[collect(no_drop)]
struct RegisterVec<'gc> {
    registers: Vec<Value<'gc>>,
    length: usize,
}

impl<'gc> RegisterVec<'gc> {
    /// Get the current logical length of the register vector.
    #[inline]
    fn len(&self) -> usize {
        self.length
    }

    /// Set the current logical length of the register vector.
    #[inline]
    fn set_len(&mut self, length: usize) {
        self.length = length;
        // Truncate to the extent of the furthest possible valid frame.
        self.registers.truncate(length + 256);
    }

    /// Request a 256 wide slice of registers somewhere in the valid range of the register vector.
    ///
    /// The `bottom` value must be less than or equal to the logical length, but the returned slice
    /// may extend beyond this in order to be exactly 256 wide.
    #[track_caller]
    #[inline]
    fn frame(&mut self, bottom: usize) -> &mut [Value<'gc>; 256] {
        assert!(bottom <= self.length);
        if self.registers.len() < bottom + 256 {
            self.registers.resize(bottom + 256, Value::Undefined);
        }
        (&mut self.registers[bottom..bottom + 256])
            .try_into()
            .unwrap()
    }

    /// Clear the vector and set the logical length to 0.
    #[inline]
    fn clear(&mut self) {
        self.registers.clear();
        self.length = 0;
    }
}
