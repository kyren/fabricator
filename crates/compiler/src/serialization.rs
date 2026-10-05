use fabricator_vm as vm;
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
            &HeapVarDescriptor::Owned(heap_idx) => {
                SerializeHeapVarDescriptor::Owned(SerializedFromRaw::from_raw(heap_idx))
            }
            HeapVarDescriptor::Static(constant) => {
                SerializeHeapVarDescriptor::Static(SerializeConstant::from_constant(constant))
            }
            &HeapVarDescriptor::UpValue(heap_idx) => {
                SerializeHeapVarDescriptor::UpValue(SerializedFromRaw::from_raw(heap_idx))
            }
        }
    }
}

impl<S> SerializeHeapVarDescriptor<S> {
    fn into_heap_var_descriptor(self) -> HeapVarDescriptor<S> {
        match self {
            SerializeHeapVarDescriptor::Owned(heap_idx) => {
                HeapVarDescriptor::Owned(heap_idx.into_raw())
            }
            SerializeHeapVarDescriptor::Static(constant) => {
                HeapVarDescriptor::Static(constant.into_constant())
            }
            SerializeHeapVarDescriptor::UpValue(heap_idx) => {
                HeapVarDescriptor::UpValue(heap_idx.into_raw())
            }
        }
    }
}

trait SerializedFromRaw<I> {
    fn from_raw(v: I) -> Self;
}

trait SerializedIntoRaw<I> {
    fn into_raw(self) -> I;
}

macro_rules! impl_unit_serialize {
    ($ty:ty) => {
        impl SerializedFromRaw<$ty> for $ty {
            fn from_raw(v: $ty) -> Self {
                v
            }
        }

        impl SerializedIntoRaw<$ty> for $ty {
            fn into_raw(self) -> $ty {
                self
            }
        }
    };
}

impl_unit_serialize!(bool);

impl<I, S: SerializedFromRaw<I>> SerializedFromRaw<Option<I>> for Option<S> {
    fn from_raw(v: Option<I>) -> Self {
        v.map(|v| S::from_raw(v))
    }
}

impl<I, S: SerializedIntoRaw<I>> SerializedIntoRaw<Option<I>> for Option<S> {
    fn into_raw(self) -> Option<I> {
        self.map(|v| v.into_raw())
    }
}

macro_rules! make_idx {
    ($name:ident, $ty:ty) => {
        #[derive(Copy, Clone, Serialize, Deserialize)]
        #[serde(transparent)]
        struct $name($ty);

        impl SerializedFromRaw<vm::instructions::$name> for $name {
            #[inline]
            fn from_raw(v: vm::instructions::$name) -> Self {
                Self(v.0)
            }
        }

        impl SerializedIntoRaw<vm::instructions::$name> for $name {
            #[inline]
            fn into_raw(self) -> vm::instructions::$name {
                vm::instructions::$name(self.0)
            }
        }
    };
}

make_idx!(RegIdx, u8);
make_idx!(ConstIdx, u16);
make_idx!(HeapIdx, u16);
make_idx!(ProtoIdx, u16);
make_idx!(MagicIdx, u32);
make_idx!(InstIdx, u32);

macro_rules! define_serialize_instruction {
    ($(
        [$_category:ident] $(#[$_attr:meta])* $snake_name:ident = $name:ident { $($field:ident: $field_ty:ty),* $(,)? };
    )*) => {
        #[derive(Copy, Clone, Serialize, Deserialize)]
        enum SerializeInstruction {
            $(
                $name {
                    $($field: $field_ty),*
                }
            ),*
        }
    };
}

vm::instructions::for_each_instruction!(define_serialize_instruction);

macro_rules! define_instruction_conversion {
    ($(
        [$_category:ident] $(#[$_attr:meta])* $_snake_name:ident = $name:ident { $($field:ident: $field_ty:ty),* $(,)? };
    )*) => {
        impl SerializeInstruction {
            fn from_instruction(inst: vm::instructions::Instruction) -> Self {
                match inst {
                    $(
                        vm::instructions::Instruction::$name {
                            $($field),*
                        } => {
                            SerializeInstruction::$name {
                                $($field: SerializedFromRaw::from_raw($field)),*
                            }
                        }
                    ),*
                }
            }

            fn to_instruction(self) -> vm::instructions::Instruction {
                match self {
                    $(
                        SerializeInstruction::$name {
                            $($field),*
                        } => {
                            vm::instructions::Instruction::$name {
                                $($field: SerializedIntoRaw::into_raw($field)),*
                            }
                        }
                    ),*
                }
            }
        }
    };
}

vm::instructions::for_each_instruction!(define_instruction_conversion);

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
