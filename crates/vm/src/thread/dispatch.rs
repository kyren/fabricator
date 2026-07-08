use std::ops::ControlFlow;

use gc_arena::Gc;
use thiserror::Error;

use crate::{
    array::Array,
    closure::{Closure, Constant, HeapVar, HeapVarDescriptor},
    error::{Error, ExternValue, RuntimeError, ScriptError},
    instructions::{self, ConstIdx, HeapIdx, IndexType as _, MagicIdx, ProtoIdx, RegIdx, StackIdx},
    interpreter::Context,
    object::Object,
    string::String,
    thread::{thread::OwnedHeapVar, vec_end_slice::VecEndSlice},
    value::{Function, Value},
};

#[derive(Debug, Clone, Error)]
pub enum OpError {
    #[error("bad unary op {op:?} {arg:?}")]
    BadUnOp { op: &'static str, arg: ExternValue },
    #[error("bad binary op {left:?} {op:?} {right:?}")]
    BadBinOp {
        op: &'static str,
        left: ExternValue,
        right: ExternValue,
    },
    #[error("bad object {object:?}")]
    BadObject { object: ExternValue },
    #[error("bad key {key:?}")]
    BadKey { key: ExternValue },
    #[error("{target:?} does not allow indexing")]
    NotIndexable { target: ExternValue },
    #[error("{target:?} does not allow multi-indexing")]
    NotMultiIndexable { target: ExternValue },
    #[error("bad index {index:?} of {target:?}")]
    BadIndex {
        target: ExternValue,
        index: ExternValue,
    },
    #[error("no such field {field:?} in {target:?}")]
    NoSuchField {
        target: ExternValue,
        field: ExternValue,
    },
    #[error("bad call of {target:?}")]
    BadCall { target: &'static str },
    #[error("bad index value {index}")]
    BadIndexValue { index: ExternValue },
    #[error("not enough stack frames in {op:?}")]
    InvalidStackFrames { op: &'static str },
}

#[derive(Debug, Error)]
#[error("out of bounds access on array for index {0}")]
pub struct ArrayBoundsError(usize);

pub(super) enum Next<'gc> {
    Call {
        function: Function<'gc>,
        args_bottom: usize,
        this: Value<'gc>,
    },
    Return {
        returns_bottom: usize,
    },
}

pub(super) struct Dispatch<'gc, 'a> {
    ctx: Context<'gc>,
    closure: Closure<'gc>,
    // The register slice is fixed size to avoid bounds checks.
    registers: &'a mut [Value<'gc>; 256],
    stack: VecEndSlice<'a, Value<'gc>>,
    stack_frame_boundaries: VecEndSlice<'a, usize>,
    this: VecEndSlice<'a, Value<'gc>>,
    heap: &'a mut [OwnedHeapVar<'gc>],
}

impl<'gc, 'a> Dispatch<'gc, 'a> {
    pub(super) fn new(
        ctx: Context<'gc>,
        closure: Closure<'gc>,
        registers: &'a mut [Value<'gc>; 256],
        stack: VecEndSlice<'a, Value<'gc>>,
        stack_frame_boundaries: VecEndSlice<'a, usize>,
        this: VecEndSlice<'a, Value<'gc>>,
        heap: &'a mut [OwnedHeapVar<'gc>],
    ) -> Self {
        Self {
            ctx,
            closure,
            registers,
            heap,
            stack,
            stack_frame_boundaries,
            this,
        }
    }
}

