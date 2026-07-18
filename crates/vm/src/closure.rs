use std::{fmt, hash};

use gc_arena::{Collect, Gc, Lock, Mutation};
use thiserror::Error;

use crate::{
    debug::{Chunk, FunctionIdentifier, FunctionRef},
    instructions::{
        ByteCode, ConstIdx, HeapIdx, IndexType as _, InstIdx, Instruction, MagicIdx, ProtoIdx,
        RegIdx,
    },
    magic::MagicSet,
    object::Object,
    string::{SharedStr, String},
    value::Value,
};

#[derive(Debug, Copy, Clone, PartialEq, Collect)]
#[collect(no_drop)]
pub enum Constant<'gc> {
    Undefined,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String<'gc>),
}

impl<'gc> Constant<'gc> {
    pub fn to_value(self) -> Value<'gc> {
        match self {
            Constant::Undefined => Value::Undefined,
            Constant::Boolean(b) => Value::Boolean(b),
            Constant::Integer(i) => Value::Integer(i),
            Constant::Float(f) => Value::Float(f),
            Constant::String(s) => Value::String(s),
        }
    }
}

/// A shared [`Value`] that can be referenced by multiple closures with independent lifetimes.
pub type SharedValue<'gc> = Gc<'gc, Lock<Value<'gc>>>;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Collect)]
#[collect(require_static)]
pub enum HeapVarDescriptor {
    /// This heap variable is owned by a closure.
    ///
    /// Contains a "slot" for an owned heap variable. Slots may be re-used for a set of variables if
    /// their lifetimes do not overlap with each other and have `ResetHeap` instructions in-between.
    ///
    /// When the running closure created from this prototype executes and uses this heap variable,
    /// it will create a new, unique instance of it.
    Owned(HeapIdx),
    /// This heap variable is a prototype-level static.
    ///
    /// The index must be a valid index into the `static_vars` prototype table.
    Static(HeapIdx),
    /// This heap variable is a reference to a heap variable from a parent closure.
    ///
    /// Having a non-owned heap variable means that this prototype represents a function that closes
    /// over an upper variable, and the two functions need to share this variable with potentially
    /// different lifetimes.
    ///
    /// Contains the index into the *parent* heap variable list for the heap variable that this
    /// upvalue references.
    UpValue(HeapIdx),
}

