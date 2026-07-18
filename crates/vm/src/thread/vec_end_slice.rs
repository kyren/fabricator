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
    #[track_caller]
    #[inline]
    pub fn new(values: &'a mut Vec<T>, bottom: usize) -> Self {
        assert!(
            bottom <= values.len(),
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

    #[track_caller]
    #[inline]
    pub fn sub_slice(&mut self, bottom: usize) -> VecEndSlice<'_, T> {
        let len = self.inner.len() - self.bottom;
        assert!(
            bottom <= len,
            "sub-slice bottom {bottom} is greater than slice len {len}"
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

    #[track_caller]
    #[inline]
    pub fn resize(&mut self, size: usize, value: T)
    where
        T: Clone,
    {
        self.inner.resize(
            self.bottom
                .checked_add(size)
                .expect("size overflow in `VecEndSlice::resize`"),
            value,
        );
    }

    #[inline]
    pub fn truncate(&mut self, size: usize) {
        self.inner.truncate(self.bottom.saturating_add(size));
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity() - self.bottom
    }

    #[track_caller]
    #[inline]
    pub fn remove(&mut self, index: usize) -> T {
        self.inner.remove(
            self.bottom
                .checked_add(index)
                .expect("size overflow in `VecEndSlice::remove`"),
        )
    }

    #[inline]
    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> vec::Drain<'_, T> {
        let start = match range.start_bound() {
            Bound::Unbounded => Bound::Included(self.bottom),
            bound => bound.map(|&r| self.bottom.saturating_add(r)),
        };
        let end = range.end_bound().map(|&r| self.bottom.saturating_add(r));
        self.inner.drain((start, end))
    }

    #[track_caller]
    #[inline]
    pub fn extend_from_within<R: RangeBounds<usize>>(&mut self, range: R)
    where
        T: Clone,
    {
        const EXPECT_MSG: &str = "size overflow in `VecEndSlice::extend_from_within`";

        let start = match range.start_bound() {
            Bound::Unbounded => Bound::Included(self.bottom),
            bound => bound.map(|&r| self.bottom.checked_add(r).expect(EXPECT_MSG)),
        };
        let end = range
            .end_bound()
            .map(|&r| self.bottom.checked_add(r).expect(EXPECT_MSG));
        self.inner.extend_from_within((start, end));
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
