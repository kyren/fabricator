use std::{
    iter,
    ops::{self, Bound, Index, IndexMut, RangeBounds},
    slice::{self, SliceIndex},
    vec,
};

use crate::{
    conversion::{FromMultiValue, FromValue, IntoMultiValue, TypeError},
    interpreter::Context,
    value::Value,
};

#[derive(Debug)]
pub struct Stack<'gc, 'a> {
    values: &'a mut Vec<Value<'gc>>,
    bottom: usize,
}

impl<'gc, 'a> Stack<'gc, 'a> {
    #[inline]
    pub fn new(values: &'a mut Vec<Value<'gc>>, bottom: usize) -> Self {
        assert!(
            values.len() >= bottom,
            "stack bottom {bottom} is greater than stack len {}",
            values.len()
        );
        Self { values, bottom }
    }

    #[inline]
    pub fn reborrow(&mut self) -> Stack<'gc, '_> {
        self.sub_stack(0)
    }

    #[inline]
    pub fn sub_stack(&mut self, bottom: usize) -> Stack<'gc, '_> {
        assert!(
            self.values.len() - self.bottom >= bottom,
            "sub-stack bottom {bottom} is greater than stack len {}",
            self.values.len() - self.bottom,
        );
        Stack {
            values: self.values,
            bottom: self.bottom + bottom,
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> Value<'gc> {
        self.values
            .get(self.bottom + i)
            .copied()
            .unwrap_or_default()
    }

    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }

    #[inline]
    pub fn push_back(&mut self, value: impl Into<Value<'gc>>) {
        self.values.push(value.into());
    }

    #[inline]
    pub fn pop_back(&mut self) -> Option<Value<'gc>> {
        if self.values.len() > self.bottom {
            Some(self.values.pop().unwrap())
        } else {
            None
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.values.truncate(self.bottom);
    }

    #[inline]
    pub fn resize(&mut self, size: usize) {
        self.values.resize(self.bottom + size, Value::Undefined);
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.values.reserve(additional);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.values.capacity() - self.bottom
    }

    #[inline]
    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> vec::Drain<'_, Value<'gc>> {
        let start = match range.start_bound().cloned() {
            Bound::Included(r) => Bound::Included(self.bottom + r),
            Bound::Excluded(r) => Bound::Excluded(self.bottom + r),
            Bound::Unbounded => Bound::Included(self.bottom),
        };
        let end = match range.end_bound().cloned() {
            Bound::Included(r) => Bound::Included(self.bottom + r),
            Bound::Excluded(r) => Bound::Excluded(self.bottom + r),
            Bound::Unbounded => Bound::Unbounded,
        };
        self.values.drain((start, end))
    }

    #[inline]
    pub fn from_index<V: FromValue<'gc>>(
        &self,
        ctx: Context<'gc>,
        i: usize,
    ) -> Result<V, TypeError> {
        V::from_value(ctx, self.get(i))
    }

    #[inline]
    pub fn consume<V: FromMultiValue<'gc>>(&mut self, ctx: Context<'gc>) -> Result<V, TypeError> {
        V::from_multi_value(ctx, self.drain(..))
    }

    #[inline]
    pub fn replace(&mut self, ctx: Context<'gc>, v: impl IntoMultiValue<'gc>) {
        self.clear();
        self.extend(v.into_multi_value(ctx));
    }
}

impl<'gc, 'a> ops::Deref for Stack<'gc, 'a> {
    type Target = [Value<'gc>];

    fn deref(&self) -> &Self::Target {
        &self.values[self.bottom..]
    }
}

impl<'gc, 'a> ops::DerefMut for Stack<'gc, 'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values[self.bottom..]
    }
}

impl<'gc: 'b, 'a, 'b> IntoIterator for &'b Stack<'gc, 'a> {
    type Item = Value<'gc>;
    type IntoIter = iter::Copied<slice::Iter<'b, Value<'gc>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.values[self.bottom..].iter().copied()
    }
}

impl<'gc, 'a> Extend<Value<'gc>> for Stack<'gc, 'a> {
    #[inline]
    fn extend<I: IntoIterator<Item = Value<'gc>>>(&mut self, iter: I) {
        self.values.extend(iter);
    }
}

impl<'gc, 'a, 'b> Extend<Value<'gc>> for &'b mut Stack<'gc, 'a> {
    #[inline]
    fn extend<I: IntoIterator<Item = Value<'gc>>>(&mut self, iter: I) {
        self.values.extend(iter);
    }
}

impl<'gc, 'a> Extend<&'a Value<'gc>> for Stack<'gc, 'a> {
    #[inline]
    fn extend<I: IntoIterator<Item = &'a Value<'gc>>>(&mut self, iter: I) {
        self.values.extend(iter);
    }
}

impl<'gc: 'b, 'a, 'b, 'c> Extend<&'b Value<'gc>> for &'c mut Stack<'gc, 'a> {
    #[inline]
    fn extend<I: IntoIterator<Item = &'b Value<'gc>>>(&mut self, iter: I) {
        self.values.extend(iter);
    }
}

impl<'gc, 'a, I: SliceIndex<[Value<'gc>]>> Index<I> for Stack<'gc, 'a> {
    type Output = <Vec<Value<'gc>> as Index<I>>::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        &self.values[self.bottom..][index]
    }
}

impl<'gc, 'a, I: SliceIndex<[Value<'gc>]>> IndexMut<I> for Stack<'gc, 'a> {
    #[inline]
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        &mut self.values[self.bottom..][index]
    }
}