#[derive(Debug, Error)]
pub enum PrototypeVerificationError {
    #[error("too many instructions: {0}")]
    InstructionOverflow(usize),
    #[error("too many prototypes: {0}")]
    PrototypeOverflow(usize),
    #[error("static index {0} is out of range of the list of static variables")]
    BadStaticIdx(HeapIdx),
    #[error("inner prototype has an invalid upvalue idx {0}, for prototype {1}")]
    BadUpValueIdx(HeapIdx, ProtoIdx),
    #[error("const idx {0} out of range at instruction {1}")]
    BadConstIdx(ConstIdx, InstIdx),
    #[error("heap idx {0} out of range at instruction {1}")]
    BadHeapIdx(HeapIdx, InstIdx),
    #[error("proto idx {0} out of range at instruction {1}")]
    BadProtoIdx(ProtoIdx, InstIdx),
    #[error("no magic variable with index {0} at instruction {1}")]
    BadMagicIdx(MagicIdx, InstIdx),
    #[error("field constant {0} is not a `Constant::String` at instruction {1}")]
    FieldIsNotString(ConstIdx, InstIdx),
    #[error(
        "index constant {0} is not an `Constant::Integer` convertible to `usize` at instruction {1}"
    )]
    IndexIsNotUsize(ConstIdx, InstIdx),
    #[error("cannot reset a shared heap variable {0} at instruction {1}")]
    ResetSharedHeap(HeapIdx, InstIdx),
}

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct Prototype<'gc> {
    chunk: Chunk<'gc>,
    reference: FunctionRef<SharedStr>,
    magic: Gc<'gc, MagicSet<'gc>>,
    bytecode: Gc<'gc, ByteCode>,
    constants: Box<[Constant<'gc>]>,
    prototypes: Box<[Gc<'gc, Prototype<'gc>>]>,
    static_vars: Box<[SharedValue<'gc>]>,
    heap_vars: Box<[HeapVarDescriptor]>,
    used_registers: usize,
    owned_heap: usize,
    constructor_super: Gc<'gc, Lock<Option<Object<'gc>>>>,
}

impl<'gc> Prototype<'gc> {
    pub fn new(
        mc: &Mutation<'gc>,
        chunk: Chunk<'gc>,
        reference: FunctionRef<SharedStr>,
        magic: Gc<'gc, MagicSet<'gc>>,
        bytecode: Gc<'gc, ByteCode>,
        constants: Box<[Constant<'gc>]>,
        prototypes: Box<[Gc<'gc, Prototype<'gc>>]>,
        static_vars: Box<[SharedValue<'gc>]>,
        heap_vars: Box<[HeapVarDescriptor]>,
    ) -> Result<Self, PrototypeVerificationError> {
        let mut owned_heap = 0;
        for &heap_var in &heap_vars {
            match heap_var {
                HeapVarDescriptor::Owned(idx) => {
                    owned_heap = owned_heap.max(idx.index() + 1);
                }
                HeapVarDescriptor::Static(idx) => {
                    if idx.index() >= static_vars.len() {
                        return Err(PrototypeVerificationError::BadStaticIdx(idx));
                    }
                }
                HeapVarDescriptor::UpValue(_) => {}
            }
        }

        for (inner_proto_idx, inner) in prototypes.iter().enumerate() {
            let inner_proto_idx = ProtoIdx::try_from(inner_proto_idx)
                .map_err(|_| PrototypeVerificationError::PrototypeOverflow(inner_proto_idx))?;

            for &inner_heap_var in &inner.heap_vars {
                match inner_heap_var {
                    HeapVarDescriptor::Owned(_) | HeapVarDescriptor::Static(_) => {}
                    HeapVarDescriptor::UpValue(upvalue_idx) => {
                        if (upvalue_idx.index()) >= heap_vars.len() {
                            return Err(PrototypeVerificationError::BadUpValueIdx(
                                upvalue_idx,
                                inner_proto_idx,
                            ));
                        }
                    }
                }
            }
        }

        let mut max_used_register = None;
        let mut mark_reg_idx = |reg_idx: RegIdx| {
            if max_used_register.is_none_or(|max: RegIdx| reg_idx.0 > max.0) {
                max_used_register = Some(reg_idx);
            }
        };

        for (inst_index, (inst, _)) in bytecode.decode().enumerate() {
            let inst_index = InstIdx::try_from(inst_index)
                .map_err(|_| PrototypeVerificationError::InstructionOverflow(inst_index))?;

            let verify_const_idx = |const_idx: ConstIdx| {
                if (const_idx.index()) < constants.len() {
                    Ok(())
                } else {
                    Err(PrototypeVerificationError::BadConstIdx(
                        const_idx, inst_index,
                    ))
                }
            };

            let verify_heap_idx = |heap_idx: HeapIdx| {
                if (heap_idx.index()) < heap_vars.len() {
                    Ok(())
                } else {
                    Err(PrototypeVerificationError::BadHeapIdx(heap_idx, inst_index))
                }
            };

            let verify_proto_idx = |proto_idx: ProtoIdx| {
                if (proto_idx.index()) < prototypes.len() {
                    Ok(())
                } else {
                    Err(PrototypeVerificationError::BadProtoIdx(
                        proto_idx, inst_index,
                    ))
                }
            };

            let verify_magic_idx = |magic_idx: MagicIdx| {
                magic
                    .get(magic_idx.index())
                    .map_err(|_| PrototypeVerificationError::BadMagicIdx(magic_idx, inst_index))
            };

            let verify_const_as_field = |const_idx: ConstIdx| {
                if matches!(constants[const_idx.index()], Constant::String(_)) {
                    Ok(())
                } else {
                    Err(PrototypeVerificationError::FieldIsNotString(
                        const_idx, inst_index,
                    ))
                }
            };

            match inst {
                Instruction::Undefined { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::Boolean { dest, .. } => {
                    mark_reg_idx(dest);
                }
                Instruction::LoadConstant { dest, constant } => {
                    mark_reg_idx(dest);
                    verify_const_idx(constant)?;
                }
                Instruction::GetHeap { dest, heap } => {
                    mark_reg_idx(dest);
                    verify_heap_idx(heap)?;
                }
                Instruction::SetHeap { heap, source } => {
                    verify_heap_idx(heap)?;
                    mark_reg_idx(source);
                }
                Instruction::ResetHeap { heap } => {
                    verify_heap_idx(heap)?;
                    if !matches!(heap_vars[heap.index()], HeapVarDescriptor::Owned(_)) {
                        return Err(PrototypeVerificationError::ResetSharedHeap(
                            heap, inst_index,
                        ));
                    }
                }
                Instruction::Globals { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::PushThis {} => {}
                Instruction::PopThis {} => {}
                Instruction::This { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::SetThis { source } => {
                    mark_reg_idx(source);
                }
                Instruction::Other { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::Closure { dest, proto, .. } => {
                    mark_reg_idx(dest);
                    verify_proto_idx(proto)?;
                }
                Instruction::CurrentClosure { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::ArgCount { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::ArgGet { dest, .. } => {
                    mark_reg_idx(dest);
                }
                Instruction::ArgGetAt { dest, index } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(index);
                }
                Instruction::NewObject { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::NewArray { dest } => {
                    mark_reg_idx(dest);
                }
                Instruction::GetField { dest, target, key } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(target);
                    mark_reg_idx(key);
                }
                Instruction::SetField { target, key, value } => {
                    mark_reg_idx(target);
                    mark_reg_idx(key);
                    mark_reg_idx(value);
                }
                Instruction::GetFieldConst { dest, target, key } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(target);
                    verify_const_idx(key)?;
                    verify_const_as_field(key)?;
                }
                Instruction::SetFieldConst { target, key, value } => {
                    mark_reg_idx(target);
                    verify_const_idx(key)?;
                    verify_const_as_field(key)?;
                    mark_reg_idx(value);
                }
                Instruction::GetIndex {
                    dest,
                    target,
                    index,
                } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(target);
                    mark_reg_idx(index);
                }
                Instruction::SetIndex {
                    target,
                    index,
                    value,
                } => {
                    mark_reg_idx(target);
                    mark_reg_idx(index);
                    mark_reg_idx(value);
                }
                Instruction::GetIndexConst {
                    dest,
                    target,
                    index,
                } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(target);
                    verify_const_idx(index)?;
                }
                Instruction::SetIndexConst {
                    target,
                    index,
                    value,
                } => {
                    mark_reg_idx(target);
                    verify_const_idx(index)?;
                    mark_reg_idx(value);
                }
                Instruction::Copy { dest, source } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(source);
                }
                Instruction::IsDefined { dest, arg }
                | Instruction::IsUndefined { dest, arg }
                | Instruction::Test { dest, arg }
                | Instruction::Not { dest, arg }
                | Instruction::Negate { dest, arg }
                | Instruction::BitNegate { dest, arg }
                | Instruction::Increment { dest, arg }
                | Instruction::Decrement { dest, arg } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(arg);
                }
                Instruction::Add { dest, left, right }
                | Instruction::Subtract { dest, left, right }
                | Instruction::Multiply { dest, left, right }
                | Instruction::Divide { dest, left, right }
                | Instruction::Remainder { dest, left, right }
                | Instruction::IntDivide { dest, left, right }
                | Instruction::IsEqual { dest, left, right }
                | Instruction::IsNotEqual { dest, left, right }
                | Instruction::IsLess { dest, left, right }
                | Instruction::IsLessEqual { dest, left, right }
                | Instruction::And { dest, left, right }
                | Instruction::Or { dest, left, right }
                | Instruction::Xor { dest, left, right }
                | Instruction::BitAnd { dest, left, right }
                | Instruction::BitOr { dest, left, right }
                | Instruction::BitXor { dest, left, right }
                | Instruction::BitShiftLeft { dest, left, right }
                | Instruction::BitShiftRight { dest, left, right }
                | Instruction::NullCoalesce { dest, left, right } => {
                    mark_reg_idx(dest);
                    mark_reg_idx(left);
                    mark_reg_idx(right);
                }
                Instruction::PushStackFrame {} => {}
                Instruction::PopStackFrame {} => {}
                Instruction::JoinStackFrame {} => {}
                Instruction::SplitStackFrame { .. } => {}
                Instruction::StackPush { source } => {
                    mark_reg_idx(source);
                }
                Instruction::StackPush2 { source_a, source_b } => {
                    mark_reg_idx(source_a);
                    mark_reg_idx(source_b);
                }
                Instruction::StackPush3 {
                    source_a,
                    source_b,
                    source_c,
                } => {
                    mark_reg_idx(source_a);
                    mark_reg_idx(source_b);
                    mark_reg_idx(source_c);
                }
                Instruction::StackPush4 {
                    source_a,
                    source_b,
                    source_c,
                    source_d,
                } => {
                    mark_reg_idx(source_a);
                    mark_reg_idx(source_b);
                    mark_reg_idx(source_c);
                    mark_reg_idx(source_d);
                }
                Instruction::StackPushArgs { .. } => {}
                Instruction::StackGet { dest, .. } => {
                    mark_reg_idx(dest);
                }
                Instruction::GetMagic { dest, magic } => {
                    mark_reg_idx(dest);
                    verify_magic_idx(magic)?;
                }
                Instruction::SetMagic { magic, source } => {
                    verify_magic_idx(magic)?;
                    mark_reg_idx(source);
                }
                Instruction::Throw { source } => {
                    mark_reg_idx(source);
                }
                Instruction::Jump { .. } => {}
                Instruction::JumpIf { arg, .. } => {
                    mark_reg_idx(arg);
                }
                Instruction::JumpIfUndefined { arg, .. } => {
                    mark_reg_idx(arg);
                }
                Instruction::JumpIfEqual { left, right, .. } => {
                    mark_reg_idx(left);
                    mark_reg_idx(right);
                }
                Instruction::JumpIfNotEqual { left, right, .. } => {
                    mark_reg_idx(left);
                    mark_reg_idx(right);
                }
                Instruction::JumpIfLess { left, right, .. } => {
                    mark_reg_idx(left);
                    mark_reg_idx(right);
                }
                Instruction::JumpIfLessEqual { left, right, .. } => {
                    mark_reg_idx(left);
                    mark_reg_idx(right);
                }
                Instruction::Call { func, this } => {
                    mark_reg_idx(func);
                    if let Some(this) = this {
                        mark_reg_idx(this);
                    }
                }
                Instruction::Return {} => {}
            }
        }

        let constructor_super = Gc::new(mc, Lock::new(None));

        Ok(Self {
            chunk,
            reference,
            magic,
            bytecode,
            constants,
            prototypes,
            static_vars,
            heap_vars,
            used_registers: max_used_register.map(|r| r.0 as usize + 1).unwrap_or(0),
            owned_heap,
            constructor_super,
        })
    }

    #[inline]
    pub fn chunk(&self) -> Chunk<'gc> {
        self.chunk
    }

    #[inline]
    pub fn reference(&self) -> &FunctionRef<SharedStr> {
        &self.reference
    }

    #[inline]
    pub fn identifier(&self) -> FunctionIdentifier<&str> {
        self.chunk.function_identifier(&self.reference)
    }

    #[inline]
    pub fn magic(&self) -> Gc<'gc, MagicSet<'gc>> {
        self.magic
    }

    #[inline]
    pub fn bytecode(&self) -> Gc<'gc, ByteCode> {
        self.bytecode
    }

    #[inline]
    pub fn constants(&self) -> &[Constant<'gc>] {
        &self.constants
    }

    #[inline]
    pub fn prototypes(&self) -> &[Gc<'gc, Prototype<'gc>>] {
        &self.prototypes
    }

    #[inline]
    pub fn static_vars(&self) -> &[SharedValue<'gc>] {
        &self.static_vars
    }

    #[inline]
    pub fn heap_vars(&self) -> &[HeapVarDescriptor] {
        &self.heap_vars
    }

    /// If it is not already created, associate a new `Object` with this prototype that defines the
    /// super-object of all constructed objects and return it.
    ///
    /// If it is already created, then return the existing super object.
    #[inline]
    pub fn init_constructor_super(&self, mc: &Mutation<'gc>) -> Object<'gc> {
        match self.constructor_super.get() {
            Some(obj) => obj,
            None => {
                let obj = Object::new(mc);
                self.constructor_super.set(mc, Some(obj));
                obj
            }
        }
    }

    #[inline]
    pub fn constructor_super(&self) -> Option<Object<'gc>> {
        self.constructor_super.get()
    }

    #[inline]
    pub fn used_registers(&self) -> usize {
        self.used_registers
    }

    /// Returns the required length for a buffer of owned heap values for this prototype.
    ///
    /// This will return 1 + the maximum "slot" used by any `HeapVarDescriptor::Owned` variable.
    #[inline]
    pub fn owned_heap(&self) -> usize {
        self.owned_heap
    }

    pub fn has_upvalues(&self) -> bool {
        for &h in &self.heap_vars {
            if matches!(h, HeapVarDescriptor::UpValue(_)) {
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub enum HeapVar<'gc> {
    /// A `HeapVarDescriptor::Owned` heap variable.
    Owned(HeapIdx),
    /// A shared heap variable (either a static or an upvalue).
    Shared(SharedValue<'gc>),
}

#[derive(Debug, Error)]
#[error("missing upvalue")]
pub struct MissingUpValue;

#[derive(Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Closure<'gc>(Gc<'gc, ClosureInner<'gc>>);

#[derive(Collect)]
#[collect(no_drop)]
pub struct ClosureInner<'gc> {
    proto: Gc<'gc, Prototype<'gc>>,
    this: Value<'gc>,
    heap: Gc<'gc, Box<[HeapVar<'gc>]>>,
}

impl<'gc> fmt::Debug for Closure<'gc> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_tuple("Function")
            .field(&Gc::as_ptr(self.0))
            .finish()
    }
}

impl<'gc> PartialEq for Closure<'gc> {
    fn eq(&self, other: &Closure<'gc>) -> bool {
        Gc::ptr_eq(self.0, other.0)
    }
}

impl<'gc> Eq for Closure<'gc> {}

impl<'gc> hash::Hash for Closure<'gc> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        Gc::as_ptr(self.0).hash(state)
    }
}

