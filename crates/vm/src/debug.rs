use std::fmt;

use gc_arena::{Collect, Gc, Mutation, Rootable, arena::Root, barrier};

use crate::{
    any::{Any, AnyInner},
    string::SharedStr,
};

/// A region of some chunk, expressed in byte offsets.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Collect)]
#[collect(require_static)]
pub struct Span {
    start: usize,
    end: usize,
}

impl Span {
    /// Create a new `Span`.
    ///
    /// # Panics
    ///
    /// Panics if `start` is not less than or equal to `end`.
    #[must_use]
    pub fn new(start: usize, end: usize) -> Self {
        assert!(start <= end);
        Self { start, end }
    }

    /// Returns a maximally empty span with `start` set to `usize::MAX` and `end` set to `0`.
    ///
    /// Combining a null span with any other span will always result the other span.
    #[must_use]
    pub fn null() -> Self {
        Self {
            start: usize::MAX,
            end: 0,
        }
    }

    /// Returns true if this span is the special null span.
    #[must_use]
    pub fn is_null(&self) -> bool {
        *self == Self::null()
    }

    /// Returns a maximally full span with `start` set to `0` and `end` set to `usize::MAX`.
    ///
    /// Combining an everywhere span with another span will always result in the everywhere span.
    #[must_use]
    pub fn everywhere() -> Self {
        Self {
            start: 0,
            end: usize::MAX,
        }
    }

    /// Returns true if this span is the special everywhere span.
    pub fn is_everywhere(&self) -> bool {
        *self == Self::everywhere()
    }

    /// Returns an empty span starting at the given position.
    #[must_use]
    pub fn empty(start: usize) -> Self {
        Self { start, end: start }
    }

    /// Returns true if `start` is not strictly less than `end`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    #[must_use]
    pub fn start(&self) -> usize {
        self.start
    }

    #[must_use]
    pub fn start_span(&self) -> Span {
        Span::empty(self.start)
    }

    #[must_use]
    pub fn end(&self) -> usize {
        self.end
    }

    #[must_use]
    pub fn end_span(&self) -> Span {
        Span::empty(self.end)
    }

    /// Return a span that encloses both this span and the given span.
    #[must_use]
    pub fn combine(&self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A line number within a chunk.
///
/// It is stored as 0-indexed internally, but will display as a more human-readable 1-indexed line
/// number.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LineNumber(pub usize);

impl fmt::Display for LineNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0 + 1)
    }
}

/// A trait for representing a single unit of FML source code, generally a single source file, for
/// the purposes of displaying debug information.
pub trait ChunkData {
    /// The name of this chunk, usually the name of the source code file.
    #[must_use]
    fn name(&self) -> &SharedStr;

    /// Returns the line number for a given byte offset.
    #[must_use]
    fn line_number(&self, byte_offset: usize) -> LineNumber;
}

impl<T: ChunkData> ChunkData for gc_arena::Static<T> {
    fn name(&self) -> &SharedStr {
        self.0.name()
    }

    fn line_number(&self, byte_offset: usize) -> LineNumber {
        self.0.line_number(byte_offset)
    }
}

#[derive(Debug, Copy, Clone)]
struct ChunkMethods {
    name: for<'gc> fn(Any<'gc, ChunkMeta>) -> &'gc SharedStr,
    line_number: for<'gc> fn(Any<'gc, ChunkMeta>, usize) -> LineNumber,
}

impl ChunkMethods {
    fn new<R>() -> &'static Self
    where
        R: for<'a> Rootable<'a> + 'static,
        for<'gc> Root<'gc, R>: Sized + ChunkData + Collect<'gc>,
    {
        &Self {
            name: |any| any.downcast::<R>().unwrap().name(),
            line_number: |any, byte_offset| any.downcast::<R>().unwrap().line_number(byte_offset),
        }
    }
}

/// Meta-data for a `Chunk` type.
#[derive(Debug, Copy, Clone, Collect)]
#[collect(require_static)]
pub struct ChunkMeta {
    methods: &'static ChunkMethods,
}

pub type ChunkInner = AnyInner<ChunkMeta>;

/// A handle to metadata for a single unit of FML source code, for the purposes of displaying debug
/// information.
///
/// Internally holds an implementation of `ChunkData` and allows for downcasting.
#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct Chunk<'gc>(Any<'gc, ChunkMeta>);

