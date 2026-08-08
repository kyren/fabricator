use std::{cell::RefCell, marker::PhantomData, mem};

use thiserror::Error;

pub trait Freeze<'f> {
    type Frozen: 'f;
}

pub struct DynFreeze<T: ?Sized>(PhantomData<T>);

impl<'f, T: ?Sized + for<'a> Freeze<'a>> Freeze<'f> for DynFreeze<T> {
    type Frozen = <T as Freeze<'f>>::Frozen;
}

#[macro_export]
#[doc(hidden)]
macro_rules! __freeze_Freeze {
    ($f:lifetime => $frozen:ty) => {
        $crate::freeze::DynFreeze::<
            dyn for<$f> $crate::freeze::Freeze<$f, Frozen = $frozen>,
        >
    };
    ($frozen:ty) => {
        $crate::freeze::Freeze!['freeze => $frozen]
    };
}

#[doc(inline)]
pub use crate::__freeze_Freeze as Freeze;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Error)]
pub enum AccessError {
    #[error("frozen value accessed outside of enclosing freeze scope")]
    Expired,
    #[error("already borrowed incompatibly")]
    BadBorrow,
}

/// Safely erase a lifetime from a value and store it in a scoped handle.
///
/// Works by providing only limited access to the held value within an enclosing call to
/// [`FreezeCell::with`] or [`FreezeCell::with_mut`].
pub struct FreezeCell<F: for<'f> Freeze<'f>> {
    inner: FreezeRefCell<F>,
}

impl<F: for<'f> Freeze<'f>> Default for FreezeCell<F> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<F: for<'f> Freeze<'f>> FreezeCell<F> {
    #[inline]
    pub const fn new() -> FreezeCell<F> {
        FreezeCell {
            inner: RefCell::new(None),
        }
    }

    /// Set a value for the duration of the provided closure.
    ///
    /// It is explicitly allowed to call this method recursively from the provided callback, and the
    /// previous set value will be restored once the inner callback finishes.
    ///
    /// It is however NOT permitted to call this method from within [`FreezeCell::with`] or
    /// [`FreezeCell::with_mut`].
    ///
    /// # Panics
    ///
    /// Calling this method from a call to [`FreezeCell::with`] or [`FreezeCell::with_mut`] will
    /// panic.
    #[inline]
    pub fn freeze<'f, R>(&self, v: <F as Freeze<'f>>::Frozen, f: impl FnOnce() -> R) -> R {
        // SAFETY:
        //
        // 1) The real lifetime of the set value lasts at least as long as the body of this
        //    function, and the value is unset before this function returns because the returned
        //    guard is always dropped.
        //
        // 2) We only allow access to the set value via `FreezeCell::with` and
        //    `FreezeCell::with_mut`, both of which require a callback that must work for *any*
        //    lifetime, so they must work with the lifetime we have erased. User code is never able
        //    to observe the false 'static lifetime.

        let _guard = unsafe { freeze_value::<F>(&self.inner, v) };
        f()
    }

    /// Access the stored value.
    #[inline]
    pub fn with<R>(
        &self,
        f: impl for<'f> FnOnce(&<F as Freeze<'f>>::Frozen) -> R,
    ) -> Result<R, AccessError> {
        let val = self
            .inner
            .try_borrow()
            .map_err(|_| AccessError::BadBorrow)?;
        let val = val.as_ref().ok_or(AccessError::Expired)?;
        Ok(f(val))
    }

    /// Access the stored value mutably.
    #[inline]
    pub fn with_mut<R>(
        &self,
        f: impl for<'f> FnOnce(&mut <F as Freeze<'f>>::Frozen) -> R,
    ) -> Result<R, AccessError> {
        let mut val = self
            .inner
            .try_borrow_mut()
            .map_err(|_| AccessError::BadBorrow)?;
        let val = val.as_mut().ok_or(AccessError::Expired)?;
        Ok(f(val))
    }
}

/// A builder type that makes it easier to freeze values inside several [`FreezeCell`]s at once.
///
/// This can be used to avoid the rightward drift that results from making several individual nested
/// calls to [`FreezeCell::freeze`], but is otherwise identical.
pub struct FreezeMany<T = ()>(T);

impl FreezeMany<()> {
    #[inline]
    pub fn new() -> Self {
        FreezeMany(())
    }
}

impl<T> FreezeMany<T> {
    /// Freeze the given value in the provided [`FreezeCell`] during the call to
    /// [`FreezeMany::in_scope`].
    #[inline]
    pub fn freeze<'h, 'f, F: for<'a> Freeze<'a>>(
        self,
        cell: &'h FreezeCell<F>,
        value: <F as Freeze<'f>>::Frozen,
    ) -> FreezeMany<(T, FreezeOne<'h, 'f, F>)> {
        FreezeMany((self.0, FreezeOne { cell, value }))
    }
}

