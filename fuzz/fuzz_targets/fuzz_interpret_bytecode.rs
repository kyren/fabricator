#![no_main]

use std::iter;

use arbitrary::Arbitrary;
use fabricator_cli::TestingStdlibContext as _;
use fabricator_vm as vm;
use gc_arena::{Collect, Gc};
use libfuzzer_sys::fuzz_target;
use thiserror::Error;

type RegIdx = u8;
type ConstIdx = u16;
type HeapIdx = u16;
type InstIdx = u32;

#[derive(Debug, Copy, Clone, Arbitrary)]
enum MagicIdx {
    Method,
    Pcall,
    ArrayCreateExt,
}

#[derive(Debug, Copy, Clone, Arbitrary)]
enum Instruction {
    Undefined {
        dest: RegIdx,
    },
    Boolean {
        dest: RegIdx,
        value: bool,
    },
    LoadConstant {
        dest: RegIdx,
        constant: ConstIdx,
    },
    GetHeap {
        dest: RegIdx,
        heap: HeapIdx,
    },
    SetHeap {
        heap: HeapIdx,
        source: RegIdx,
    },
    ResetHeap {
        heap: HeapIdx,
    },
    Globals {
        dest: RegIdx,
    },
    PushThis {},
    PopThis {},
    This {
        dest: RegIdx,
    },
    SetThis {
        source: RegIdx,
    },
    Other {
        dest: RegIdx,
    },
    CurrentClosure {
        dest: RegIdx,
    },
    ArgCount {
        dest: RegIdx,
    },
    GetArg {
        dest: RegIdx,
        index: RegIdx,
    },
    GetArgConst {
        dest: RegIdx,
        index: ConstIdx,
    },
    NewObject {
        dest: RegIdx,
    },
    NewArray {
        dest: RegIdx,
    },
    GetField {
        dest: RegIdx,
        object: RegIdx,
        key: RegIdx,
    },
    SetField {
        object: RegIdx,
        key: RegIdx,
        value: RegIdx,
    },
    GetFieldConst {
        dest: RegIdx,
        object: RegIdx,
        key: ConstIdx,
    },
    SetFieldConst {
        object: RegIdx,
        key: ConstIdx,
        value: RegIdx,
    },
    GetIndex {
        dest: RegIdx,
        array: RegIdx,
        index: RegIdx,
    },
    SetIndex {
        array: RegIdx,
        index: RegIdx,
        value: RegIdx,
    },
    GetIndexConst {
        dest: RegIdx,
        array: RegIdx,
        index: ConstIdx,
    },
    SetIndexConst {
        array: RegIdx,
        index: ConstIdx,
        value: RegIdx,
    },
    Copy {
        dest: RegIdx,
        source: RegIdx,
    },
    IsDefined {
        dest: RegIdx,
        arg: RegIdx,
    },
    IsUndefined {
        dest: RegIdx,
        arg: RegIdx,
    },
    Test {
        dest: RegIdx,
        arg: RegIdx,
    },
    Not {
        dest: RegIdx,
        arg: RegIdx,
    },
    Negate {
        dest: RegIdx,
        arg: RegIdx,
    },
    BitNegate {
        dest: RegIdx,
        arg: RegIdx,
    },
    Increment {
        dest: RegIdx,
        arg: RegIdx,
    },
    Decrement {
        dest: RegIdx,
        arg: RegIdx,
    },
    Subtract {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    Multiply {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    Divide {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    Remainder {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    IntDivide {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    IsEqual {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    IsNotEqual {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    IsLess {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    IsLessEqual {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    And {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    Or {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    Xor {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    BitAnd {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    BitOr {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    BitXor {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    BitShiftLeft {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    BitShiftRight {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    NullCoalesce {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    },
    PushStackFrame {},
    PopStackFrame {},
    StackPush {
        source: RegIdx,
    },
    StackPush2 {
        source_a: RegIdx,
        source_b: RegIdx,
    },
    StackPush3 {
        source_a: RegIdx,
        source_b: RegIdx,
        source_c: RegIdx,
    },
    StackPush4 {
        source_a: RegIdx,
        source_b: RegIdx,
        source_c: RegIdx,
        source_d: RegIdx,
    },
    StackGet {
        dest: RegIdx,
        index: RegIdx,
    },
    StackGetConst {
        dest: RegIdx,
        index: ConstIdx,
    },
    GetIndexMulti {
        dest: RegIdx,
        array: RegIdx,
    },
    SetIndexMulti {
        array: RegIdx,
        value: RegIdx,
    },
    GetMagic {
        dest: RegIdx,
        magic: MagicIdx,
    },
    SetMagic {
        magic: MagicIdx,
        source: RegIdx,
    },
    Throw {
        source: RegIdx,
    },
    Jump {
        target: InstIdx,
    },
    JumpIf {
        target: InstIdx,
        arg: RegIdx,
        is_true: bool,
    },
    JumpIfUndefined {
        target: InstIdx,
        arg: RegIdx,
        is_undefined: bool,
    },
    JumpIfEqual {
        target: InstIdx,
        left: RegIdx,
        right: RegIdx,
    },
    JumpIfNotEqual {
        target: InstIdx,
        left: RegIdx,
        right: RegIdx,
    },
    JumpIfLess {
        target: InstIdx,
        left: RegIdx,
        right: RegIdx,
    },
    JumpIfLessEqual {
        target: InstIdx,
        left: RegIdx,
        right: RegIdx,
    },
    Call {
        func: RegIdx,
        this: Option<RegIdx>,
    },
    Return {},
}

#[derive(Debug, Copy, Clone, Arbitrary)]
enum ConstString {
    A,
    B,
    C,
    D,
    E,
}

impl ConstString {
    fn as_str(self) -> &'static str {
        match self {
            ConstString::A => "a",
            ConstString::B => "b",
            ConstString::C => "c",
            ConstString::D => "d",
            ConstString::E => "e",
        }
    }
}

#[derive(Debug, Copy, Clone, Arbitrary)]
enum Constant {
    Undefined,
    Boolean(bool),
    Integer(i8),
    String(ConstString),
}

impl Constant {
    fn to_vm<'gc>(self, ctx: vm::Context<'gc>) -> vm::Constant<'gc> {
        match self {
            Constant::Undefined => vm::Constant::Undefined,
            Constant::Boolean(b) => vm::Constant::Boolean(b),
            Constant::Integer(i) => vm::Constant::Integer(i as i64),
            Constant::String(s) => vm::Constant::String(ctx.intern(s.as_str())),
        }
    }
}

#[derive(Debug, Arbitrary)]
struct Prototype {
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    static_vars: Vec<Constant>,
    owned_vars: u8,
}

impl Prototype {
    fn to_vm<'gc>(
        self,
        ctx: vm::Context<'gc>,
        magic: Gc<'gc, vm::MagicSet<'gc>>,
    ) -> Result<vm::Prototype<'gc>, vm::closure::PrototypeVerificationError> {
        use vm::instructions as inst;

        struct Chunk(vm::SharedStr);

        impl<'gc> vm::debug::ChunkData for Chunk {
            fn name(&self) -> &vm::SharedStr {
                &self.0
            }

            fn line_number(&self, byte_offset: usize) -> vm::LineNumber {
                vm::LineNumber(byte_offset)
            }
        }

        let chunk = vm::Chunk::new_static(&ctx, Chunk("<randomized>".into()));
        let reference = vm::FunctionRef::Chunk;

        let constants = iter::once(Constant::Undefined)
            .chain(self.constants.into_iter())
            .map(|c| c.to_vm(ctx))
            .collect::<Vec<_>>();

        let static_vars = iter::once(Constant::Undefined)
            .chain(self.static_vars.into_iter())
            .map(|c| vm::closure::SharedValue::new(&ctx, c.to_vm(ctx).to_value().into()))
            .collect::<Vec<_>>();

        let mut heap_vars = Vec::new();
        for i in 0..(static_vars.len().min(u16::MAX as usize)) {
            heap_vars.push(vm::closure::HeapVarDescriptor::Static(inst::HeapIdx(
                i as u16,
            )));
        }
        for i in 0..self.owned_vars {
            heap_vars.push(vm::closure::HeapVarDescriptor::Owned(inst::HeapIdx(
                i as u16,
            )));
        }

        let valid_const = |c: ConstIdx| inst::ConstIdx(((c as usize) % constants.len()) as u16);

        let valid_heap = |c: HeapIdx| inst::HeapIdx(((c as usize) % heap_vars.len()) as u16);

        let valid_owned_heap = |c: HeapIdx| {
            inst::HeapIdx((((c as usize) + static_vars.len()) % heap_vars.len()) as u16)
        };

        let valid_inst =
            |i: InstIdx| inst::InstIdx(((i as usize) % self.instructions.len()) as u32);

        let valid_magic = |i: MagicIdx| {
            inst::MagicIdx(
                magic
                    .find(ctx.intern(match i {
                        MagicIdx::Method => "method",
                        MagicIdx::Pcall => "pcall",
                        MagicIdx::ArrayCreateExt => "array_create_ext",
                    }))
                    .unwrap()
                    .try_into()
                    .unwrap(),
            )
        };

        let mut instructions = Vec::new();

        for &instruction in &self.instructions {
            let inst = match instruction {
                Instruction::Undefined { dest } => inst::Instruction::Undefined {
                    dest: inst::RegIdx(dest),
                },
                Instruction::Boolean { dest, value } => inst::Instruction::Boolean {
                    dest: inst::RegIdx(dest),
                    value,
                },
                Instruction::LoadConstant { dest, constant } => inst::Instruction::LoadConstant {
                    dest: inst::RegIdx(dest),
                    constant: valid_const(constant),
                },
                Instruction::GetHeap { dest, heap } => inst::Instruction::GetHeap {
                    dest: inst::RegIdx(dest),
                    heap: valid_heap(heap),
                },
                Instruction::SetHeap { heap, source } => inst::Instruction::SetHeap {
                    heap: valid_heap(heap),
                    source: inst::RegIdx(source),
                },
                Instruction::ResetHeap { heap } => inst::Instruction::ResetHeap {
                    heap: valid_owned_heap(heap),
                },
                Instruction::Globals { dest } => inst::Instruction::Globals {
                    dest: inst::RegIdx(dest),
                },
                Instruction::PushThis {} => inst::Instruction::PushThis {},
                Instruction::PopThis {} => inst::Instruction::PopThis {},
                Instruction::This { dest } => inst::Instruction::This {
                    dest: inst::RegIdx(dest),
                },
                Instruction::SetThis { source } => inst::Instruction::SetThis {
                    source: inst::RegIdx(source),
                },
                Instruction::Other { dest } => inst::Instruction::Other {
                    dest: inst::RegIdx(dest),
                },
                Instruction::CurrentClosure { dest } => inst::Instruction::CurrentClosure {
                    dest: inst::RegIdx(dest),
                },
                Instruction::ArgCount { dest } => inst::Instruction::ArgCount {
                    dest: inst::RegIdx(dest),
                },
                Instruction::GetArg { dest, index } => inst::Instruction::GetArg {
                    dest: inst::RegIdx(dest),
                    index: inst::RegIdx(index),
                },
                Instruction::GetArgConst { dest, index } => inst::Instruction::GetArgConst {
                    dest: inst::RegIdx(dest),
                    index: valid_const(index),
                },
                Instruction::NewObject { dest } => inst::Instruction::NewObject {
                    dest: inst::RegIdx(dest),
                },
                Instruction::NewArray { dest } => inst::Instruction::NewArray {
                    dest: inst::RegIdx(dest),
                },
                Instruction::GetField { dest, object, key } => inst::Instruction::GetField {
                    dest: inst::RegIdx(dest),
                    object: inst::RegIdx(object),
                    key: inst::RegIdx(key),
                },
                Instruction::SetField { object, key, value } => inst::Instruction::SetField {
                    object: inst::RegIdx(object),
                    key: inst::RegIdx(key),
                    value: inst::RegIdx(value),
                },
                Instruction::GetFieldConst { dest, object, key } => {
                    inst::Instruction::GetFieldConst {
                        dest: inst::RegIdx(dest),
                        object: inst::RegIdx(object),
                        key: valid_const(key),
                    }
                }
                Instruction::SetFieldConst { object, key, value } => {
                    inst::Instruction::SetFieldConst {
                        object: inst::RegIdx(object),
                        key: valid_const(key),
                        value: inst::RegIdx(value),
                    }
                }
                Instruction::GetIndex { dest, array, index } => inst::Instruction::GetIndex {
                    dest: inst::RegIdx(dest),
                    array: inst::RegIdx(array),
                    index: inst::RegIdx(index),
                },
                Instruction::SetIndex {
                    array,
                    index,
                    value,
                } => inst::Instruction::SetIndex {
                    array: inst::RegIdx(array),
                    index: inst::RegIdx(index),
                    value: inst::RegIdx(value),
                },
                Instruction::GetIndexConst { dest, array, index } => {
                    inst::Instruction::GetIndexConst {
                        dest: inst::RegIdx(dest),
                        array: inst::RegIdx(array),
                        index: valid_const(index),
                    }
                }
                Instruction::SetIndexConst {
                    array,
                    index,
                    value,
                } => inst::Instruction::SetIndexConst {
                    array: inst::RegIdx(array),
                    index: valid_const(index),
                    value: inst::RegIdx(value),
                },
                Instruction::Copy { dest, source } => inst::Instruction::Copy {
                    dest: inst::RegIdx(dest),
                    source: inst::RegIdx(source),
                },
                Instruction::IsDefined { dest, arg } => inst::Instruction::IsDefined {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::IsUndefined { dest, arg } => inst::Instruction::IsUndefined {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::Test { dest, arg } => inst::Instruction::Test {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::Not { dest, arg } => inst::Instruction::Not {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::Negate { dest, arg } => inst::Instruction::Negate {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::BitNegate { dest, arg } => inst::Instruction::BitNegate {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::Increment { dest, arg } => inst::Instruction::Increment {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::Decrement { dest, arg } => inst::Instruction::Decrement {
                    dest: inst::RegIdx(dest),
                    arg: inst::RegIdx(arg),
                },
                Instruction::Subtract { dest, left, right } => inst::Instruction::Subtract {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::Multiply { dest, left, right } => inst::Instruction::Multiply {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::Divide { dest, left, right } => inst::Instruction::Divide {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::Remainder { dest, left, right } => inst::Instruction::Remainder {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::IntDivide { dest, left, right } => inst::Instruction::IntDivide {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::IsEqual { dest, left, right } => inst::Instruction::IsEqual {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::IsNotEqual { dest, left, right } => inst::Instruction::IsNotEqual {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::IsLess { dest, left, right } => inst::Instruction::IsLess {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::IsLessEqual { dest, left, right } => inst::Instruction::IsLessEqual {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::And { dest, left, right } => inst::Instruction::And {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::Or { dest, left, right } => inst::Instruction::Or {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::Xor { dest, left, right } => inst::Instruction::Xor {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::BitAnd { dest, left, right } => inst::Instruction::BitAnd {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::BitOr { dest, left, right } => inst::Instruction::BitOr {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::BitXor { dest, left, right } => inst::Instruction::BitXor {
                    dest: inst::RegIdx(dest),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::BitShiftLeft { dest, left, right } => {
                    inst::Instruction::BitShiftLeft {
                        dest: inst::RegIdx(dest),
                        left: inst::RegIdx(left),
                        right: inst::RegIdx(right),
                    }
                }
                Instruction::BitShiftRight { dest, left, right } => {
                    inst::Instruction::BitShiftRight {
                        dest: inst::RegIdx(dest),
                        left: inst::RegIdx(left),
                        right: inst::RegIdx(right),
                    }
                }
                Instruction::NullCoalesce { dest, left, right } => {
                    inst::Instruction::NullCoalesce {
                        dest: inst::RegIdx(dest),
                        left: inst::RegIdx(left),
                        right: inst::RegIdx(right),
                    }
                }
                Instruction::PushStackFrame {} => inst::Instruction::PushStackFrame {},
                Instruction::PopStackFrame {} => inst::Instruction::PopStackFrame {},
                Instruction::StackPush { source } => inst::Instruction::StackPush {
                    source: inst::RegIdx(source),
                },
                Instruction::StackPush2 { source_a, source_b } => inst::Instruction::StackPush2 {
                    source_a: inst::RegIdx(source_a),
                    source_b: inst::RegIdx(source_b),
                },
                Instruction::StackPush3 {
                    source_a,
                    source_b,
                    source_c,
                } => inst::Instruction::StackPush3 {
                    source_a: inst::RegIdx(source_a),
                    source_b: inst::RegIdx(source_b),
                    source_c: inst::RegIdx(source_c),
                },
                Instruction::StackPush4 {
                    source_a,
                    source_b,
                    source_c,
                    source_d,
                } => inst::Instruction::StackPush4 {
                    source_a: inst::RegIdx(source_a),
                    source_b: inst::RegIdx(source_b),
                    source_c: inst::RegIdx(source_c),
                    source_d: inst::RegIdx(source_d),
                },
                Instruction::StackGet { dest, index } => inst::Instruction::StackGet {
                    dest: inst::RegIdx(dest),
                    index: inst::RegIdx(index),
                },
                Instruction::StackGetConst { dest, index } => inst::Instruction::StackGetConst {
                    dest: inst::RegIdx(dest),
                    index: valid_const(index),
                },
                Instruction::GetIndexMulti { dest, array } => inst::Instruction::GetIndexMulti {
                    dest: inst::RegIdx(dest),
                    array: inst::RegIdx(array),
                },
                Instruction::SetIndexMulti { array, value } => inst::Instruction::SetIndexMulti {
                    array: inst::RegIdx(array),
                    value: inst::RegIdx(value),
                },
                Instruction::GetMagic { dest, magic } => inst::Instruction::GetMagic {
                    dest: inst::RegIdx(dest),
                    magic: valid_magic(magic),
                },
                Instruction::SetMagic { magic, source } => inst::Instruction::SetMagic {
                    magic: valid_magic(magic),
                    source: inst::RegIdx(source),
                },
                Instruction::Throw { source } => inst::Instruction::Throw {
                    source: inst::RegIdx(source),
                },
                Instruction::Jump { target } => inst::Instruction::Jump {
                    target: valid_inst(target),
                },
                Instruction::JumpIf {
                    target,
                    arg,
                    is_true,
                } => inst::Instruction::JumpIf {
                    target: valid_inst(target),
                    arg: inst::RegIdx(arg),
                    is_true,
                },
                Instruction::JumpIfUndefined {
                    target,
                    arg,
                    is_undefined,
                } => inst::Instruction::JumpIfUndefined {
                    target: valid_inst(target),
                    arg: inst::RegIdx(arg),
                    is_undefined,
                },
                Instruction::JumpIfEqual {
                    target,
                    left,
                    right,
                } => inst::Instruction::JumpIfEqual {
                    target: valid_inst(target),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::JumpIfNotEqual {
                    target,
                    left,
                    right,
                } => inst::Instruction::JumpIfNotEqual {
                    target: valid_inst(target),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::JumpIfLess {
                    target,
                    left,
                    right,
                } => inst::Instruction::JumpIfLess {
                    target: valid_inst(target),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::JumpIfLessEqual {
                    target,
                    left,
                    right,
                } => inst::Instruction::JumpIfLessEqual {
                    target: valid_inst(target),
                    left: inst::RegIdx(left),
                    right: inst::RegIdx(right),
                },
                Instruction::Call { func, this } => inst::Instruction::Call {
                    func: inst::RegIdx(func),
                    this: this.map(inst::RegIdx),
                },
                Instruction::Return {} => inst::Instruction::Return {},
            };

            instructions.push(inst);
        }

        instructions.push(inst::Instruction::PushStackFrame {});
        instructions.push(inst::Instruction::Return {});

        let bytecode = vm::ByteCode::encode(
            instructions
                .into_iter()
                .enumerate()
                .map(|(i, inst)| (inst, vm::Span::new(i, i))),
        )
        .unwrap();

        let proto = vm::Prototype::new(
            &ctx,
            chunk,
            reference,
            magic,
            Gc::new(&ctx, bytecode),
            constants.into_boxed_slice(),
            Vec::new().into_boxed_slice(),
            static_vars.into_boxed_slice(),
            heap_vars.into_boxed_slice(),
        );

        dbg!(&proto);

        proto
    }
}

fuzz_target!(|prototype: Prototype| {
    const FRAME_LIMIT: u32 = 64;
    const INST_LIMIT: u32 = 16384;

    #[derive(Debug, Error)]
    #[error("vm limit reached")]
    struct VmLimitError;

    #[derive(Default, Collect)]
    #[collect(require_static)]
    struct VmLimiter {
        frames_available: u32,
        insts_available: u32,
    }

    impl VmLimiter {
        fn new() -> Self {
            Self {
                frames_available: FRAME_LIMIT,
                insts_available: INST_LIMIT,
            }
        }
    }

    impl<'gc> vm::Hook<'gc> for VmLimiter {
        fn on_call(
            &mut self,
            _ctx: vm::Context<'gc>,
            _backtrace: vm::Backtrace<'gc, '_>,
        ) -> Result<(), vm::RuntimeError> {
            if let Some(dec) = self.frames_available.checked_sub(1) {
                self.frames_available = dec;
                Ok(())
            } else {
                Err(VmLimitError.into())
            }
        }

        fn on_return(&mut self, _ctx: vm::Context<'gc>, _backtrace: vm::Backtrace<'gc, '_>) {
            self.frames_available += 1;
        }

        fn on_step(
            &mut self,
            _ctx: vm::Context<'gc>,
            instruction_count: u32,
        ) -> Result<u32, vm::RuntimeError> {
            self.insts_available = self.insts_available.saturating_sub(instruction_count);
            if self.insts_available == 0 {
                Err(VmLimitError.into())
            } else {
                Ok(self.insts_available)
            }
        }
    }

    let interpreter = vm::Interpreter::new();

    let stdlib = interpreter.enter(|ctx| {
        let mut lib = vm::MagicSet::new();
        lib.merge(&ctx.testing_stdlib());

        for n in ["a", "b", "c", "d", "e"] {
            lib.insert(
                ctx.intern(n),
                vm::magic::MagicConstant::new_ptr(&ctx, vm::Value::Boolean(true)),
            );
        }

        ctx.stash(Gc::new(&ctx, lib))
    });

    let thread = interpreter.enter(|ctx| {
        let thread = vm::Thread::new(&ctx);
        thread.set_hook(&ctx, VmLimiter::new());
        ctx.stash(thread)
    });

    let Ok(closure) = interpreter.enter(
        |ctx| -> Result<_, vm::closure::PrototypeVerificationError> {
            let prototype = prototype.to_vm(ctx, ctx.fetch(&stdlib))?;
            let closure =
                vm::Closure::new(&ctx, Gc::new(&ctx, prototype), vm::Object::new(&ctx).into())
                    .unwrap();
            Ok(ctx.stash(closure))
        },
    ) else {
        return;
    };
    let _ = interpreter.enter(|ctx| ctx.fetch(&thread).run(ctx, ctx.fetch(&closure)));
});
