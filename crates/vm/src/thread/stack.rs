use std::{
    iter,
    ops::{self, RangeBounds},
    slice, vec,
};

use crate::{
    conversion::{FromMultiValue, FromValue, IntoMultiValue},
    error::RuntimeError,
    interpreter::Context,
    thread::vec_end_slice::VecEndSlice,
    value::Value,
};

/// A reference to the top of the current thread's stack.
#[derive(Debug)]
pub struct Stack<'gc, 'a> {
    slice: VecEndSlice<'a, Value<'gc>>,
}

impl<'gc, 'a> Stack<'gc, 'a> {
    #[track_caller]
    #[inline]
    pub fn new(values: &'a mut Vec<Value<'gc>>, bottom: usize) -> Self {
        Stack {
            slice: VecEndSlice::new(values, bottom),
        }
    }

    #[inline]
    pub fn reborrow(&mut self) -> Stack<'gc, '_> {
        Stack {
            slice: self.slice.reborrow(),
        }
    }

    #[track_caller]
    #[inline]
    pub fn sub_stack(&mut self, bottom: usize) -> Stack<'gc, '_> {
        Stack {
            slice: self.slice.sub_slice(bottom),
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> Value<'gc> {
        self.slice.get(i).copied().unwrap_or_default()
    }

    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }

    #[inline]
    pub fn push(&mut self, value: impl Into<Value<'gc>>) {
        self.slice.push(value.into());
    }

    #[inline]
    pub fn pop(&mut self) -> Value<'gc> {
        self.slice.pop().unwrap_or_default()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.slice.clear();
    }

    #[track_caller]
    #[inline]
    pub fn resize(&mut self, size: usize) {
        self.slice.resize(size, Value::Undefined);
    }

    #[inline]
    pub fn truncate(&mut self, size: usize) {
        self.slice.truncate(size);
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.slice.reserve(additional);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.slice.capacity()
    }

    #[track_caller]
    #[inline]
    pub fn remove(&mut self, index: usize) -> Value<'gc> {
        self.slice.remove(index)
    }

    #[inline]
    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> vec::Drain<'_, Value<'gc>> {
        self.slice.drain(range)
    }

    #[inline]
    pub fn from_index<V: FromValue<'gc>>(
        &self,
        ctx: Context<'gc>,
        i: usize,
    ) -> Result<V, RuntimeError> {
        V::from_value(ctx, self.get(i))
    }

    /// Drain the entire stack, converting all stack values into `V` which must implement
    /// [`FromMultiValue`].
    #[inline]
    pub fn consume<V: FromMultiValue<'gc>>(
        &mut self,
        ctx: Context<'gc>,
    ) -> Result<V, RuntimeError> {
        V::from_multi_value(ctx, self.drain(..))
    }

    /// Replace the entire stack with the given `v` which must implement [`IntoMultiValue`].
    #[inline]
    pub fn replace(&mut self, ctx: Context<'gc>, v: impl IntoMultiValue<'gc>) {
        self.clear();
        self.extend(v.into_multi_value(ctx));
    }
}

impl<'gc, 'a> ops::Deref for Stack<'gc, 'a> {
    type Target = [Value<'gc>];

    fn deref(&self) -> &Self::Target {
        &self.slice[..]
    }
}

impl<'gc, 'a> ops::DerefMut for Stack<'gc, 'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slice[..]
    }
}

impl<'gc: 'b, 'a, 'b> IntoIterator for &'b Stack<'gc, 'a> {
    type Item = Value<'gc>;
    type IntoIter = iter::Copied<slice::Iter<'b, Value<'gc>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.slice[..].iter().copied()
    }
}

impl<'gc, 'a> Extend<Value<'gc>> for Stack<'gc, 'a> {
    #[inline]
    fn extend<I: IntoIterator<Item = Value<'gc>>>(&mut self, iter: I) {
        self.slice.extend(iter);
    }
}
