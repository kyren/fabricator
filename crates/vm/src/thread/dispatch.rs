use std::{mem, ops::ControlFlow};

use gc_arena::Gc;
use thiserror::Error;

use crate::{
    array::Array,
    closure::{Closure, Constant, HeapVar, HeapVarDescriptor},
    error::{Error, ExternValue, RuntimeError, ScriptError},
    instructions::{self, ConstIdx, HeapIdx, MagicIdx, ProtoIdx, RegIdx},
    interpreter::Context,
    object::Object,
    stack::Stack,
    string::String,
    thread::thread::OwnedHeapVar,
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
    #[error("bad array {array:?}")]
    BadArray { array: ExternValue },
    #[error("bad index {index:?} of {target:?}")]
    BadIndex {
        target: ExternValue,
        index: ExternValue,
    },
    #[error("{target:?} does not allow multi-indexing")]
    BadMultiIndex { target: ExternValue },
    #[error("no such field {field:?} in {target:?}")]
    NoSuchField {
        target: ExternValue,
        field: ExternValue,
    },
    #[error("bad call of {target:?}")]
    BadCall { target: &'static str },
    #[error("bad stack index {index}")]
    BadStackIdx { index: ExternValue },
    #[error("bad stack index {index} and offset {offset}")]
    BadStackIdxOffset {
        index: ExternValue,
        offset: ExternValue,
    },
}

#[derive(Debug, Error)]
#[error("out of bounds access on array for index {0}")]
pub struct ArrayBoundsError(usize);

pub(super) enum Next<'gc> {
    Call {
        function: Function<'gc>,
        args_bottom: usize,
    },
    Return {
        returns_bottom: usize,
    },
}

pub(super) struct Dispatch<'gc, 'a> {
    ctx: Context<'gc>,
    closure: Closure<'gc>,
    this: &'a mut Value<'gc>,
    other: &'a mut Value<'gc>,
    // The register slice is fixed size to avoid bounds checks.
    registers: &'a mut [Value<'gc>; 256],
    heap: &'a mut [OwnedHeapVar<'gc>],
    stack: Stack<'gc, 'a>,
}

impl<'gc, 'a> Dispatch<'gc, 'a> {
    pub(super) fn new(
        ctx: Context<'gc>,
        closure: Closure<'gc>,
        this: &'a mut Value<'gc>,
        other: &'a mut Value<'gc>,
        registers: &'a mut [Value<'gc>; 256],
        heap: &'a mut [OwnedHeapVar<'gc>],
        stack: Stack<'gc, 'a>,
    ) -> Self {
        Self {
            ctx,
            closure,
            this,
            other,
            registers,
            heap,
            stack,
        }
    }
}

