use fabricator_vm::{self as vm, instructions::Instruction};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    code_gen::{HeapVarDescriptor, Prototype},
    constant::Constant,
    frontend::PrototypeOutput,
    line_numbers::LineNumbers,
    preprocessing::SourceChunk,
};

impl<S: Serialize> Serialize for PrototypeOutput<S> {
    fn serialize<SR: Serializer>(&self, serializer: SR) -> Result<SR::Ok, SR::Error> {
        SerializePrototypeOutput::from_prototype_output(self).serialize(serializer)
    }
}

impl<'de, S: Deserialize<'de>> Deserialize<'de> for PrototypeOutput<S> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(SerializePrototypeOutput::deserialize(deserializer)?.into_prototype_output())
    }
}

#[derive(Copy, Clone, Serialize, Deserialize)]
struct SerializeSpan {
    start: usize,
    end: usize,
}

impl SerializeSpan {
    fn from_span(span: vm::Span) -> Self {
        Self {
            start: span.start(),
            end: span.end(),
        }
    }

    fn to_span(self) -> vm::Span {
        vm::Span::new(self.start, self.end)
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum SerializeFunctionRef<S> {
    Named(S, SerializeSpan),
    Expression(SerializeSpan),
    Chunk,
}

impl<'a, S> SerializeFunctionRef<&'a S> {
    fn from_function_ref(func_ref: &'a vm::FunctionRef<S>) -> Self {
        match *func_ref {
            vm::FunctionRef::Named(ref name, span) => {
                Self::Named(name, SerializeSpan::from_span(span))
            }
            vm::FunctionRef::Expression(span) => Self::Expression(SerializeSpan::from_span(span)),
            vm::FunctionRef::Chunk => Self::Chunk,
        }
    }
}

impl<S> SerializeFunctionRef<S> {
    fn into_function_ref(self) -> vm::FunctionRef<S> {
        match self {
            SerializeFunctionRef::Named(name, span) => vm::FunctionRef::Named(name, span.to_span()),
            SerializeFunctionRef::Expression(span) => vm::FunctionRef::Expression(span.to_span()),
            SerializeFunctionRef::Chunk => vm::FunctionRef::Chunk,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum SerializeConstant<S> {
    Undefined,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(S),
}

impl<'a, S> SerializeConstant<&'a S> {
    fn from_constant(constant: &'a Constant<S>) -> Self {
        match *constant {
            Constant::Undefined => SerializeConstant::Undefined,
            Constant::Boolean(b) => SerializeConstant::Boolean(b),
            Constant::Integer(i) => SerializeConstant::Integer(i),
            Constant::Float(f) => SerializeConstant::Float(f),
            Constant::String(ref s) => SerializeConstant::String(s),
        }
    }
}

impl<S> SerializeConstant<S> {
    fn into_constant(self) -> Constant<S> {
        match self {
            SerializeConstant::Undefined => Constant::Undefined,
            SerializeConstant::Boolean(b) => Constant::Boolean(b),
            SerializeConstant::Integer(i) => Constant::Integer(i),
            SerializeConstant::Float(f) => Constant::Float(f),
            SerializeConstant::String(s) => Constant::String(s),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum SerializeHeapVarDescriptor<S> {
    Owned(HeapIdx),
    Static(SerializeConstant<S>),
    UpValue(HeapIdx),
}

impl<'a, S> SerializeHeapVarDescriptor<&'a S> {
    fn from_heap_var_descriptor(heap_var_desc: &'a HeapVarDescriptor<S>) -> Self {
        match heap_var_desc {
            HeapVarDescriptor::Owned(heap_idx) => SerializeHeapVarDescriptor::Owned(heap_idx.0),
            HeapVarDescriptor::Static(constant) => {
                SerializeHeapVarDescriptor::Static(SerializeConstant::from_constant(constant))
            }
            HeapVarDescriptor::UpValue(heap_idx) => SerializeHeapVarDescriptor::UpValue(heap_idx.0),
        }
    }
}

impl<S> SerializeHeapVarDescriptor<S> {
    fn into_heap_var_descriptor(self) -> HeapVarDescriptor<S> {
        match self {
            SerializeHeapVarDescriptor::Owned(heap_idx) => {
                HeapVarDescriptor::Owned(heap_idx.into())
            }
            SerializeHeapVarDescriptor::Static(constant) => {
                HeapVarDescriptor::Static(constant.into_constant())
            }
            SerializeHeapVarDescriptor::UpValue(heap_idx) => {
                HeapVarDescriptor::UpValue(heap_idx.into())
            }
        }
    }
}

type RegIdx = u8;
type StackIdx = u8;
type ConstIdx = u16;
type HeapIdx = u16;
type ProtoIdx = u16;
type MagicIdx = u32;
type InstIdx = u32;

#[derive(Copy, Clone, Serialize, Deserialize)]
enum SerializeInstruction {
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
    Closure {
        dest: RegIdx,
        proto: ProtoIdx,
        bind_this: bool,
    },
    CurrentClosure {
        dest: RegIdx,
    },
    ArgCount {
        dest: RegIdx,
    },
    ArgGet {
        dest: RegIdx,
        index: StackIdx,
    },
    ArgGetAt {
        dest: RegIdx,
        index: RegIdx,
    },
    NewObject {
        dest: RegIdx,
    },
    NewArray {
        dest: RegIdx,
    },
    GetField {
        dest: RegIdx,
        target: RegIdx,
        key: RegIdx,
    },
    SetField {
        target: RegIdx,
        key: RegIdx,
        value: RegIdx,
    },
    GetFieldConst {
        dest: RegIdx,
        target: RegIdx,
        key: ConstIdx,
    },
    SetFieldConst {
        target: RegIdx,
        key: ConstIdx,
        value: RegIdx,
    },
    GetIndex {
        dest: RegIdx,
        target: RegIdx,
        index: RegIdx,
    },
    SetIndex {
        target: RegIdx,
        index: RegIdx,
        value: RegIdx,
    },
    GetIndexConst {
        dest: RegIdx,
        target: RegIdx,
        index: ConstIdx,
    },
    SetIndexConst {
        target: RegIdx,
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
    Add {
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
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
    JoinStackFrame {},
    SplitStackFrame {
        base: StackIdx,
    },
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
    StackPushArgs {
        first_index: StackIdx,
    },
    StackGet {
        dest: RegIdx,
        index: StackIdx,
    },
    GetMagic {
        dest: RegIdx,
        magic: MagicIdx,
    },
    SetMagic {
        magic: MagicIdx,
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

impl SerializeInstruction {
    fn from_instruction(inst: Instruction) -> Self {
        match inst {
            Instruction::Undefined { dest } => SerializeInstruction::Undefined { dest: dest.0 },
            Instruction::Boolean { dest, value } => SerializeInstruction::Boolean {
                dest: dest.0,
                value,
            },
            Instruction::LoadConstant { dest, constant } => SerializeInstruction::LoadConstant {
                dest: dest.0,
                constant: constant.0,
            },
            Instruction::GetHeap { dest, heap } => SerializeInstruction::GetHeap {
                dest: dest.0,
                heap: heap.0,
            },
            Instruction::SetHeap { heap, source } => SerializeInstruction::SetHeap {
                heap: heap.0,
                source: source.0,
            },
            Instruction::ResetHeap { heap } => SerializeInstruction::ResetHeap { heap: heap.0 },
            Instruction::Globals { dest } => SerializeInstruction::Globals { dest: dest.0 },
            Instruction::PushThis {} => SerializeInstruction::PushThis {},
            Instruction::PopThis {} => SerializeInstruction::PopThis {},
            Instruction::This { dest } => SerializeInstruction::This { dest: dest.0 },
            Instruction::SetThis { source } => SerializeInstruction::SetThis { source: source.0 },
            Instruction::Other { dest } => SerializeInstruction::Other { dest: dest.0 },
            Instruction::Closure {
                dest,
                proto,
                bind_this,
            } => SerializeInstruction::Closure {
                dest: dest.0,
                proto: proto.0,
                bind_this,
            },
            Instruction::CurrentClosure { dest } => {
                SerializeInstruction::CurrentClosure { dest: dest.0 }
            }
            Instruction::ArgCount { dest } => SerializeInstruction::ArgCount { dest: dest.0 },
            Instruction::ArgGet { dest, index } => SerializeInstruction::ArgGet {
                dest: dest.0,
                index: index.0,
            },
            Instruction::ArgGetAt { dest, index } => SerializeInstruction::ArgGetAt {
                dest: dest.0,
                index: index.0,
            },
            Instruction::NewObject { dest } => SerializeInstruction::NewObject { dest: dest.0 },
            Instruction::NewArray { dest } => SerializeInstruction::NewArray { dest: dest.0 },
            Instruction::GetField { dest, target, key } => SerializeInstruction::GetField {
                dest: dest.0,
                target: target.0,
                key: key.0,
            },
            Instruction::SetField { target, key, value } => SerializeInstruction::SetField {
                target: target.0,
                key: key.0,
                value: value.0,
            },
            Instruction::GetFieldConst { dest, target, key } => {
                SerializeInstruction::GetFieldConst {
                    dest: dest.0,
                    target: target.0,
                    key: key.0,
                }
            }
            Instruction::SetFieldConst { target, key, value } => {
                SerializeInstruction::SetFieldConst {
                    target: target.0,
                    key: key.0,
                    value: value.0,
                }
            }
            Instruction::GetIndex {
                dest,
                target,
                index,
            } => SerializeInstruction::GetIndex {
                dest: dest.0,
                target: target.0,
                index: index.0,
            },
            Instruction::SetIndex {
                target,
                index,
                value,
            } => SerializeInstruction::SetIndex {
                target: target.0,
                index: index.0,
                value: value.0,
            },
            Instruction::GetIndexConst {
                dest,
                target,
                index,
            } => SerializeInstruction::GetIndexConst {
                dest: dest.0,
                target: target.0,
                index: index.0,
            },
            Instruction::SetIndexConst {
                target,
                index,
                value,
            } => SerializeInstruction::SetIndexConst {
                target: target.0,
                index: index.0,
                value: value.0,
            },
            Instruction::Copy { dest, source } => SerializeInstruction::Copy {
                dest: dest.0,
                source: source.0,
            },
            Instruction::IsDefined { dest, arg } => SerializeInstruction::IsDefined {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::IsUndefined { dest, arg } => SerializeInstruction::IsUndefined {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::Test { dest, arg } => SerializeInstruction::Test {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::Not { dest, arg } => SerializeInstruction::Not {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::Negate { dest, arg } => SerializeInstruction::Negate {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::BitNegate { dest, arg } => SerializeInstruction::BitNegate {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::Increment { dest, arg } => SerializeInstruction::Increment {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::Decrement { dest, arg } => SerializeInstruction::Decrement {
                dest: dest.0,
                arg: arg.0,
            },
            Instruction::Add { dest, left, right } => SerializeInstruction::Add {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Subtract { dest, left, right } => SerializeInstruction::Subtract {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Multiply { dest, left, right } => SerializeInstruction::Multiply {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Divide { dest, left, right } => SerializeInstruction::Divide {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Remainder { dest, left, right } => SerializeInstruction::Remainder {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::IntDivide { dest, left, right } => SerializeInstruction::IntDivide {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::IsEqual { dest, left, right } => SerializeInstruction::IsEqual {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::IsNotEqual { dest, left, right } => SerializeInstruction::IsNotEqual {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::IsLess { dest, left, right } => SerializeInstruction::IsLess {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::IsLessEqual { dest, left, right } => SerializeInstruction::IsLessEqual {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::And { dest, left, right } => SerializeInstruction::And {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Or { dest, left, right } => SerializeInstruction::Or {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Xor { dest, left, right } => SerializeInstruction::Xor {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::BitAnd { dest, left, right } => SerializeInstruction::BitAnd {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::BitOr { dest, left, right } => SerializeInstruction::BitOr {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::BitXor { dest, left, right } => SerializeInstruction::BitXor {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::BitShiftLeft { dest, left, right } => SerializeInstruction::BitShiftLeft {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::BitShiftRight { dest, left, right } => {
                SerializeInstruction::BitShiftRight {
                    dest: dest.0,
                    left: left.0,
                    right: right.0,
                }
            }
            Instruction::NullCoalesce { dest, left, right } => SerializeInstruction::NullCoalesce {
                dest: dest.0,
                left: left.0,
                right: right.0,
            },
            Instruction::PushStackFrame {} => SerializeInstruction::PushStackFrame {},
            Instruction::PopStackFrame {} => SerializeInstruction::PopStackFrame {},
            Instruction::JoinStackFrame {} => SerializeInstruction::JoinStackFrame {},
            Instruction::SplitStackFrame { base } => {
                SerializeInstruction::SplitStackFrame { base: base.0 }
            }
            Instruction::StackPush { source } => {
                SerializeInstruction::StackPush { source: source.0 }
            }
            Instruction::StackPush2 { source_a, source_b } => SerializeInstruction::StackPush2 {
                source_a: source_a.0,
                source_b: source_b.0,
            },
            Instruction::StackPush3 {
                source_a,
                source_b,
                source_c,
            } => SerializeInstruction::StackPush3 {
                source_a: source_a.0,
                source_b: source_b.0,
                source_c: source_c.0,
            },
            Instruction::StackPush4 {
                source_a,
                source_b,
                source_c,
                source_d,
            } => SerializeInstruction::StackPush4 {
                source_a: source_a.0,
                source_b: source_b.0,
                source_c: source_c.0,
                source_d: source_d.0,
            },
            Instruction::StackPushArgs { first_index } => SerializeInstruction::StackPushArgs {
                first_index: first_index.0,
            },
            Instruction::StackGet { dest, index } => SerializeInstruction::StackGet {
                dest: dest.0,
                index: index.0,
            },
            Instruction::GetMagic { dest, magic } => SerializeInstruction::GetMagic {
                dest: dest.0,
                magic: magic.0,
            },
            Instruction::SetMagic { magic, source } => SerializeInstruction::SetMagic {
                magic: magic.0,
                source: source.0,
            },
            Instruction::Jump { target } => SerializeInstruction::Jump { target: target.0 },
            Instruction::JumpIf {
                target,
                arg,
                is_true,
            } => SerializeInstruction::JumpIf {
                target: target.0,
                arg: arg.0,
                is_true,
            },
            Instruction::JumpIfUndefined {
                target,
                arg,
                is_undefined,
            } => SerializeInstruction::JumpIfUndefined {
                target: target.0,
                arg: arg.0,
                is_undefined,
            },
            Instruction::JumpIfEqual {
                target,
                left,
                right,
            } => SerializeInstruction::JumpIfEqual {
                target: target.0,
                left: left.0,
                right: right.0,
            },
            Instruction::JumpIfNotEqual {
                target,
                left,
                right,
            } => SerializeInstruction::JumpIfNotEqual {
                target: target.0,
                left: left.0,
                right: right.0,
            },
            Instruction::JumpIfLess {
                target,
                left,
                right,
            } => SerializeInstruction::JumpIfLess {
                target: target.0,
                left: left.0,
                right: right.0,
            },
            Instruction::JumpIfLessEqual {
                target,
                left,
                right,
            } => SerializeInstruction::JumpIfLessEqual {
                target: target.0,
                left: left.0,
                right: right.0,
            },
            Instruction::Call { func, this } => SerializeInstruction::Call {
                func: func.0,
                this: this.map(|t| t.0),
            },
            Instruction::Return {} => SerializeInstruction::Return {},
        }
    }

    fn to_instruction(self) -> Instruction {
        match self {
            SerializeInstruction::Undefined { dest } => {
                Instruction::Undefined { dest: dest.into() }
            }
            SerializeInstruction::Boolean { dest, value } => Instruction::Boolean {
                dest: dest.into(),
                value,
            },
            SerializeInstruction::LoadConstant { dest, constant } => Instruction::LoadConstant {
                dest: dest.into(),
                constant: constant.into(),
            },
            SerializeInstruction::GetHeap { dest, heap } => Instruction::GetHeap {
                dest: dest.into(),
                heap: heap.into(),
            },
            SerializeInstruction::SetHeap { heap, source } => Instruction::SetHeap {
                heap: heap.into(),
                source: source.into(),
            },
            SerializeInstruction::ResetHeap { heap } => {
                Instruction::ResetHeap { heap: heap.into() }
            }
            SerializeInstruction::Globals { dest } => Instruction::Globals { dest: dest.into() },
            SerializeInstruction::PushThis {} => Instruction::PushThis {},
            SerializeInstruction::PopThis {} => Instruction::PopThis {},
            SerializeInstruction::This { dest } => Instruction::This { dest: dest.into() },
            SerializeInstruction::SetThis { source } => Instruction::SetThis {
                source: source.into(),
            },
            SerializeInstruction::Other { dest } => Instruction::Other { dest: dest.into() },
            SerializeInstruction::Closure {
                dest,
                proto,
                bind_this,
            } => Instruction::Closure {
                dest: dest.into(),
                proto: proto.into(),
                bind_this,
            },
            SerializeInstruction::CurrentClosure { dest } => {
                Instruction::CurrentClosure { dest: dest.into() }
            }
            SerializeInstruction::ArgCount { dest } => Instruction::ArgCount { dest: dest.into() },
            SerializeInstruction::ArgGet { dest, index } => Instruction::ArgGet {
                dest: dest.into(),
                index: index.into(),
            },
            SerializeInstruction::ArgGetAt { dest, index } => Instruction::ArgGetAt {
                dest: dest.into(),
                index: index.into(),
            },
            SerializeInstruction::NewObject { dest } => {
                Instruction::NewObject { dest: dest.into() }
            }
            SerializeInstruction::NewArray { dest } => Instruction::NewArray { dest: dest.into() },
            SerializeInstruction::GetField { dest, target, key } => Instruction::GetField {
                dest: dest.into(),
                target: target.into(),
                key: key.into(),
            },
            SerializeInstruction::SetField { target, key, value } => Instruction::SetField {
                target: target.into(),
                key: key.into(),
                value: value.into(),
            },
            SerializeInstruction::GetFieldConst { dest, target, key } => {
                Instruction::GetFieldConst {
                    dest: dest.into(),
                    target: target.into(),
                    key: key.into(),
                }
            }
            SerializeInstruction::SetFieldConst { target, key, value } => {
                Instruction::SetFieldConst {
                    target: target.into(),
                    key: key.into(),
                    value: value.into(),
                }
            }
            SerializeInstruction::GetIndex {
                dest,
                target,
                index,
            } => Instruction::GetIndex {
                dest: dest.into(),
                target: target.into(),
                index: index.into(),
            },
            SerializeInstruction::SetIndex {
                target,
                index,
                value,
            } => Instruction::SetIndex {
                target: target.into(),
                index: index.into(),
                value: value.into(),
            },
            SerializeInstruction::GetIndexConst {
                dest,
                target,
                index,
            } => Instruction::GetIndexConst {
                dest: dest.into(),
                target: target.into(),
                index: index.into(),
            },
            SerializeInstruction::SetIndexConst {
                target,
                index,
                value,
            } => Instruction::SetIndexConst {
                target: target.into(),
                index: index.into(),
                value: value.into(),
            },
            SerializeInstruction::Copy { dest, source } => Instruction::Copy {
                dest: dest.into(),
                source: source.into(),
            },
            SerializeInstruction::IsDefined { dest, arg } => Instruction::IsDefined {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::IsUndefined { dest, arg } => Instruction::IsUndefined {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::Test { dest, arg } => Instruction::Test {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::Not { dest, arg } => Instruction::Not {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::Negate { dest, arg } => Instruction::Negate {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::BitNegate { dest, arg } => Instruction::BitNegate {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::Increment { dest, arg } => Instruction::Increment {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::Decrement { dest, arg } => Instruction::Decrement {
                dest: dest.into(),
                arg: arg.into(),
            },
            SerializeInstruction::Add { dest, left, right } => Instruction::Add {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Subtract { dest, left, right } => Instruction::Subtract {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Multiply { dest, left, right } => Instruction::Multiply {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Divide { dest, left, right } => Instruction::Divide {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Remainder { dest, left, right } => Instruction::Remainder {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::IntDivide { dest, left, right } => Instruction::IntDivide {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::IsEqual { dest, left, right } => Instruction::IsEqual {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::IsNotEqual { dest, left, right } => Instruction::IsNotEqual {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::IsLess { dest, left, right } => Instruction::IsLess {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::IsLessEqual { dest, left, right } => Instruction::IsLessEqual {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::And { dest, left, right } => Instruction::And {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Or { dest, left, right } => Instruction::Or {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Xor { dest, left, right } => Instruction::Xor {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::BitAnd { dest, left, right } => Instruction::BitAnd {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::BitOr { dest, left, right } => Instruction::BitOr {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::BitXor { dest, left, right } => Instruction::BitXor {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::BitShiftLeft { dest, left, right } => Instruction::BitShiftLeft {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::BitShiftRight { dest, left, right } => {
                Instruction::BitShiftRight {
                    dest: dest.into(),
                    left: left.into(),
                    right: right.into(),
                }
            }
            SerializeInstruction::NullCoalesce { dest, left, right } => Instruction::NullCoalesce {
                dest: dest.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::PushStackFrame {} => Instruction::PushStackFrame {},
            SerializeInstruction::PopStackFrame {} => Instruction::PopStackFrame {},
            SerializeInstruction::JoinStackFrame {} => Instruction::JoinStackFrame {},
            SerializeInstruction::SplitStackFrame { base } => {
                Instruction::SplitStackFrame { base: base.into() }
            }
            SerializeInstruction::StackPush { source } => Instruction::StackPush {
                source: source.into(),
            },
            SerializeInstruction::StackPush2 { source_a, source_b } => Instruction::StackPush2 {
                source_a: source_a.into(),
                source_b: source_b.into(),
            },
            SerializeInstruction::StackPush3 {
                source_a,
                source_b,
                source_c,
            } => Instruction::StackPush3 {
                source_a: source_a.into(),
                source_b: source_b.into(),
                source_c: source_c.into(),
            },
            SerializeInstruction::StackPush4 {
                source_a,
                source_b,
                source_c,
                source_d,
            } => Instruction::StackPush4 {
                source_a: source_a.into(),
                source_b: source_b.into(),
                source_c: source_c.into(),
                source_d: source_d.into(),
            },
            SerializeInstruction::StackPushArgs { first_index } => Instruction::StackPushArgs {
                first_index: first_index.into(),
            },
            SerializeInstruction::StackGet { dest, index } => Instruction::StackGet {
                dest: dest.into(),
                index: index.into(),
            },
            SerializeInstruction::GetMagic { dest, magic } => Instruction::GetMagic {
                dest: dest.into(),
                magic: magic.into(),
            },
            SerializeInstruction::SetMagic { magic, source } => Instruction::SetMagic {
                magic: magic.into(),
                source: source.into(),
            },
            SerializeInstruction::Jump { target } => Instruction::Jump {
                target: target.into(),
            },
            SerializeInstruction::JumpIf {
                target,
                arg,
                is_true,
            } => Instruction::JumpIf {
                target: target.into(),
                arg: arg.into(),
                is_true,
            },
            SerializeInstruction::JumpIfUndefined {
                target,
                arg,
                is_undefined,
            } => Instruction::JumpIfUndefined {
                target: target.into(),
                arg: arg.into(),
                is_undefined,
            },
            SerializeInstruction::JumpIfEqual {
                target,
                left,
                right,
            } => Instruction::JumpIfEqual {
                target: target.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::JumpIfNotEqual {
                target,
                left,
                right,
            } => Instruction::JumpIfNotEqual {
                target: target.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::JumpIfLess {
                target,
                left,
                right,
            } => Instruction::JumpIfLess {
                target: target.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::JumpIfLessEqual {
                target,
                left,
                right,
            } => Instruction::JumpIfLessEqual {
                target: target.into(),
                left: left.into(),
                right: right.into(),
            },
            SerializeInstruction::Call { func, this } => Instruction::Call {
                func: func.into(),
                this: this.map(|t| t.into()),
            },
            SerializeInstruction::Return {} => Instruction::Return {},
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct SerializePrototype<S> {
    reference: SerializeFunctionRef<S>,
    instructions: Box<[(SerializeInstruction, SerializeSpan)]>,
    constants: Box<[SerializeConstant<S>]>,
    prototypes: Box<[SerializePrototype<S>]>,
    heap_vars: Box<[SerializeHeapVarDescriptor<S>]>,
}

impl<'a, S> SerializePrototype<&'a S> {
    fn from_prototype(prototype: &'a Prototype<S>) -> Self {
        Self {
            reference: SerializeFunctionRef::from_function_ref(&prototype.reference),
            instructions: prototype
                .instructions
                .iter()
                .map(|&(inst, span)| {
                    (
                        SerializeInstruction::from_instruction(inst),
                        SerializeSpan::from_span(span),
                    )
                })
                .collect(),
            constants: prototype
                .constants
                .iter()
                .map(SerializeConstant::from_constant)
                .collect(),
            prototypes: prototype
                .prototypes
                .iter()
                .map(SerializePrototype::from_prototype)
                .collect(),
            heap_vars: prototype
                .heap_vars
                .iter()
                .map(SerializeHeapVarDescriptor::from_heap_var_descriptor)
                .collect(),
        }
    }
}

impl<S> SerializePrototype<S> {
    fn into_prototype(self) -> Prototype<S> {
        Prototype {
            reference: self.reference.into_function_ref(),
            instructions: self
                .instructions
                .into_iter()
                .map(|(inst, span)| (inst.to_instruction(), span.to_span()))
                .collect(),
            constants: self
                .constants
                .into_iter()
                .map(|c| c.into_constant())
                .collect(),
            prototypes: self
                .prototypes
                .into_iter()
                .map(|p| p.into_prototype())
                .collect(),
            heap_vars: self
                .heap_vars
                .into_iter()
                .map(|h| h.into_heap_var_descriptor())
                .collect(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct SerializeSourceChunk {
    name: String,
    line_breaks: Vec<usize>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SerializePrototypeOutput<S> {
    magic_vars: Vec<S>,
    exported_functions: Vec<(S, SerializePrototype<S>, usize)>,
    chunks: Vec<(SerializeSourceChunk, SerializePrototype<S>)>,
}

impl<'a, S> SerializePrototypeOutput<&'a S> {
    fn from_prototype_output(output: &'a PrototypeOutput<S>) -> Self {
        Self {
            magic_vars: output.magic_vars.iter().collect(),
            exported_functions: output
                .exported_functions
                .iter()
                .map(|(name, proto, chunk_index)| {
                    (
                        name,
                        SerializePrototype::from_prototype(proto),
                        *chunk_index,
                    )
                })
                .collect(),
            chunks: output
                .chunks
                .iter()
                .map(|(source_chunk, proto)| {
                    (
                        SerializeSourceChunk {
                            name: source_chunk.name.as_str().to_owned(),
                            line_breaks: source_chunk
                                .line_numbers
                                .clone()
                                .into_line_breaks()
                                .collect(),
                        },
                        SerializePrototype::from_prototype(proto),
                    )
                })
                .collect(),
        }
    }
}

impl<S> SerializePrototypeOutput<S> {
    fn into_prototype_output(self) -> PrototypeOutput<S> {
        PrototypeOutput {
            magic_vars: self.magic_vars,
            exported_functions: self
                .exported_functions
                .into_iter()
                .map(|(name, proto, chunk_index)| (name, proto.into_prototype(), chunk_index))
                .collect(),
            chunks: self
                .chunks
                .into_iter()
                .map(|(source_chunk, proto)| {
                    (
                        SourceChunk {
                            name: source_chunk.name.into(),
                            line_numbers: LineNumbers::from_line_breaks(source_chunk.line_breaks),
                        },
                        proto.into_prototype(),
                    )
                })
                .collect(),
        }
    }
}
