use std::convert::Infallible;

use fabricator_vm::{
    self as vm,
    closure::{self, PrototypeVerificationError},
    instructions::{ByteCodeEncodingError, HeapIdx, Instruction, MagicIdx},
};
use gc_arena::{Collect, Gc, Mutation};
use thiserror::Error;

use crate::constant::Constant;

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub enum HeapVarDescriptor<S> {
    Owned(HeapIdx),
    Static(Constant<S>),
    UpValue(HeapIdx),
}

impl<S> HeapVarDescriptor<S> {
    #[must_use]
    pub fn as_string_ref(&self) -> HeapVarDescriptor<&S> {
        match *self {
            HeapVarDescriptor::Owned(idx) => HeapVarDescriptor::Owned(idx),
            HeapVarDescriptor::Static(ref constant) => {
                HeapVarDescriptor::Static(constant.as_string_ref())
            }
            HeapVarDescriptor::UpValue(idx) => HeapVarDescriptor::UpValue(idx),
        }
    }

    #[must_use]
    pub fn map_string<S2>(self, map: impl Fn(S) -> S2) -> HeapVarDescriptor<S2> {
        match self {
            HeapVarDescriptor::Owned(idx) => HeapVarDescriptor::Owned(idx),
            HeapVarDescriptor::Static(constant) => {
                HeapVarDescriptor::Static(constant.map_string(map))
            }
            HeapVarDescriptor::UpValue(idx) => HeapVarDescriptor::UpValue(idx),
        }
    }
}

/// A compiler generated prototype with compiled bytecode.
///
/// This is distinct from a VM prototype in that it may not use VM interned strings, does not store
/// child prototypes as a `Gc` pointer, and does not have a reference to a concrete `MagicSet`.
#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct Prototype<S> {
    pub reference: vm::FunctionRef<S>,
    pub instructions: Box<[(Instruction, vm::Span)]>,
    pub constants: Box<[Constant<S>]>,
    pub prototypes: Box<[Prototype<S>]>,
    pub heap_vars: Box<[HeapVarDescriptor<S>]>,
}

impl<S> Prototype<S> {
    pub fn map_string<S2>(self, map: impl Fn(S) -> S2) -> Prototype<S2> {
        fn do_map_string<S, S2>(this: Prototype<S>, map: &impl Fn(S) -> S2) -> Prototype<S2> {
            let reference = this.reference.map_string(map);
            let constants = this
                .constants
                .into_iter()
                .map(|c| c.map_string(map))
                .collect();
            let prototypes = this
                .prototypes
                .into_iter()
                .map(|p| do_map_string(p, map))
                .collect();
            let heap_vars = this
                .heap_vars
                .into_iter()
                .map(|h| h.map_string(map))
                .collect();

            Prototype {
                reference,
                instructions: this.instructions.clone(),
                constants,
                prototypes,
                heap_vars,
            }
        }

        do_map_string(self, &map)
    }

    pub fn try_map_magic_idx<E>(
        self,
        map: impl Fn(MagicIdx) -> Result<MagicIdx, E>,
    ) -> Result<Self, E> {
        fn do_map<S, E>(
            this: Prototype<S>,
            map: &impl Fn(MagicIdx) -> Result<MagicIdx, E>,
        ) -> Result<Prototype<S>, E> {
            Ok(Prototype {
                reference: this.reference,
                instructions: this
                    .instructions
                    .into_iter()
                    .map(|(inst, span)| {
                        let inst = match inst {
                            Instruction::GetMagic { dest, magic } => Instruction::GetMagic {
                                dest,
                                magic: map(magic)?,
                            },
                            Instruction::SetMagic { magic, source } => Instruction::SetMagic {
                                magic: map(magic)?,
                                source,
                            },
                            inst => inst,
                        };
                        Ok((inst, span))
                    })
                    .collect::<Result<_, _>>()?,
                constants: this.constants,
                prototypes: this
                    .prototypes
                    .into_iter()
                    .map(|p| do_map(p, map))
                    .collect::<Result<_, _>>()?,
                heap_vars: this.heap_vars,
            })
        }

        do_map(self, &map)
    }

    pub fn map_magic_idx(self, map: impl Fn(MagicIdx) -> MagicIdx) -> Self {
        let Ok(r) = self.try_map_magic_idx::<Infallible>(|idx| Ok(map(idx)));
        r
    }

    pub fn has_upvalues(&self) -> bool {
        for h in &self.heap_vars {
            if matches!(h, HeapVarDescriptor::UpValue(_)) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Error)]
pub enum VmPrototypeError {
    #[error("prototype heap var descriptors overflow HeapIdx")]
    HeapVarOverflow,
    #[error("{0}")]
    ByteCodeEncodingError(#[from] ByteCodeEncodingError),
    #[error("{0}")]
    PrototypeVerificationError(#[from] PrototypeVerificationError),
}

impl<'gc> Prototype<vm::String<'gc>> {
    /// The given `MagicSet` pointer must match the magic variables provided during codegen.
    pub fn into_vm(
        self,
        mc: &Mutation<'gc>,
        chunk: vm::Chunk<'gc>,
        magic: Gc<'gc, vm::MagicSet<'gc>>,
    ) -> Result<Gc<'gc, vm::Prototype<'gc>>, VmPrototypeError> {
        fn const_conv<'gc>(c: Constant<vm::String<'gc>>) -> vm::Constant<'gc> {
            match c {
                Constant::Undefined => vm::Constant::Undefined,
                Constant::Boolean(b) => vm::Constant::Boolean(b),
                Constant::Integer(i) => vm::Constant::Integer(i),
                Constant::Float(f) => vm::Constant::Float(f),
                Constant::String(s) => vm::Constant::String(s),
            }
        }

        let Self {
            reference,
            instructions,
            constants,
            prototypes,
            heap_vars,
        } = self;

        let reference = reference.map_string(|s| s.as_shared().clone());
        let bytecode = vm::ByteCode::encode(instructions)?;
        let constants = constants.into_iter().map(const_conv).collect();

        let prototypes = prototypes
            .into_iter()
            .map(|p| p.into_vm(mc, chunk, magic))
            .collect::<Result<_, _>>()?;

        let mut static_vars = Vec::new();
        let heap_vars = heap_vars
            .into_iter()
            .map(|heap_var| {
                Ok(match heap_var {
                    HeapVarDescriptor::Owned(idx) => closure::HeapVarDescriptor::Owned(idx),
                    HeapVarDescriptor::Static(constant) => {
                        let ind = static_vars
                            .len()
                            .try_into()
                            .map_err(|_| VmPrototypeError::HeapVarOverflow)?;
                        static_vars.push(closure::SharedValue::new(
                            mc,
                            const_conv(constant).to_value().into(),
                        ));
                        closure::HeapVarDescriptor::Static(ind)
                    }
                    HeapVarDescriptor::UpValue(idx) => closure::HeapVarDescriptor::UpValue(idx),
                })
            })
            .collect::<Result<_, VmPrototypeError>>()?;

        Ok(Gc::new(
            mc,
            vm::Prototype::new(
                mc,
                chunk,
                reference,
                magic,
                Gc::new(mc, bytecode),
                constants,
                prototypes,
                static_vars.into_boxed_slice(),
                heap_vars,
            )?,
        ))
    }
}