impl<'gc, 'a> Dispatch<'gc, 'a> {
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
                    return Err(OpError::BadMultiIndex {
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
                    return Err(OpError::BadMultiIndex {
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
            _ => Err(OpError::BadArray {
                array: target.into(),
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
                    return Err(OpError::BadMultiIndex {
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
                    return Err(OpError::BadMultiIndex {
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
            _ => Err(OpError::BadArray {
                array: target.into(),
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
        self.registers[dest as usize] = Value::Undefined;
        Ok(())
    }

    #[inline]
    fn boolean(&mut self, dest: RegIdx, value: bool) -> Result<(), Self::Error> {
        self.registers[dest as usize] = Value::Boolean(value);
        Ok(())
    }

    #[inline]
    fn load_constant(&mut self, dest: RegIdx, constant: ConstIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] =
            self.closure.prototype().constants()[constant as usize].to_value();
        Ok(())
    }

    #[inline]
    fn get_heap(&mut self, dest: RegIdx, heap: HeapIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = match self.closure.heap()[heap as usize] {
            HeapVar::Owned(idx) => self.heap[idx as usize].get(),
            HeapVar::Shared(v) => v.get(),
        };
        Ok(())
    }

    #[inline]
    fn set_heap(&mut self, heap: HeapIdx, source: RegIdx) -> Result<(), Self::Error> {
        let source = self.registers[source as usize];
        match self.closure.heap()[heap as usize] {
            HeapVar::Owned(idx) => self.heap[idx as usize].set(&self.ctx, source),
            HeapVar::Shared(v) => v.set(&self.ctx, source),
        };
        Ok(())
    }

    #[inline]
    fn reset_heap(&mut self, heap: HeapIdx) -> Result<(), Self::Error> {
        match self.closure.heap()[heap as usize] {
            HeapVar::Owned(idx) => {
                self.heap[idx as usize] = OwnedHeapVar::unique(Value::Undefined);
                Ok(())
            }
            HeapVar::Shared(_) => panic!("reset of shared heap var"),
        }
    }

    #[inline]
    fn globals(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = self.ctx.globals().into();
        Ok(())
    }

    #[inline]
    fn this(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = *self.this;
        Ok(())
    }

    #[inline]
    fn set_this(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        *self.this = self.registers[source as usize];
        Ok(())
    }

    #[inline]
    fn other(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = *self.other;
        Ok(())
    }

    #[inline]
    fn set_other(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        *self.other = self.registers[source as usize];
        Ok(())
    }

    #[inline]
    fn swap_this_other(&mut self) -> Result<(), Self::Error> {
        mem::swap(self.this, self.other);
        Ok(())
    }

    #[inline]
    fn closure(
        &mut self,
        dest: RegIdx,
        proto: ProtoIdx,
        bind_this: bool,
    ) -> Result<(), Self::Error> {
        let proto = self.closure.prototype().prototypes()[proto as usize];

        let mut heap = Vec::new();
        for &hd in proto.heap_vars() {
            match hd {
                HeapVarDescriptor::Owned(idx) => {
                    heap.push(HeapVar::Owned(idx));
                }
                HeapVarDescriptor::Static(idx) => {
                    heap.push(HeapVar::Shared(proto.static_vars()[idx as usize]))
                }
                HeapVarDescriptor::UpValue(idx) => {
                    heap.push(HeapVar::Shared(match self.closure.heap()[idx as usize] {
                        HeapVar::Owned(idx) => self.heap[idx as usize].make_shared(&self.ctx),
                        HeapVar::Shared(v) => v,
                    }));
                }
            }
        }

        // Inner closures bind the current `this` value if the `bind_this` flag is set, otherwise
        // they are created unbound.
        self.registers[dest as usize] = Closure::from_parts(
            &self.ctx,
            proto,
            if bind_this {
                *self.this
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
        self.registers[dest as usize] = self.closure.into();
        Ok(())
    }

    #[inline]
    fn new_object(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = Object::new(&self.ctx).into();
        Ok(())
    }

    #[inline]
    fn new_array(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = Array::new(&self.ctx).into();
        Ok(())
    }

    #[inline]
    fn get_field(&mut self, dest: RegIdx, object: RegIdx, key: RegIdx) -> Result<(), Self::Error> {
        let key_val = self.registers[key as usize];
        let Some(key) = key_val.as_string() else {
            return Err(OpError::BadKey {
                key: key_val.into(),
            }
            .into());
        };

        self.registers[dest as usize] = self.do_get_field(self.registers[object as usize], key)?;
        Ok(())
    }

    #[inline]
    fn set_field(&mut self, object: RegIdx, key: RegIdx, value: RegIdx) -> Result<(), Self::Error> {
        let key_val = self.registers[key as usize];
        let Some(key) = key_val.as_string() else {
            return Err(OpError::BadKey {
                key: key_val.into(),
            }
            .into());
        };
        Ok(self.do_set_field(
            self.registers[object as usize],
            key,
            self.registers[value as usize],
        )?)
    }

    #[inline]
    fn get_field_const(
        &mut self,
        dest: RegIdx,
        object: RegIdx,
        key: ConstIdx,
    ) -> Result<(), Self::Error> {
        let Constant::String(key) = self.closure.prototype().constants()[key as usize] else {
            panic!("const key is not a string");
        };
        self.registers[dest as usize] = self.do_get_field(self.registers[object as usize], key)?;
        Ok(())
    }

    #[inline]
    fn set_field_const(
        &mut self,
        object: RegIdx,
        key: ConstIdx,
        value: RegIdx,
    ) -> Result<(), Self::Error> {
        let Constant::String(key) = self.closure.prototype().constants()[key as usize] else {
            panic!("const key is not a string");
        };
        Ok(self.do_set_field(
            self.registers[object as usize],
            key,
            self.registers[value as usize],
        )?)
    }

    #[inline]
    fn get_index(&mut self, dest: RegIdx, array: RegIdx, index: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = self.do_get_index(
            self.registers[array as usize],
            &[self.registers[index as usize]],
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
            self.registers[array as usize],
            &[self.registers[index as usize]],
            self.registers[value as usize],
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
        self.registers[dest as usize] = self.do_get_index(
            self.registers[array as usize],
            &[self.closure.prototype().constants()[index as usize].to_value()],
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
            self.registers[array as usize],
            &[self.closure.prototype().constants()[index as usize].to_value()],
            self.registers[value as usize],
        )?;
        Ok(())
    }

    #[inline]
    fn copy(&mut self, dest: RegIdx, source: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = self.registers[source as usize];
        Ok(())
    }

    #[inline]
    fn is_defined(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = (!self.registers[arg as usize].is_undefined()).into();
        Ok(())
    }

    #[inline]
    fn is_undefined(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = self.registers[arg as usize].is_undefined().into();
        Ok(())
    }

    #[inline]
    fn test(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = self.registers[arg as usize].cast_bool().into();
        Ok(())
    }

    #[inline]
    fn not(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = (!self.registers[arg as usize].cast_bool()).into();
        Ok(())
    }

    #[inline]
    fn negate(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg as usize];
        self.registers[dest as usize] = arg.negate().ok_or_else(|| OpError::BadUnOp {
            op: "neg",
            arg: arg.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn bit_negate(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg as usize];
        self.registers[dest as usize] = arg
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
        let arg = self.registers[arg as usize];
        self.registers[dest as usize] =
            arg.add(Value::Integer(1)).ok_or_else(|| OpError::BadUnOp {
                op: "inc",
                arg: arg.into(),
            })?;
        Ok(())
    }

    #[inline]
    fn decrement(&mut self, dest: RegIdx, arg: RegIdx) -> Result<(), Self::Error> {
        let arg = self.registers[arg as usize];
        self.registers[dest as usize] =
            arg.sub(Value::Integer(1)).ok_or_else(|| OpError::BadUnOp {
                op: "dec",
                arg: arg.into(),
            })?;
        Ok(())
    }

    #[inline]
    fn add(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.sub(right).ok_or_else(|| OpError::BadBinOp {
            op: "sub",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn multiply(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.mult(right).ok_or_else(|| OpError::BadBinOp {
            op: "mult",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn divide(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.div(right).ok_or_else(|| OpError::BadBinOp {
            op: "div",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn remainder(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.rem(right).ok_or_else(|| OpError::BadBinOp {
            op: "rem",
            left: left.into(),
            right: right.into(),
        })?;
        Ok(())
    }

    #[inline]
    fn int_divide(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = (!left.equal(right)).into();
        Ok(())
    }

    #[inline]
    fn is_less(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        self.registers[dest as usize] = left
            .less_than(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "is_less / is_greater",
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        self.registers[dest as usize] = left
            .less_equal(right)
            .ok_or_else(|| OpError::BadBinOp {
                op: "is_less_eq / is_greater_eq",
                left: left.into(),
                right: right.into(),
            })?
            .into();
        Ok(())
    }

    #[inline]
    fn and(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.and(right).into();
        Ok(())
    }

    #[inline]
    fn or(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.or(right).into();
        Ok(())
    }

    #[inline]
    fn xor(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.xor(right).into();
        Ok(())
    }

    #[inline]
    fn bit_and(&mut self, dest: RegIdx, left: RegIdx, right: RegIdx) -> Result<(), Self::Error> {
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
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
        let left = self.registers[left as usize];
        let right = self.registers[right as usize];
        let dest = &mut self.registers[dest as usize];
        *dest = left.null_coalesce(right);
        Ok(())
    }

    #[inline]
    fn jump_if(&mut self, test: RegIdx, is_true: bool) -> Result<bool, Self::Error> {
        Ok(self.registers[test as usize].cast_bool() == is_true)
    }

    #[inline]
    fn stack_top(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = (self.stack.len() as i64).into();
        Ok(())
    }

    #[inline]
    fn stack_resize(&mut self, stack_top: RegIdx) -> Result<(), Self::Error> {
        let stack_top = self.registers[stack_top as usize];
        let stack_top = stack_top
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_top.into(),
            })?;
        self.stack.resize(stack_top);
        Ok(())
    }

    #[inline]
    fn stack_resize_const(&mut self, stack_top: ConstIdx) -> Result<(), Self::Error> {
        let stack_top = self.closure.prototype().constants()[stack_top as usize];
        let stack_top = stack_top
            .to_value()
            .as_integer()
            .expect("const index is not integer")
            .try_into()
            .map_err(|_| OpError::BadStackIdx {
                index: stack_top.to_value().into(),
            })?;
        self.stack.resize(stack_top);
        Ok(())
    }

    #[inline]
    fn stack_get(&mut self, dest: RegIdx, stack_pos: RegIdx) -> Result<(), Self::Error> {
        let stack_idx = self.registers[stack_pos as usize];
        let stack_idx = stack_idx
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_idx.into(),
            })?;
        // We return `Undefined` if the `stack_idx` is out of range.
        self.registers[dest as usize] = self.stack.get(stack_idx);
        Ok(())
    }

    #[inline]
    fn stack_get_const(&mut self, dest: RegIdx, stack_pos: ConstIdx) -> Result<(), Self::Error> {
        let stack_idx = self.closure.prototype().constants()[stack_pos as usize];
        let stack_idx = stack_idx
            .to_value()
            .as_integer()
            .expect("const index is not integer")
            .try_into()
            .map_err(|_| OpError::BadStackIdx {
                index: stack_idx.to_value().into(),
            })?;
        // We return `Undefined` if the `stack_idx` is out of range.
        self.registers[dest as usize] = self.stack.get(stack_idx);
        Ok(())
    }

    #[inline]
    fn stack_get_offset(
        &mut self,
        dest: RegIdx,
        stack_base: RegIdx,
        offset: ConstIdx,
    ) -> Result<(), Self::Error> {
        let offset = self.closure.prototype().constants()[offset as usize]
            .to_value()
            .as_integer()
            .expect("const index is not integer");
        let stack_idx = self.registers[stack_base as usize];
        let stack_idx = stack_idx
            .as_integer()
            .and_then(|i| i.checked_add(offset))
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdxOffset {
                index: stack_idx.into(),
                offset: Value::Integer(offset).into(),
            })?;
        // We return `Undefined` if the `stack_idx` is out of range.
        self.registers[dest as usize] = self.stack.get(stack_idx);
        Ok(())
    }

    #[inline]
    fn stack_set(&mut self, source: RegIdx, stack_pos: RegIdx) -> Result<(), Self::Error> {
        let stack_idx = self.registers[stack_pos as usize];
        let stack_idx: usize = stack_idx
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_idx.into(),
            })?;
        // We need to implicitly grow the stack if the register is out of range.
        if stack_idx >= self.stack.len() {
            self.stack.resize(stack_idx + 1);
        }
        self.stack[stack_idx] = self.registers[source as usize];
        Ok(())
    }

    #[inline]
    fn stack_push(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        self.stack.push_back(self.registers[source as usize]);
        Ok(())
    }

    #[inline]
    fn stack_pop(&mut self, dest: RegIdx) -> Result<(), Self::Error> {
        self.registers[dest as usize] = self.stack.pop_back().unwrap_or_default();
        Ok(())
    }

    #[inline]
    fn get_index_multi(
        &mut self,
        dest: RegIdx,
        array: RegIdx,
        stack_bottom: RegIdx,
    ) -> Result<(), Self::Error> {
        let stack_bottom = self.registers[stack_bottom as usize];
        let mut stack_bottom: usize = stack_bottom
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_bottom.into(),
            })?;
        stack_bottom = stack_bottom.min(self.stack.len());

        self.registers[dest as usize] =
            self.do_get_index(self.registers[array as usize], &self.stack[stack_bottom..])?;

        self.stack.resize(stack_bottom);
        Ok(())
    }

    #[inline]
    fn set_index_multi(
        &mut self,
        array: RegIdx,
        stack_bottom: RegIdx,
        value: RegIdx,
    ) -> Result<(), Self::Error> {
        let stack_bottom = self.registers[stack_bottom as usize];
        let mut stack_bottom: usize = stack_bottom
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_bottom.into(),
            })?;
        stack_bottom = stack_bottom.min(self.stack.len());

        self.do_set_index(
            self.registers[array as usize],
            &self.stack[stack_bottom..],
            self.registers[value as usize],
        )?;

        self.stack.resize(stack_bottom);
        Ok(())
    }

    #[inline]
    fn get_magic(&mut self, dest: RegIdx, magic: MagicIdx) -> Result<(), Self::Error> {
        let magic = self
            .closure
            .prototype()
            .magic()
            .get(magic as usize)
            .expect("magic idx is not valid");
        self.registers[dest as usize] = magic.get(self.ctx)?;
        Ok(())
    }

    #[inline]
    fn set_magic(&mut self, magic: MagicIdx, source: RegIdx) -> Result<(), Self::Error> {
        let magic = self
            .closure
            .prototype()
            .magic()
            .get(magic as usize)
            .expect("magic idx is not valid");
        magic.set(self.ctx, self.registers[source as usize])?;
        Ok(())
    }

    #[inline]
    fn throw(&mut self, source: RegIdx) -> Result<(), Self::Error> {
        Err(ScriptError::new(self.registers[source as usize]).into())
    }

    #[inline]
    fn call(
        &mut self,
        func: RegIdx,
        stack_bottom: RegIdx,
    ) -> Result<ControlFlow<Self::Break>, Self::Error> {
        let func = self.registers[func as usize];
        let func = func.as_function().ok_or_else(|| OpError::BadCall {
            target: func.type_name(),
        })?;

        let stack_bottom = self.registers[stack_bottom as usize];
        let stack_bottom: usize = stack_bottom
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_bottom.into(),
            })?;
        if stack_bottom > self.stack.len() {
            self.stack.resize(stack_bottom);
        }

        Ok(ControlFlow::Break(Next::Call {
            function: func,
            args_bottom: stack_bottom,
        }))
    }

    #[inline]
    fn return_(&mut self, stack_bottom: RegIdx) -> Result<Self::Break, Self::Error> {
        let stack_bottom = self.registers[stack_bottom as usize];
        let stack_bottom: usize = stack_bottom
            .as_integer()
            .and_then(|i| i.try_into().ok())
            .ok_or_else(|| OpError::BadStackIdx {
                index: stack_bottom.into(),
            })?;
        let stack_bottom = stack_bottom.min(self.stack.len());

        Ok(Next::Return {
            returns_bottom: stack_bottom,
        })
    }
}