impl<'gc, 'a> Dispatch<'gc, 'a> {
    #[inline]
    fn get_this(&self) -> Value<'gc> {
        self.this
            .last()
            .or_else(|| self.this.below().last())
            .copied()
            .unwrap_or_else(|| self.ctx.globals().into())
    }

    #[inline]
    fn get_other(&self) -> Value<'gc> {
        self.this
            .iter()
            .rev()
            .chain(self.this.below().iter().rev())
            .copied()
            .nth(1)
            .unwrap_or_else(|| self.ctx.globals().into())
    }

    #[inline]
    fn get_arg_count(&self) -> usize {
        self.stack_frame_boundaries
            .first()
            .copied()
            .unwrap_or(self.stack.len())
    }

    #[inline]
    fn do_get_field(&self, obj: Value<'gc>, key: String<'gc>) -> Result<Value<'gc>, RuntimeError> {
        match obj {
            Value::Object(object) => object.get(key).ok_or_else(|| {
                OpError::NoSuchField {
                    target: obj.into(),
                    field: Value::from(key).into(),
                }
                .into()
            }),
            Value::UserData(user_data) => Ok(user_data.get_field(self.ctx, key)?),
            _ => Err(OpError::BadObject { object: obj.into() }.into()),
        }
    }

    #[inline]
    fn do_set_field(
        &self,
        obj: Value<'gc>,
        key: String<'gc>,
        value: Value<'gc>,
    ) -> Result<(), RuntimeError> {
        match obj {
            Value::Object(object) => {
                object.set(&self.ctx, key, value);
            }
            Value::UserData(user_data) => {
                user_data.set_field(self.ctx, key, value)?;
            }
            _ => {
                return Err(OpError::BadObject { object: obj.into() }.into());
            }
        }

        Ok(())
    }

    #[inline]
    fn do_get_index(
        &self,
        target: Value<'gc>,
        indexes: &[Value<'gc>],
    ) -> Result<Value<'gc>, RuntimeError> {
        match target {
            Value::Object(object) => {
                if indexes.len() != 1 {
                    return Err(OpError::NotMultiIndexable {
                        target: target.into(),
                    }
                    .into());
                }
                let Some(index) = indexes[0].coerce_string(self.ctx) else {
                    return Err(OpError::BadIndex {
                        target: target.into(),
                        index: indexes[0].into(),
                    }
                    .into());
                };
                Ok(object.get(index).unwrap_or_default())
            }
            Value::Array(array) => {
                if indexes.len() != 1 {
                    return Err(OpError::NotMultiIndexable {
                        target: target.into(),
                    }
                    .into());
                }
                let index = indexes[0]
                    .cast_integer()
                    .and_then(|i| i.try_into().ok())
                    .ok_or_else(|| OpError::BadIndex {
                        target: target.into(),
                        index: indexes[0].into(),
                    })?;
                Ok(array.get(index).ok_or(ArrayBoundsError(index))?)
            }
            Value::UserData(user_data) => Ok(user_data.get_index(self.ctx, indexes)?),
            _ => Err(OpError::NotIndexable {
                target: target.into(),
            }
            .into()),
        }
    }

    #[inline]
    fn do_set_index(
        &self,
        target: Value<'gc>,
        indexes: &[Value<'gc>],
        value: Value<'gc>,
    ) -> Result<(), RuntimeError> {
        match target {
            Value::Object(object) => {
                if indexes.len() != 1 {
                    return Err(OpError::NotMultiIndexable {
                        target: target.into(),
                    }
                    .into());
                }
                let Some(index) = indexes[0].coerce_string(self.ctx) else {
                    return Err(OpError::BadIndex {
                        target: target.into(),
                        index: indexes[0].into(),
                    }
                    .into());
                };
                object.set(&self.ctx, index, value);
                Ok(())
            }
            Value::Array(array) => {
                if indexes.len() != 1 {
                    return Err(OpError::NotMultiIndexable {
                        target: target.into(),
                    }
                    .into());
                }

                let index = indexes[0]
                    .cast_integer()
                    .and_then(|i| i.try_into().ok())
                    .ok_or_else(|| OpError::BadIndex {
                        target: target.into(),
                        index: indexes[0].into(),
                    })?;
                array.set(&self.ctx, index, value);
                Ok(())
            }
            Value::UserData(user_data) => Ok(user_data.set_index(self.ctx, indexes, value)?),
            _ => Err(OpError::NotIndexable {
                target: target.into(),
            }
            .into()),
        }
    }
}

