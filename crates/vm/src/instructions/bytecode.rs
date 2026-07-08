use std::{
    fmt,
    mem::{self, MaybeUninit},
    ops::ControlFlow,
};

use gc_arena::{Collect, Gc};
use thiserror::Error;

use crate::{
    debug::Span,
    instructions::instruction::{
        ConstIdx, HeapIdx, InstIdx, Instruction, MagicIdx, ProtoIdx, RegIdx, StackIdx,
    },
};

#[derive(Debug, Error)]
pub enum ByteCodeEncodingError {
    #[error("jump target {0} out of range")]
    InvalidJump(InstIdx),
    #[error("no return or jump as last instruction")]
    BadLastInstruction,
}

/// An encoded list of [`Instruction`]s.
///
/// Stored in a variable-length, optimized bytecode format internally.
#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct ByteCode {
    // Encoded bytecode, each instruction is serialized directly into the byte array and jump
    // offsets are stored in byte offets.
    bytes: Box<[MaybeUninit<u8>]>,
    // Ordered list of each instruction bytecode boundary, including an entry for the very end of
    // bytecode.
    inst_boundaries: Box<[usize]>,
    // Span data for each instruction.
    inst_spans: Box<[Span]>,
}

impl ByteCode {
    /// Encode a list of instructions as bytecode.
    pub fn encode(
        insts: impl IntoIterator<Item = (Instruction, Span)>,
    ) -> Result<Self, ByteCodeEncodingError> {
        fn opcode_for_inst(inst: Instruction) -> OpCode {
            macro_rules! match_inst {
                ($(
                    [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $($field:ident : $field_ty:ty),* $(,)? };
                )*) => {
                    match inst {
                        $(Instruction::$name { .. } => OpCode::$name),*
                    }
                };
            }
            for_each_instruction!(match_inst)
        }

        fn op_param_len(opcode: OpCode) -> usize {
            macro_rules! match_opcode {
                ($(
                    [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $($field:ident : $field_ty:ty),* $(,)? };
                )*) => {
                    match opcode {
                        $(OpCode::$name { .. } => mem::size_of::<params::$name>()),*
                    }
                };
            }
            for_each_instruction!(match_opcode)
        }

        let insts_iter = insts.into_iter();
        let size_hint = insts_iter.size_hint().0;

        let mut inst_spans = Vec::with_capacity(size_hint);
        let mut insts = Vec::with_capacity(size_hint);

        for (inst, span) in insts_iter {
            insts.push(inst);
            inst_spans.push(span);
        }

        if !matches!(
            insts.last(),
            Some(Instruction::Jump { .. } | Instruction::Return { .. } | Instruction::Throw { .. })
        ) {
            return Err(ByteCodeEncodingError::BadLastInstruction);
        }

        let mut inst_positions = Vec::with_capacity(insts.len());
        let mut pos = 0;
        for &inst in &insts {
            inst_positions.push(pos);
            pos += 1 + op_param_len(opcode_for_inst(inst));
        }
        inst_positions.push(pos);

        let mut bytes = Vec::new();
        for (i, mut inst) in insts.iter().copied().enumerate() {
            assert_eq!(inst_positions[i], bytes.len());

            // Rewrite jump instruction targets to be in bytes

            macro_rules! fixup_targets {
                (
                    $([basic] $(#[$_basic_attr:meta])* $basic_snake_name:ident = $basic_name:ident { $($basic_field:ident : $basic_field_ty:ty),* $(,)? };)*
                    $([$(jump)? $(jump_if)?] $(#[$_jump_attr:meta])* $jump_snake_name:ident = $jump_name:ident { target: InstIdx $(, $jump_field:ident : $jump_field_ty:ty)* $(,)? };)*
                    $([control] $(#[$_control_attr:meta])* $control_snake_name:ident = $control_name:ident { $($control_field:ident : $control_field_ty:ty),* $(,)? };)*
                ) => {
                    match &mut inst {
                        $(Instruction::$jump_name { target, .. })|* => {
                            *target = inst_positions[target.0 as usize].try_into().map_err(|_| {
                                ByteCodeEncodingError::InvalidJump(*target)
                            })?;
                        }
                        _ => {}
                    };
                }
            }
            for_each_instruction!(fixup_targets);

            bytecode_write(&mut bytes, opcode_for_inst(inst));

            macro_rules! write_instruction {
                ($(
                    [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $( $field:ident : $field_ty:ty ),* $(,)? };
                )*) => {
                    match inst {
                        $(Instruction::$name { $($field),* } => {
                            bytecode_write(&mut bytes, params::$name { $($field),* });
                        }),*
                    }
                };
            }
            for_each_instruction!(write_instruction);
        }

        Ok(Self {
            bytes: bytes.into_boxed_slice(),
            inst_boundaries: inst_positions.into_boxed_slice(),
            inst_spans: inst_spans.into_boxed_slice(),
        })
    }

    /// Return the count of encoded instructions.
    #[inline]
    pub fn instruction_len(&self) -> usize {
        self.inst_boundaries.len() - 1
    }

    #[inline]
    pub fn instruction(&self, inst_index: usize) -> Instruction {
        assert!(inst_index < self.instruction_len());
        let pc = self.inst_boundaries[inst_index];
        unsafe { self.decode_instruction_at(pc) }
    }

    #[inline]
    pub fn span(&self, inst_index: usize) -> Span {
        self.inst_spans[inst_index]
    }

    /// Return the program counter for the given instruction index. The `inst_index` may be any
    /// *value up to and including* the instruction length (so one past the final instruction).
    #[inline]
    pub fn pc_for_instruction_index(&self, inst_index: usize) -> usize {
        self.inst_boundaries[inst_index]
    }

    /// Find the instruction index for the given program counter value.
    ///
    /// If the given `pc` is not the start of an instruction or the exact end of bytecode, will
    /// return `None`.
    #[inline]
    pub fn instruction_index_for_pc(&self, pc: usize) -> Option<usize> {
        self.inst_boundaries
            .binary_search_by_key(&pc, |&offset| offset)
            .ok()
    }

    pub fn decode(&self) -> impl Iterator<Item = (Instruction, Span)> {
        (0..self.instruction_len()).map(|i| (self.instruction(i), self.span(i)))
    }

    pub fn pretty_print(&self, f: &mut dyn fmt::Write, indent: u8) -> fmt::Result {
        for (i, inst) in self.decode().map(|(i, _)| i).enumerate() {
            write!(f, "{:indent$}", "", indent = indent as usize)?;
            write!(f, "{i}: ")?;
            inst.pretty_print(f)?;
            writeln!(f)?;
        }
        Ok(())
    }

    // # SAFETY
    //
    // Must be called with a `pc` that is the start of a valid instruction.
    unsafe fn decode_instruction_at(&self, pc: usize) -> Instruction {
        unsafe {
            let mut ptr = self.bytes.as_ptr().add(pc);
            let opcode: OpCode = bytecode_read(&mut ptr);

            macro_rules! decode {
                (
                    $([basic] $(#[$_basic_attr:meta])* $basic_snake_name:ident = $basic_name:ident { $($basic_field:ident : $basic_field_ty:ty),* $(,)? };)*
                    $([$(jump)? $(jump_if)?] $(#[$_jump_attr:meta])* $jump_snake_name:ident = $jump_name:ident { target: InstIdx $(, $jump_field:ident : $jump_field_ty:ty)* $(,)? };)*
                    $([control] $(#[$_control_attr:meta])* $control_snake_name:ident = $control_name:ident { $($control_field:ident : $control_field_ty:ty),* $(,)? };)*
                ) => {
                    match opcode {
                        $(OpCode::$basic_name => {
                            let params::$basic_name { $($basic_field),* } = bytecode_read(&mut ptr);
                            Instruction::$basic_name { $($basic_field),* }
                        })*
                        $(OpCode::$jump_name => {
                            let params::$jump_name { mut target $(, $jump_field)* } = bytecode_read(&mut ptr);
                            target = InstIdx(self.instruction_index_for_pc(target.0 as usize).unwrap() as _);
                            Instruction::$jump_name { target $(, $jump_field)* }
                        })*
                        $(OpCode::$control_name => {
                            let params::$control_name { $($control_field),* } = bytecode_read(&mut ptr);
                            Instruction::$control_name { $($control_field),* }
                        })*
                    }
                };
            }

            for_each_instruction!(decode)
        }
    }
}

impl fmt::Debug for ByteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ByteCode[")?;
        self.pretty_print(f, 4)?;
        writeln!(f, "]")?;
        Ok(())
    }
}

/// Read [`ByteCode`] in a optimized way and dispatch to instruction handlers.
///
/// Highly unsafe internally and relies on `ByteCode` being built correctly for soundness. Since
/// it is only possible to build `ByteCode` in-memory by validating a sequence of [`Instruction`]s,
/// this can provide a completely safe interface to highly optimized instruction dispatch.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Dispatcher<'gc> {
    bytecode: Gc<'gc, ByteCode>,
    #[collect(require_static)]
    ptr: *const MaybeUninit<u8>,
}

impl<'gc> Dispatcher<'gc> {
    /// Construct a new `Dispatcher` for the given bytecode, restoring the given `pc` program
    /// counter.
    ///
    /// # Panics
    ///
    /// Panics if given a program counter that does not fall on an instruction start point. `0` is
    /// always a valid program counter for the starting instruction.
    #[inline]
    pub fn new(bytecode: Gc<'gc, ByteCode>, pc: usize) -> Self {
        assert!(
            pc == 0
                || pc == bytecode.bytes.len()
                || bytecode.instruction_index_for_pc(pc).is_some()
        );
        Self {
            bytecode,
            ptr: unsafe { bytecode.bytes.as_ptr().add(pc) },
        }
    }

    #[inline]
    pub fn bytecode(&self) -> Gc<'gc, ByteCode> {
        self.bytecode
    }

    /// Returns true if this `Dispatcher` has reached the end of the bytecode.
    #[inline]
    pub fn at_end(&self) -> bool {
        self.pc() == self.bytecode.bytes.len()
    }

    /// Returns the current program counter.
    ///
    /// The "program counter" is the byte offset of the next instruction to execute in the bytecode.
    ///
    /// `Dispatcher` state can be completely restored by constructing a new `Dispatcher` with the
    /// same [`ByteCode`] and a stored program counter.
    #[inline]
    pub fn pc(&self) -> usize {
        unsafe { self.ptr.offset_from(self.bytecode.bytes.as_ptr()) as usize }
    }

    /// Returns the current instruction index.
    #[inline]
    pub fn instruction_index(&self) -> usize {
        self.bytecode.instruction_index_for_pc(self.pc()).unwrap()
    }

    /// Dispatch instructions to the given [`Dispatch`] impl.
    ///
    /// # Panics
    ///
    /// Panics if `Self::at_end()` returns true.
    #[inline(never)]
    pub fn dispatch_loop<D: Dispatch>(&mut self, dispatch: &mut D) -> Result<D::Break, D::Error> {
        assert!(
            self.pc() != self.bytecode.bytes.len(),
            "dispatcher reached end of bytecode"
        );

        loop {
            let prev_ptr = self.ptr;
            match self.dispatch_one(dispatch) {
                Ok(ControlFlow::Continue(())) => {}
                Ok(ControlFlow::Break(b)) => {
                    return Ok(b);
                }
                Err(err) => {
                    // Reset the PC back to the instruction that caused the error.
                    self.ptr = prev_ptr;
                    return Err(err);
                }
            }
        }
    }

    /// Dispatch up to `count` instructions on the given [`Dispatch`] impl. Returns Some if the
    /// dispatch impl breaks or errors, None if the instruction count limit was reached. If Some is
    /// returned, the second element of the returned tuple is the remaining allocated instructions.
    ///
    /// # Panics
    ///
    /// Panics if `count` is non-zero and `Self::at_end()` returns true.
    #[inline(never)]
    pub fn dispatch_count<D: Dispatch>(
        &mut self,
        dispatch: &mut D,
        mut count: u32,
    ) -> Option<(Result<D::Break, D::Error>, u32)> {
        if count == 0 {
            return None;
        }

        assert!(
            self.pc() != self.bytecode.bytes.len(),
            "dispatcher reached end of bytecode"
        );

        while count > 0 {
            count -= 1;
            let prev_ptr = self.ptr;
            match self.dispatch_one(dispatch) {
                Ok(ControlFlow::Continue(())) => {}
                Ok(ControlFlow::Break(b)) => {
                    return Some((Ok(b), count));
                }
                Err(err) => {
                    // Reset the PC back to the instruction that caused the error.
                    self.ptr = prev_ptr;
                    // We count an instruction that errors as an executed instruction here.
                    return Some((Err(err), count));
                }
            }
        }

        None
    }

    #[inline(always)]
    fn dispatch_one<D: Dispatch>(
        &mut self,
        dispatch: &mut D,
    ) -> Result<ControlFlow<D::Break>, D::Error> {
        unsafe {
            let opcode: OpCode = bytecode_read(&mut self.ptr);

            macro_rules! dispatch {
                (
                    $([basic] $(#[$_basic_attr:meta])* $basic_snake_name:ident = $basic_name:ident { $($basic_field:ident : $basic_field_ty:ty),* $(,)? };)*
                    $([jump] $(#[$_jump_attr:meta])* $jump_snake_name:ident = $jump_name:ident { $($jump_field:ident : $jump_field_ty:ty),* $(,)? };)*
                    $([jump_if] $(#[$_jump_if_attr:meta])* $jump_if_snake_name:ident = $jump_if_name:ident { target: InstIdx $(, $jump_if_field:ident : $jump_if_field_ty:ty)* $(,)? };)*
                    $([control] $(#[$_control_attr:meta])* $control_snake_name:ident = $control_name:ident { $($control_field:ident : $control_field_ty:ty),* $(,)? };)*
                ) => {
                    match opcode {
                        $(
                            OpCode::$basic_name => {
                                let params::$basic_name { $($basic_field),* } = bytecode_read(&mut self.ptr);
                                dispatch.$basic_snake_name($($basic_field),*)?;
                            }
                        )*

                        OpCode::Jump => {
                            let params::Jump { target } = bytecode_read(&mut self.ptr);
                            self.ptr = self.bytecode.bytes.as_ptr().add(target.0 as usize);
                        }

                        $(
                            OpCode::$jump_if_name => {
                                let params::$jump_if_name { target  $(, $jump_if_field)* } = bytecode_read(&mut self.ptr);
                                if dispatch.$jump_if_snake_name($($jump_if_field),*)? {
                                    self.ptr = self.bytecode.bytes.as_ptr().add(target.0 as usize);
                                }
                            }
                        )*

                        $(
                            OpCode::$control_name => {
                                let params::$control_name { $($control_field),* } = bytecode_read(&mut self.ptr);
                                if let ControlFlow::Break(b) = dispatch.$control_snake_name($($control_field),*)? {
                                    return Ok(ControlFlow::Break(b));
                                }
                            }
                        )*
                    }
                };
            }

            for_each_instruction!(dispatch);
        }

        Ok(ControlFlow::Continue(()))
    }
}

macro_rules! define_dispatch {
    (
        $([basic] $(#[$_basic_attr:meta])* $basic_snake_name:ident = $basic_name:ident { $($basic_field:ident : $basic_field_ty:ty),* $(,)? };)*
        $([jump] $(#[$_jump_attr:meta])* $jump_snake_name:ident = $jump_name:ident { target: InstIdx $(, $jump_field:ident : $jump_field_ty:ty)* $(,)? };)*
        $([jump_if] $(#[$_jump_if_attr:meta])* $jump_if_snake_name:ident = $jump_if_name:ident { target: InstIdx $(, $jump_if_field:ident : $jump_if_field_ty:ty)* $(,)? };)*
        $([control] $(#[$_control_attr:meta])* $control_snake_name:ident = $control_name:ident { $($control_field:ident : $control_field_ty:ty),* $(,)? };)*
    ) => {
        pub trait Dispatch {
            type Break;
            type Error;

            $(fn $basic_snake_name(&mut self, $($basic_field: $basic_field_ty),*) -> Result<(), Self::Error>;)*
            $(fn $jump_if_snake_name(&mut self, $($jump_if_field: $jump_if_field_ty),*) -> Result<bool, Self::Error>;)*
            $(fn $control_snake_name(&mut self, $($control_field: $control_field_ty),*) -> Result<ControlFlow<Self::Break>, Self::Error>;)*
        }
    };
}
for_each_instruction!(define_dispatch);

mod params {
    use super::*;

    macro_rules! define_params {
        ($(
            [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $( $field:ident : $field_ty:ty ),* $(,)? };
        )*) => {
            $(
                #[derive(Copy, Clone)]
                #[repr(packed)]
                pub struct $name {
                    $(pub $field: $field_ty),*
                }
            )*
        };
    }
    for_each_instruction!(define_params);
}

macro_rules! define_opcode {
    ($(
        [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $( $field:ident : $field_ty:ty ),* $(,)? };
    )*) => {
        #[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
        #[repr(u8)]
        enum OpCode {
            $($name),*
        }
    };
}
for_each_instruction!(define_opcode);

#[inline]
fn bytecode_write<T: Copy>(buf: &mut Vec<MaybeUninit<u8>>, val: T) {
    const { assert!(mem::align_of::<T>() == 1) }
    unsafe {
        let len = buf.len();
        buf.reserve(mem::size_of::<T>());
        let p = buf.as_mut_ptr().add(len) as *mut T;
        p.write(val);
        buf.set_len(len + mem::size_of::<T>());
    }
}

#[inline]
unsafe fn bytecode_read<T: Copy>(ptr: &mut *const MaybeUninit<u8>) -> T {
    const { assert!(mem::align_of::<T>() == 1) }
    unsafe {
        let p = *ptr as *const T;
        let v = p.read();
        *ptr = ptr.add(mem::size_of::<T>());
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let insts = &[
            Instruction::LoadConstant {
                constant: ConstIdx(1),
                dest: RegIdx(2),
            },
            Instruction::Jump { target: InstIdx(2) },
            Instruction::IsEqual {
                left: RegIdx(3),
                right: RegIdx(4),
                dest: RegIdx(5),
            },
            Instruction::JumpIf {
                target: InstIdx(1),
                arg: RegIdx(6),
                is_true: true,
            },
            Instruction::Copy {
                source: RegIdx(7),
                dest: RegIdx(8),
            },
            Instruction::Return {},
        ];

        let bytecode = ByteCode::encode(insts.iter().map(|&i| (i, Span::null()))).unwrap();
        let decoded = bytecode.decode().map(|(i, _)| i).collect::<Vec<_>>();
        assert_eq!(insts, decoded.as_slice());
    }
}
