use fabricator_vm::{
    self as vm, closure,
    instructions::{HeapIdx, Instruction, MagicIdx},
};
use gc_arena::{Collect, Gc, Mutation};

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
#[derive(Debug, Collect)]
#[collect(no_drop)]
pub struct Prototype<S> {
    pub reference: vm::FunctionRef<S>,
    pub bytecode: vm::ByteCode,
    pub constants: Box<[Constant<S>]>,
    pub prototypes: Box<[Prototype<S>]>,
    pub heap_vars: Box<[HeapVarDescriptor<S>]>,
}

impl<S: Clone> Clone for Prototype<S> {
    fn clone(&self) -> Self {
        self.clone_with_map_string(|s| s.clone())
    }
}

impl<S> Prototype<S> {
    pub fn clone_with_map_string<S2>(&self, map: impl Fn(&S) -> S2) -> Prototype<S2> {
        fn do_clone<S, S2>(this: &Prototype<S>, map: &impl Fn(&S) -> S2) -> Prototype<S2> {
            let reference = this.reference.as_string_ref().map_string(map);
            let constants = this
                .constants
                .iter()
                .map(|c| c.as_string_ref().map_string(map))
                .collect();
            let prototypes = this.prototypes.iter().map(|p| do_clone(p, map)).collect();
            let heap_vars = this
                .heap_vars
                .iter()
                .map(|h| h.as_string_ref().map_string(map))
                .collect();

            Prototype {
                reference,
                bytecode: this.bytecode.clone(),
                constants,
                prototypes,
                heap_vars,
            }
        }

        do_clone(self, &map)
    }

    pub fn map_magic_idx(self, map: impl Fn(MagicIdx) -> MagicIdx) -> Self {
        fn do_map<S>(this: Prototype<S>, map: &impl Fn(MagicIdx) -> MagicIdx) -> Prototype<S> {
            Prototype {
                reference: this.reference,
                bytecode: vm::ByteCode::encode(this.bytecode.decode().map(|(inst, span)| {
                    let inst = match inst {
                        Instruction::GetMagic { dest, magic } => Instruction::GetMagic {
                            dest,
                            magic: map(magic),
                        },
                        Instruction::SetMagic { magic, source } => Instruction::SetMagic {
                            magic: map(magic),
                            source,
                        },
                        inst => inst,
                    };
                    (inst, span)
                }))
                .unwrap(),
                constants: this.constants,
                prototypes: this
                    .prototypes
                    .into_iter()
                    .map(|p| do_map(p, map))
                    .collect(),
                heap_vars: this.heap_vars,
            }
        }

        do_map(self, &map)
    }
}

impl<'gc> Prototype<vm::String<'gc>> {
    /// The given `MagicSet` pointer must match the magic variables provided during codegen.
    pub fn into_vm(
        self,
        mc: &Mutation<'gc>,
        chunk: vm::Chunk<'gc>,
        magic: Gc<'gc, vm::MagicSet<'gc>>,
    ) -> Gc<'gc, vm::Prototype<'gc>> {
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
            bytecode,
            constants,
            prototypes,
            heap_vars,
        } = self;

        let reference = reference.map_string(|s| s.as_shared().clone());
        let constants = constants.into_iter().map(const_conv).collect();

        let prototypes = prototypes
            .into_iter()
            .map(|p| p.into_vm(mc, chunk, magic))
            .collect();

        let mut static_vars = Vec::new();
        let heap_vars = heap_vars
            .into_iter()
            .map(|heap_var| match heap_var {
                HeapVarDescriptor::Owned(idx) => closure::HeapVarDescriptor::Owned(idx),
                HeapVarDescriptor::Static(constant) => {
                    // There won't be more statics than there are heap vars, which each have a valid
                    // index.
                    let ind = static_vars.len().try_into().unwrap();
                    static_vars.push(closure::SharedValue::new(
                        mc,
                        const_conv(constant).to_value().into(),
                    ));
                    closure::HeapVarDescriptor::Static(ind)
                }
                HeapVarDescriptor::UpValue(idx) => closure::HeapVarDescriptor::UpValue(idx),
            })
            .collect();

        Gc::new(
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
            )
            .unwrap(),
        )
    }
}