impl<'gc> PartialEq for Chunk<'gc> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl<'gc> Eq for Chunk<'gc> {}

impl<'gc> Chunk<'gc> {
    #[must_use]
    pub fn new<R>(mc: &Mutation<'gc>, chunk: Root<'gc, R>) -> Self
    where
        R: for<'a> Rootable<'a> + 'static,
        for<'a> Root<'a, R>: Sized + ChunkData + Collect<'a>,
    {
        Self(Any::with_metadata::<R>(
            mc,
            ChunkMeta {
                methods: ChunkMethods::new::<R>(),
            },
            chunk,
        ))
    }

    #[must_use]
    pub fn new_static<T>(mc: &Mutation<'gc>, val: T) -> Self
    where
        T: ChunkData + 'static,
    {
        Self::new::<gc_arena::Static<T>>(mc, val.into())
    }

    #[must_use]
    pub fn from_inner(inner: Gc<'gc, ChunkInner>) -> Self {
        Self(Any::from_inner(inner))
    }

    #[must_use]
    pub fn into_inner(self) -> Gc<'gc, ChunkInner> {
        self.0.into_inner()
    }

    #[must_use]
    pub fn downcast<R>(self) -> Option<&'gc Root<'gc, R>>
    where
        R: for<'b> Rootable<'b> + 'static,
        Root<'gc, R>: Sized,
    {
        self.0.downcast::<R>()
    }

    #[must_use]
    pub fn downcast_write<R>(self, mc: &Mutation<'gc>) -> Option<&'gc barrier::Write<Root<'gc, R>>>
    where
        R: for<'b> Rootable<'b> + 'static,
        Root<'gc, R>: Sized,
    {
        self.0.downcast_write::<R>(mc)
    }

    #[must_use]
    pub fn downcast_static<T: 'static>(self) -> Option<&'gc T> {
        self.downcast::<gc_arena::Static<T>>().map(|r| &r.0)
    }

    #[must_use]
    pub fn name(self) -> &'gc SharedStr {
        (self.0.metadata().methods.name)(self.0)
    }

    #[must_use]
    pub fn line_number(self, byte_offset: usize) -> LineNumber {
        (self.0.metadata().methods.line_number)(self.0, byte_offset)
    }

    /// Returns a printable identifier for a function within a chunk.
    #[must_use]
    pub fn function_identifier(self, reference: FunctionRef) -> FunctionIdentifier {
        match reference {
            FunctionRef::Named(ref_name, span) => FunctionIdentifier {
                chunk_name: self.name().clone(),
                line_number: Some(self.line_number(span.start())),
                function_ref_name: Some(ref_name),
            },
            FunctionRef::Expression(span) => FunctionIdentifier {
                chunk_name: self.name().clone(),
                line_number: Some(self.line_number(span.start())),
                function_ref_name: None,
            },
            FunctionRef::Chunk => FunctionIdentifier {
                chunk_name: self.name().clone(),
                line_number: None,
                function_ref_name: None,
            },
        }
    }
}

/// The source origination of a prototype within some chunk.
#[derive(Debug, Clone, Collect)]
#[collect(require_static)]
pub enum FunctionRef {
    // The function has a name from a declaration statement and the span is of the statement.
    Named(SharedStr, Span),
    // The function is an anonymous expression and the span is of the expression.
    Expression(Span),
    // The function is top-level and represents execution of an entire chunk.
    Chunk,
}

impl FunctionRef {
    #[must_use]
    pub fn span(&self) -> Span {
        match *self {
            FunctionRef::Named(_, span) => span,
            FunctionRef::Expression(span) => span,
            FunctionRef::Chunk => Span::everywhere(),
        }
    }
}

pub struct FunctionIdentifier {
    chunk_name: SharedStr,
    line_number: Option<LineNumber>,
    function_ref_name: Option<SharedStr>,
}

impl fmt::Display for FunctionIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chunk_name = &self.chunk_name;
        match (&self.function_ref_name, &self.line_number) {
            (Some(line_number), Some(function_ref_name)) => {
                write!(f, "{chunk_name}:{function_ref_name}:{line_number}")
            }
            (Some(line_number), None) => write!(f, "{chunk_name}:{line_number}"),
            _ => write!(f, "{chunk_name}"),
        }
    }
}