impl<'gc, 'a> instructions::Dispatch for Dispatch<'gc, 'a> {
    type Break = Next<'gc>;
    type Error = Error<'gc>;

    #[inline]
    fn undefined(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = Value::Undefined;
        Ok(())
    }

    #[inline]
    fn boolean(&mut self, dest: RegIdx, value: bool) -> Result<(), Self::Error> {
        self.registers[dest.index()] = Value::Boolean(value);
        Ok(())
    }

    #[inline]
    fn load_constant(&mut self, dest: RegIdx, constant: ConstIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] =
            self.closure.prototype().constants()[constant.index()].to_value();
        Ok(())
    }

    #[inline]
    fn get_heap(&mut self, dest: RegIdx, heap: HeapIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = match self.closure.heap()[heap.index()] {
            HeapVar::Owned(idx) => self.heap[idx.index()].get(),
            HeapVar::Shared(v) => v.get(),
        };
        Ok(())
    }

    #[inline]
    fn set_heap(&mut self, heap: HeapIdx, source: RegIdx) -> Result<(), Self::Error> {
        let source = self.registers[source.index()];
        match self.closure.heap()[heap.index()] {
            HeapVar::Owned(idx) => self.heap[idx.index()].set(&self.ctx, source),
            HeapVar::Shared(v) => v.set(&self.ctx, source),
        };
        Ok(())
    }

    #[inline]
    fn reset_heap(&mut self, heap: HeapIdx) -> Result<(), Self::Error> {
        match self.closure.heap()[heap.index()] {
            HeapVar::Owned(idx) => {
                self.heap[idx.index()] = OwnedHeapVar::unique(Value::Undefined);
                Ok(())
            }
            HeapVar::Shared(_) => panic!("reset of shared heap var"),
        }
    }

    #[inline]
    fn globals(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.ctx.globals().into();
        Ok(())
    }

    #[inline]
    fn push_this(&mut self) -> Result<(), Self::Error> {
        self.this.push_back(self.get_this());
        Ok(())
    }

    #[inline]
    fn pop_this(&mut self) -> Result<(), Self::Error> {
        self.this.pop_back();
        Ok(())
    }

    #[inline]
    fn this(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.get_this();
        Ok(())
    }

    #[inline]
    fn set_this(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        if let Some(last) = self.this.last_mut() {
            *last = self.registers[source.index()];
        }
        Ok(())
    }

    #[inline]
    fn other(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.get_other();
        Ok(())
    }

    #[inline]
    fn closure(
        &mut self,
        dest: RegIdx,
        proto: ProtoIdx,
        bind_this: bool,
    ) -> Result<(), Self::Error> {
        let proto = self.closure.prototype().prototypes()[proto.index()];

        let mut heap = Vec::new();
        for &hd in proto.heap_vars() {
            match hd {
                HeapVarDescriptor::Owned(idx) => {
                    heap.push(HeapVar::Owned(idx));
                }
                HeapVarDescriptor::Static(idx) => {
                    heap.push(HeapVar::Shared(proto.static_vars()[idx.index()]))
                }
                HeapVarDescriptor::UpValue(idx) => {
                    heap.push(HeapVar::Shared(match self.closure.heap()[idx.index()] {
                        HeapVar::Owned(idx) => self.heap[idx.index()].make_shared(&self.ctx),
                        HeapVar::Shared(v) => v,
                    }));
                }
            }
        }

        // Inner closures bind the current `self` value if the `bind_this` flag is set, otherwise
        // they are created unbound.
        self.registers[dest.index()] = Closure::from_parts(
            &self.ctx,
            proto,
            if bind_this {
                self.get_this()
            } else {
                Value::Undefined
            },
            Gc::new(&self.ctx, heap.into_boxed_slice()),
        )?
        .into();
        Ok(())
    }

    #[inline]
    fn current_closure(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.closure.into();
        Ok(())
    }

    #[inline]
    fn arg_count(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = (self.get_arg_count() as i64).into();
        Ok(())
    }

    #[inline]
    fn arg_get(&mut self, dest: RegIdx, index: StackIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = if index.index() < self.get_arg_count() {
            self.stack[index.index()]
        } else {
            Value::Undefined
        };
        Ok(())
    }

    #[inline]
    fn arg_get_at(&mut self, dest: RegIdx, index: RegIdx) -> Result<(), Self::Error> {
        let arg_idx = self.registers[index.index()];
        let arg_idx: usize = arg_idx
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadIndexValue {
                index: arg_idx.into(),
            })?;

        self.registers[dest.index()] = if arg_idx < self.get_arg_count() {
            self.stack[arg_idx]
        } else {
            Value::Undefined
        };
        Ok(())
    }

    #[inline]
    fn new_object(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = Object::new(&self.ctx).into();
        Ok(())
    }

    #[inline]
    fn new_array(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = Array::new(&self.ctx).into();
        Ok(())
    }

    #[inline]
    fn get_field(&mut self, dest: RegIdx, object: RegIdx, key: RegIdx) -> Result<(), Self::Error> {
        let key_val = self.registers[key.index()];
        let Some(key) = key_val.as_string() else {
            return Err(OpError::BadKey {
                key: key_val.into(),
            }
            .into());
        };

        self.registers[dest.index()] = self.do_get_field(self.registers[object.index()], key)?;
        Ok(())
    }

    #[inline]
    fn set_field(&mut self, object: RegIdx, key: RegIdx, value: RegIdx) -> Result<(), Self::Error> {
        let key_val = self.registers[key.index()];
        let Some(key) = key_val.as_string() else {
            return Err(OpError::BadKey {
                key: key_val.into(),
            }
            .into());
        };
        Ok(self.do_set_field(
            self.registers[object.index()],
            key,
            self.registers[value.index()],
        )?)
    }

    #[inline]
    fn get_field_const(
        &mut self,
        dest: RegIdx,
        object: RegIdx,
        key: ConstIdx,
    ) -> Result<(), Self::Error> {
        let Constant::String(key) = self.closure.prototype().constants()[key.index()] else {
            panic!("const key is not a string");
        };
        self.registers[dest.index()] = self.do_get_field(self.registers[object.index()], key)?;
        Ok(())
    }

    #[inline]
    fn set_field_const(
        &mut self,
        object: RegIdx,
        key: ConstIdx,
        value: RegIdx,
    ) -> Result<(), Self::Error> {
        let Constant::String(key) = self.closure.prototype().constants()[key.index()] else {
            panic!("const key is not a string");
        };
        Ok(self.do_set_field(
            self.registers[object.index()],
            key,
            self.registers[value.index()],
        )?)
    }

    #[inline]
    fn get_index(&mut self, dest: RegIdx, array: RegIdx, index: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.do_get_index(
            self.registers[array.index()],
            &[self.registers[index.index()]],
        )?;
        Ok(())
    }

    #[inline]
    fn set_index(
        &mut self,
        array: RegIdx,
        index: RegIdx,
        value: RegIdx,
    ) -> Result<(), Self::Error> {
        self.do_set_index(
            self.registers[array.index()],
            &[self.registers[index.index()]],
            self.registers[value.index()],
        )?;
        Ok(())
    }

    #[inline]
    fn get_index_const(
        &mut self,
        dest: RegIdx,
        array: RegIdx,
        index: ConstIdx,
    ) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.do_get_index(
            self.registers[array.index()],
            &[self.closure.prototype().constants()[index.index()].to_value()],
        )?;
        Ok(())
    }

    #[inline]
    fn set_index_const(
        &mut self,
        array: RegIdx,
        index: ConstIdx,
        value: RegIdx,
    ) -> Result<(), Self::Error> {
        self.do_set_index(
            self.registers[array.index()],
            &[self.closure.prototype().constants()[index.index()].to_value()],
            self.registers[value.index()],
        )?;
        Ok(())
    }

    #[inline]
    fn copy(&mut self, dest: RegIdx, source: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.registers[source.index()];
        Ok(())
    }

    #[inline]
    fn is_defined(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = (!self.registers[arg.index()].is_undefined()).into();
        Ok(())
    }

    #[inline]
    fn is_undefined(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.registers[arg.index()].is_undefined().into();
        Ok(())
    }

    #[inline]
    fn test(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = self.registers[arg.index()].cast_bool().into();
        Ok(())
    }

    #[inline]
    fn not(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest.index()] = (!self.registers[arg.index()].cast_bool()).into();
        Ok(())
    }

    #[inline]
    fn negate(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg.index()];
        self.registers[dest.index()] = arg.negate().ok_or_else(|| OpError::BadUnOp {
            op: "neg",
            arg: arg.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn bit_negate(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg.index()];
        self.registers[dest.index()] = arg
            .bit_negate()
            .ok_or_else(|| OpError::BadUnOp {
                op: "bit_neg",
                arg: arg.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn increment(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg.index()];
        self.registers[dest.index()] =
            arg.add(Value::Integer(1)).ok_or_else(|| OpError::BadUnOp {
                op: "inc",
                arg: arg.into(),
            })?;
        Ok(())
    }

    #[inline]
    fn decrement(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg.index()];
        self.registers[dest.index()] =
            arg.sub(Value::Integer(1)).ok_or_else(|| OpError::BadUnOp {
                op: "dec",
                arg: arg.into(),
            })?;
        Ok(())
    }

    #[inline]
    fn add(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .add_or_append(self.ctx, right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "add",
                left: left.into(),
                right: right.into(),
            })?;
        Ok(())
    }

    #[inline]
    fn subtract(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.sub(right).ok_or_else(|| OpError::BadBinOp {
            op: "sub",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn multiply(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.mult(right).ok_or_else(|| OpError::BadBinOp {
            op: "mult",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn divide(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.div(right).ok_or_else(|| OpError::BadBinOp {
            op: "div",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn remainder(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.rem(right).ok_or_else(|| OpError::BadBinOp {
            op: "rem",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn int_divide(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .idiv(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "int_div",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn is_equal(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.equal(right).into();
        Ok(())
    }

    #[inline]
    fn is_not_equal(
        &mut self,
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    ) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = (!left.equal(right)).into();
        Ok(())
    }

    #[inline]
    fn is_less(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        self.registers[dest.index()] = left
            .less_than(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "is_less",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn is_less_equal(
        &mut self,
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    ) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        self.registers[dest.index()] = left
            .less_equal(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "is_less_eq",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn and(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.and(right).into();
        Ok(())
    }

    #[inline]
    fn or(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.or(right).into();
        Ok(())
    }

    #[inline]
    fn xor(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.xor(right).into();
        Ok(())
    }

    #[inline]
    fn bit_and(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .bit_and(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "bit_and",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn bit_or(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .bit_or(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "bit_or",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn bit_xor(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .bit_xor(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "bit_xor",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn bit_shift_left(
        &mut self,
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    ) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .bit_shift_left(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "bit_shl",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn bit_shift_right(
        &mut self,
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    ) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left
            .bit_shift_right(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "bit_shr",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn null_coalesce(
        &mut self,
        dest: RegIdx,
        left: RegIdx,
        right: RegIdx,
    ) -> Result<(), Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        let dest = &mut self.registers[dest.index()];
        *dest = left.null_coalesce(right);
        Ok(())
    }

    #[inline]
    fn push_stack_frame(&mut self) -> Result<(), Self::Error> {
        self.stack_frame_boundaries.push_back(self.stack.len());
        Ok(())
    }

    #[inline]
    fn pop_stack_frame(&mut self) -> Result<(), Self::Error> {
        if let Some(boundary) = self.stack_frame_boundaries.pop_back() {
            self.stack.truncate(boundary);
            Ok(())
        } else {
            Err(OpError::InvalidStackFrames {
                op: "pop_stack_frame",
            }
            .into())
        }
    }

    #[inline]
    fn join_stack_frame(&mut self) -> Result<(), Self::Error> {
        if self.stack_frame_boundaries.len() < 2 {
            Err(OpError::InvalidStackFrames {
                op: "join_stack_frame",
            }
            .into())
        } else {
            self.stack_frame_boundaries.pop_back();
            Ok(())
        }
    }

    #[inline]
    fn split_stack_frame(&mut self, base: StackIdx) -> Result<(), Self::Error> {
        if let Some(&boundary) = self.stack_frame_boundaries.last() {
            self.stack_frame_boundaries
                .push_back(boundary + base.index());
            Ok(())
        } else {
            Err(OpError::InvalidStackFrames {
                op: "split_stack_frame",
            }
            .into())
        }
    }

    #[inline]
    fn stack_push(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        if self.stack_frame_boundaries.is_empty() {
            return Err(OpError::InvalidStackFrames { op: "stack_push" }.into());
        }
        self.stack.push_back(self.registers[source.index()]);
        Ok(())
    }

    #[inline]
    fn stack_push_2(&mut self, source_a: RegIdx, source_b: RegIdx) -> Result<(), Self::Error> {
        if self.stack_frame_boundaries.is_empty() {
            return Err(OpError::InvalidStackFrames { op: "stack_push" }.into());
        }
        self.stack.extend([
            self.registers[source_a.index()],
            self.registers[source_b.index()],
        ]);
        Ok(())
    }

    #[inline]
    fn stack_push_3(
        &mut self,
        source_a: RegIdx,
        source_b: RegIdx,
        source_c: RegIdx,
    ) -> Result<(), Self::Error> {
        if self.stack_frame_boundaries.is_empty() {
            return Err(OpError::InvalidStackFrames { op: "stack_push" }.into());
        }
        self.stack.extend([
            self.registers[source_a.index()],
            self.registers[source_b.index()],
            self.registers[source_c.index()],
        ]);
        Ok(())
    }

    #[inline]
    fn stack_push_4(
        &mut self,
        source_a: RegIdx,
        source_b: RegIdx,
        source_c: RegIdx,
        source_d: RegIdx,
    ) -> Result<(), Self::Error> {
        if self.stack_frame_boundaries.is_empty() {
            return Err(OpError::InvalidStackFrames { op: "stack_push" }.into());
        }
        self.stack.extend([
            self.registers[source_a.index()],
            self.registers[source_b.index()],
            self.registers[source_c.index()],
            self.registers[source_d.index()],
        ]);
        Ok(())
    }

    #[inline]
    fn stack_get(&mut self, dest: RegIdx, index: StackIdx) -> Result<(), Self::Error> {
        let stack_frame_start = self.stack_frame_boundaries.last().copied().ok_or_else(|| {
            OpError::InvalidStackFrames {
                op: "stack_get_const",
            }
        })?;

        self.registers[dest.index()] = self
            .stack
            .sub_slice(stack_frame_start)
            .get(index.index())
            .copied()
            .unwrap_or_default();
        Ok(())
    }

    #[inline]
    fn get_index_multi(&mut self, dest: RegIdx, array: RegIdx) -> Result<(), Self::Error> {
        let stack_frame_start =
            self.stack_frame_boundaries
                .pop_back()
                .ok_or_else(|| OpError::InvalidStackFrames {
                    op: "get_index_multi",
                })?;

        let res = self.do_get_index(
            self.registers[array.index()],
            &self.stack[stack_frame_start..],
        );
        self.stack.truncate(stack_frame_start);

        self.registers[dest.index()] = res?;
        Ok(())
    }

    #[inline]
    fn set_index_multi(&mut self, array: RegIdx, value: RegIdx) -> Result<(), Self::Error> {
        let stack_frame_start =
            self.stack_frame_boundaries
                .pop_back()
                .ok_or_else(|| OpError::InvalidStackFrames {
                    op: "set_index_multi",
                })?;

        let res = self.do_set_index(
            self.registers[array.index()],
            &self.stack[stack_frame_start..],
            self.registers[value.index()],
        );

        self.stack.truncate(stack_frame_start);
        Ok(res?)
    }

    #[inline]
    fn get_magic(&mut self, dest: RegIdx, magic: MagicIdx) -> Result<(), Self::Error> {
        let magic = self
            .closure
            .prototype()
            .magic()
            .get(magic.index())
            .expect("magic idx is not valid");
        self.registers[dest.index()] = magic.get(self.ctx)?;
        Ok(())
    }

    #[inline]
    fn set_magic(&mut self, magic: MagicIdx, source: RegIdx) -> Result<(), Self::Error> {
        let magic = self
            .closure
            .prototype()
            .magic()
            .get(magic.index())
            .expect("magic idx is not valid");
        magic.set(self.ctx, self.registers[source.index()])?;
        Ok(())
    }

    #[inline]
    fn throw(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        Err(ScriptError::new(self.registers[source.index()]).into())
    }

    #[inline]
    fn jump_if(&mut self, test: RegIdx, is_true: bool) -> Result<bool, Self::Error> {
        Ok(self.registers[test.index()].cast_bool() == is_true)
    }

    #[inline]
    fn jump_if_undefined(&mut self, test: RegIdx, is_undefined: bool) -> Result<bool, Self::Error> {
        Ok(self.registers[test.index()].is_undefined() == is_undefined)
    }

    #[inline]
    fn jump_if_equal(&mut self, left: RegIdx, right: RegIdx) -> Result<bool, Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        Ok(left.equal(right))
    }

    #[inline]
    fn jump_if_not_equal(&mut self, left: RegIdx, right: RegIdx) -> Result<bool, Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        Ok(!left.equal(right))
    }

    #[inline]
    fn jump_if_less(&mut self, left: RegIdx, right: RegIdx) -> Result<bool, Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        Ok(left.less_than(right).ok_or_else(|| OpError::BadBinOp {
            op: "is_less",
            left: left.into(),
            right: right.into(),
        })?)
    }

    #[inline]
    fn jump_if_less_equal(&mut self, left: RegIdx, right: RegIdx) -> Result<bool, Self::Error> {
        let left = self.registers[left.index()];
        let right = self.registers[right.index()];
        Ok(left.less_equal(right).ok_or_else(|| OpError::BadBinOp {
            op: "is_less",
            left: left.into(),
            right: right.into(),
        })?)
    }

    #[inline]
    fn call(
        &mut self,
        func: RegIdx,
        this: Option<RegIdx>,
    ) -> Result<ControlFlow<Self::Break>, Self::Error> {
        let func = self.registers[func.index()];
        let func = func.as_function().ok_or_else(|| OpError::BadCall {
            target: func.type_name(),
        })?;

        let stack_frame_start = self
            .stack_frame_boundaries
            .last()
            .copied()
            .ok_or_else(|| OpError::InvalidStackFrames { op: "call" })?;

        let this = this.map(|r| self.registers[r.index()]).unwrap_or_default();

        Ok(ControlFlow::Break(Next::Call {
            function: func,
            args_bottom: stack_frame_start,
            this,
        }))
    }

    #[inline]
    fn return_(&mut self) -> Result<ControlFlow<Self::Break>, Self::Error> {
        let stack_frame_start = self
            .stack_frame_boundaries
            .pop_back()
            .ok_or_else(|| OpError::InvalidStackFrames { op: "return" })?;

        Ok(ControlFlow::Break(Next::Return {
            returns_bottom: stack_frame_start,
        }))
    }
}