#[allow(private_bounds)]
impl<T: SetFrozen> FreezeMany<T> {
    /// Freeze every provided value for the duration of `closure`.
    ///
    /// Equivalent nested calls to [`FreezeCell::freeze`] for every provided value.
    ///
    /// # Panics
    ///
    /// Similarly to `FreezeCell::freeze`, calling this from within  [`FreezeCell::with`] or
    /// [`FreezeCell::with_mut`] for any of the [`FreezeCell`]s being set will panic.
    #[inline]
    pub fn in_scope<R>(self, f: impl FnOnce() -> R) -> R {
        // SAFETY: See the implementation of `FreezeCell::freeze`.
        let _guard = unsafe { self.0.set() };
        f()
    }
}

pub struct FreezeOne<'h, 'f, F: for<'a> Freeze<'a>> {
    cell: &'h FreezeCell<F>,
    value: <F as Freeze<'f>>::Frozen,
}

impl<'h, 'f, F: for<'a> Freeze<'a>> SetFrozen for FreezeOne<'h, 'f, F> {
    type Guard = FreezeGuard<'h, F>;

    #[inline]
    unsafe fn set(self) -> Self::Guard {
        unsafe { freeze_value(&self.cell.inner, self.value) }
    }
}

impl SetFrozen for () {
    type Guard = ();

    #[inline]
    unsafe fn set(self) {}
}

impl<A: SetFrozen, B: SetFrozen> SetFrozen for (A, B) {
    type Guard = (A::Guard, B::Guard);

    #[inline]
    unsafe fn set(self) -> Self::Guard {
        unsafe { (self.0.set(), self.1.set()) }
    }
}

trait SetFrozen {
    type Guard;

    unsafe fn set(self) -> Self::Guard;
}

type FreezeRefCell<F> = RefCell<Option<<F as Freeze<'static>>::Frozen>>;

/// Set the previous value for a `FreezeRefCell` on drop.
///
/// If replacing the `FreezeRefCell` value fails, then this is assumed to be able to lead to
/// unsafety and the drop impl will call `std::process::abort()`.
struct FreezeGuard<'a, F: for<'f> Freeze<'f>> {
    cell: &'a FreezeRefCell<F>,
    prev: Option<<F as Freeze<'static>>::Frozen>,
}

impl<F: for<'f> Freeze<'f>> Drop for FreezeGuard<'_, F> {
    #[inline]
    fn drop(&mut self) {
        if let Ok(mut cell) = self.cell.try_borrow_mut() {
            *cell = self.prev.take();
        } else {
            // If the value is locked, then there might be a live reference to it somewhere. We can
            // no longer guarantee our invariants and are forced to abort.
            eprintln!("freeze lock held during guard drop, aborting!");
            std::process::abort()
        }
    }
}

/// Place a new value in a `FreezeRefCell` with an *erased* lifetime and return a `FreezeGuard`
/// which restores the previous value on drop.
///
/// This is used to ensure that all access to the (new) contents of the `FreezeRefCell` happens
/// between this call and the drop of `FreezeGuard`. It is *ensured* because if `FreezeGuard`
/// cannot restore the old contents of the cell, it will instead *abort the process*.
///
/// # Safety
///
/// The caller must ensure that the returned `FreezeGuard` is dropped before real lifetime of the
/// set value ends. Additionally, all user access to the set value must be `for<'f>` to ensure that
/// the accessing code works for whatever the real lifetime is.
#[inline]
unsafe fn freeze_value<'a, 'f, F: for<'b> Freeze<'b>>(
    cell: &'a FreezeRefCell<F>,
    v: <F as Freeze<'f>>::Frozen,
) -> FreezeGuard<'a, F> {
    let next =
        unsafe { mem::transmute::<<F as Freeze<'f>>::Frozen, <F as Freeze<'static>>::Frozen>(v) };

    let prev = cell
        .try_borrow_mut()
        .expect("`FreezeCell` value cannot be changed within `FreezeCell::with[_mut]`")
        .replace(next);

    FreezeGuard { cell, prev }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freeze_works() {
        struct F<'a>(&'a i32);

        let f = FreezeCell::<Freeze![F<'freeze>]>::new();

        f.freeze(F(&4), || {
            f.with(|f| {
                assert_eq!(*f.0, 4);
            })
            .unwrap();
        });

        assert!(f.inner.borrow().is_none());
    }

    #[test]
    fn test_freeze_expires() {
        struct F<'a>(&'a i32);

        let f = FreezeCell::<Freeze![F<'freeze>]>::new();
        assert_eq!(
            f.with(|f| {
                assert_eq!(*f.0, 4);
            }),
            Err(AccessError::Expired)
        );

        assert!(f.inner.borrow().is_none());
    }

    #[test]
    fn test_freeze_many() {
        struct FA<'a>(&'a i32);
        struct FB<'a>(&'a i32);

        let fa = FreezeCell::<Freeze![FA<'freeze>]>::new();
        let fb = FreezeCell::<Freeze![FB<'freeze>]>::new();

        FreezeMany::new()
            .freeze(&fa, FA(&1))
            .freeze(&fb, FB(&2))
            .in_scope(|| {
                fa.with(|fa| assert_eq!(*fa.0, 1)).unwrap();
                fb.with(|fb| assert_eq!(*fb.0, 2)).unwrap();
            });

        assert!(fa.inner.borrow().is_none());
        assert!(fb.inner.borrow().is_none());
    }
}
