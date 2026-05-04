use std::{error::Error as StdError, fmt, ops, sync::Arc};

use gc_arena::Collect;

/// A shareable, dynamically typed wrapper around a normal Rust error.
///
/// Rust errors can be caught and re-raised through FML which allows for unrestricted sharing, so
/// this type contains its error inside an `Arc` pointer to allow for this.
#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct RuntimeError(pub ThinArcError);

impl fmt::Debug for RuntimeError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.err_ref(), f)
    }
}

impl fmt::Display for RuntimeError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.err_ref(), f)
    }
}

impl<E: StdError + Send + Sync + 'static> From<E> for RuntimeError {
    #[inline]
    fn from(err: E) -> Self {
        Self::new(err)
    }
}

impl ops::Deref for RuntimeError {
    type Target = dyn StdError + Send + Sync + 'static;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.err_ref()
    }
}

impl AsRef<dyn StdError + Send + Sync + 'static> for RuntimeError {
    #[inline]
    fn as_ref(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self.0.err_ref()
    }
}

impl RuntimeError {
    pub fn new<E: StdError + Send + Sync + 'static>(err: E) -> Self {
        RuntimeError(ThinArcError::new(err))
    }

    pub fn from_boxed(boxed_err: Box<dyn StdError + Send + Sync + 'static>) -> Self {
        struct BoxErr(Box<dyn StdError + Send + Sync + 'static>);

        impl fmt::Debug for BoxErr {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl fmt::Display for BoxErr {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl StdError for BoxErr {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                self.0.source()
            }
        }

        Self::new(BoxErr(boxed_err.into()))
    }

    pub fn msg<M: fmt::Display + fmt::Debug + Send + Sync + 'static>(message: M) -> Self {
        struct MsgErr<M>(M);

        impl<M: fmt::Debug> fmt::Debug for MsgErr<M> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<M: fmt::Display> fmt::Display for MsgErr<M> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<M: fmt::Display + fmt::Debug> StdError for MsgErr<M> {}

        Self::new(MsgErr(message))
    }

    #[inline]
    pub fn is<T: StdError + Send + Sync + 'static>(&self) -> bool {
        self.as_ref().is::<T>()
    }

    #[inline]
    pub fn downcast_ref<T: StdError + Send + Sync + 'static>(&self) -> Option<&T> {
        self.as_ref().downcast_ref()
    }

    /// Convert this `RuntimeError` into a cloneable type that directly implements [`StdError`].
    ///
    /// This conversion is free and only changes the wrapper type around the inner `ThinArcError`.
    #[inline]
    pub fn into_stderr(self) -> SharedError {
        SharedError(self.0)
    }
}

#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct SharedError(pub ThinArcError);

impl fmt::Debug for SharedError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.err_ref(), f)
    }
}

impl fmt::Display for SharedError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.err_ref(), f)
    }
}

impl StdError for SharedError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.err_ref().source()
    }
}

impl SharedError {
    pub fn into_runtime_err(self) -> RuntimeError {
        RuntimeError(self.0)
    }
}

struct ThinArcErrorVTable {
    clone: unsafe fn(*const ThinArcErrorHeader) -> *const ThinArcErrorHeader,
    drop: unsafe fn(*const ThinArcErrorHeader),
    error_ref:
        unsafe fn(*const ThinArcErrorHeader) -> *const (dyn StdError + Send + Sync + 'static),
}

#[repr(transparent)]
struct ThinArcErrorHeader {
    vtable: &'static ThinArcErrorVTable,
}

/// Performance is extremely sensitive to the size of `RuntimeError`, so we represent it as a single
/// pointer with an inline VTable header.
pub struct ThinArcError(*const ThinArcErrorHeader);

unsafe impl Send for ThinArcError {}
unsafe impl Sync for ThinArcError {}

impl Clone for ThinArcError {
    fn clone(&self) -> Self {
        Self(unsafe { ((*self.0).vtable.clone)(self.0) })
    }
}

impl Drop for ThinArcError {
    fn drop(&mut self) {
        unsafe { ((*self.0).vtable.drop)(self.0) }
    }
}

impl ThinArcError {
    pub fn new<E: StdError + Send + Sync + 'static>(err: E) -> ThinArcError {
        #[repr(C)]
        struct HeaderError<E> {
            header: ThinArcErrorHeader,
            error: E,
        }

        // Helper trait to materialize vtables in static memory.
        trait HasThinArcErrVtable {
            const VTABLE: ThinArcErrorVTable;
        }

        impl<'gc, E: StdError + Send + Sync + 'static> HasThinArcErrVtable for E {
            const VTABLE: ThinArcErrorVTable = ThinArcErrorVTable {
                clone: |ptr| unsafe {
                    Arc::increment_strong_count(ptr as *const HeaderError<E>);
                    ptr
                },
                drop: |ptr| unsafe {
                    let _ = Arc::from_raw(ptr as *const HeaderError<E>);
                },
                error_ref: |ptr| unsafe {
                    let ptr = ptr as *const HeaderError<E>;
                    &raw const (*ptr).error as *const (dyn StdError + Send + Sync + 'static)
                },
            };
        }

        let vtable: &'static _ = &<E as HasThinArcErrVtable>::VTABLE;

        let he = Arc::new(HeaderError {
            header: ThinArcErrorHeader { vtable },
            error: err,
        });

        ThinArcError(Arc::into_raw(he) as *const ThinArcErrorHeader)
    }

    #[inline]
    pub fn err_ref(&self) -> &'_ (dyn StdError + Send + Sync + 'static) {
        unsafe { &*((*self.0).vtable.error_ref)(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{self, AtomicBool};

    use super::*;

    #[test]
    fn runtime_error() {
        struct TestErr {
            dropped: Arc<AtomicBool>,
        }

        impl fmt::Display for TestErr {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "TestErr")
            }
        }

        impl fmt::Debug for TestErr {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self)
            }
        }

        impl Drop for TestErr {
            fn drop(&mut self) {
                self.dropped.store(true, atomic::Ordering::Release);
            }
        }

        impl StdError for TestErr {}

        let dropped = Arc::new(AtomicBool::new(false));

        let rt_err = RuntimeError::new(TestErr {
            dropped: dropped.clone(),
        });

        assert_eq!(&format!("{}", rt_err), "TestErr");
        assert_eq!(&format!("{:?}", rt_err), "TestErr");

        drop(rt_err);

        assert!(dropped.load(atomic::Ordering::Acquire));
    }
}
