use std::{
    ops::{self, Bound, RangeBounds},
    vec,
};

/// A mutable reference to only the *end* of some `Vec<T>`.
///
/// Preserves all values below the `bottom` of the slice. Users are allowed to grow and shrink the
/// end of the `Vec` as long as all values below `bottom` are preserved.
///
/// This can be used to provide a series of nested stacks while avoiding a separate allocation for
/// each stack.
#[derive(Debug)]
pub struct VecEndSlice<'a, T> {
    inner: &'a mut Vec<T>,
    bottom: usize,
}

impl<'a, T> VecEndSlice<'a, T> {
    #[inline]
    pub fn new(values: &'a mut Vec<T>, bottom: usize) -> Self {
        assert!(
            values.len() >= bottom,
            "slice bottom {bottom} is greater than vec len {}",
            values.len()
        );
        Self {
            inner: values,
            bottom,
        }
    }

    /// Return an immutable slice of the values *below* the current bottom.
    #[inline]
    pub fn below(&self) -> &[T] {
        &self.inner[0..self.bottom]
    }

    #[inline]
    pub fn reborrow(&mut self) -> VecEndSlice<'_, T> {
        self.sub_slice(0)
    }

    #[inline]
    pub fn sub_slice(&mut self, bottom: usize) -> VecEndSlice<'_, T> {
        assert!(
            self.inner.len() - self.bottom >= bottom,
            "sub-slice bottom {bottom} is greater than slice len {}",
            self.inner.len() - self.bottom,
        );
        VecEndSlice {
            inner: self.inner,
            bottom: self.bottom + bottom,
        }
    }

    #[inline]
    pub fn push_back(&mut self, value: T) {
        self.inner.push(value);
    }

    #[inline]
    pub fn pop_back(&mut self) -> Option<T> {
        if self.inner.len() > self.bottom {
            Some(self.inner.pop().unwrap())
        } else {
            None
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.inner.truncate(self.bottom);
    }

    #[inline]
    pub fn resize(&mut self, size: usize, value: T)
    where
        T: Clone,
    {
        self.inner.resize(self.bottom + size, value);
    }

    #[inline]
    pub fn truncate(&mut self, size: usize) {
        self.inner.truncate(self.bottom + size);
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity() - self.bottom
    }

    #[inline]
    pub fn remove(&mut self, index: usize) -> T {
        self.inner.remove(self.bottom + index)
    }

    #[inline]
    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> vec::Drain<'_, T> {
        let (start, end) = self.inner_range(range);
        self.inner.drain((start, end))
    }

    #[inline]
    pub fn extend_from_within<R: RangeBounds<usize>>(&mut self, range: R)
    where
        T: Clone,
    {
        let (start, end) = self.inner_range(range);
        self.inner.extend_from_within((start, end));
    }

    fn inner_range<R: RangeBounds<usize>>(&self, range: R) -> (Bound<usize>, Bound<usize>) {
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

        (start, end)
    }
}

impl<'a, T> ops::Deref for VecEndSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.inner[self.bottom..]
    }
}

impl<'a, T> ops::DerefMut for VecEndSlice<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner[self.bottom..]
    }
}

impl<'a, T> Extend<T> for VecEndSlice<'a, T> {
    #[inline]
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}

impl<'a, T: Copy> Extend<&'a T> for VecEndSlice<'a, T> {
    #[inline]
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}