impl<'gc> Closure<'gc> {
    /// Create a new top-level closure.
    ///
    /// Given prototype must not have any upvalues.
    pub fn new(
        mc: &Mutation<'gc>,
        proto: Gc<'gc, Prototype<'gc>>,
        this: Value<'gc>,
    ) -> Result<Self, MissingUpValue> {
        let mut heap = Vec::new();
        for &h in &proto.heap_vars {
            match h {
                HeapVarDescriptor::Owned(idx) => {
                    heap.push(HeapVar::Owned(idx));
                }
                HeapVarDescriptor::Static(idx) => {
                    heap.push(HeapVar::Shared(proto.static_vars[idx.index()]))
                }
                HeapVarDescriptor::UpValue(_) => {
                    return Err(MissingUpValue);
                }
            }
        }
        Self::from_parts(mc, proto, this, Gc::new(mc, heap.into_boxed_slice()))
    }

    /// Create a new closure using the given `upvalues` array to lookup any required upvalues.
    pub fn from_parts(
        mc: &Mutation<'gc>,
        proto: Gc<'gc, Prototype<'gc>>,
        this: Value<'gc>,
        heap: Gc<'gc, Box<[HeapVar<'gc>]>>,
    ) -> Result<Self, MissingUpValue> {
        Ok(Self(Gc::new(mc, ClosureInner { proto, this, heap })))
    }

    #[inline]
    pub fn from_inner(inner: Gc<'gc, ClosureInner<'gc>>) -> Self {
        Self(inner)
    }

    #[inline]
    pub fn into_inner(self) -> Gc<'gc, ClosureInner<'gc>> {
        self.0
    }

    #[inline]
    pub fn prototype(self) -> Gc<'gc, Prototype<'gc>> {
        self.0.proto
    }

    /// Returns the currently bound `self` object.
    ///
    /// Will return `Value::Undefined` if there is no bound `self` object set.
    #[inline]
    pub fn this(self) -> Value<'gc> {
        self.0.this
    }

    /// Return a clone of this closure with the embedded `self` value changed to the provided one.
    ///
    /// If `Value::Undefined` is provided, then the bound `self` object will be removed.
    #[inline]
    pub fn rebind(self, mc: &Mutation<'gc>, this: Value<'gc>) -> Closure<'gc> {
        Self(Gc::new(
            mc,
            ClosureInner {
                proto: self.0.proto,
                heap: self.0.heap,
                this,
            },
        ))
    }

    #[inline]
    pub fn heap(self) -> &'gc [HeapVar<'gc>] {
        &self.0.as_ref().heap
    }
}
