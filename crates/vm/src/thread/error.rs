use std::{error::Error as StdError, fmt, ops, rc::Rc, sync::Arc};

use gc_arena::{Collect, Mutation, Rootable};

use crate::{
    callback::Callback,
    closure::Closure,
    debug::LineNumber,
    error::{Error, ExternError, RawGc, RuntimeError, ScriptError},
    string::SharedStr,
    user_data::{BadUserDataType, UserData},
};

#[derive(Debug)]
pub struct VmError<'gc> {
    pub error: Error<'gc>,
    pub backtrace: Option<Backtrace<'gc>>,
}

impl<'gc> fmt::Display for VmError<'gc> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.error)?;
        if let Some(backtrace) = &self.backtrace {
            write!(f, "VM backtrace:")?;
            for (i, frame) in backtrace.iter().rev().enumerate() {
                writeln!(f)?;
                write!(f, "{:>4}: ", i)?;
                match frame {
                    StackFrame::Closure(closure_frame) => {
                        write!(
                            f,
                            "{}:{}",
                            closure_frame.chunk_name(),
                            closure_frame.line_number()
                        )?;
                    }
                    StackFrame::Callback(callback) => {
                        write!(f, "<callback {:p}>", callback.into_inner())?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<'gc> VmError<'gc> {
    pub fn into_extern(self) -> ExternVmError {
        ExternVmError {
            error: self.error.into_extern(),
            backtrace: self.backtrace.map(|b| b.to_extern()),
        }
    }
}

impl<'gc> From<Error<'gc>> for VmError<'gc> {
    fn from(error: Error<'gc>) -> Self {
        Self {
            error,
            backtrace: None,
        }
    }
}

impl<'gc> From<ScriptError<'gc>> for VmError<'gc> {
    fn from(err: ScriptError<'gc>) -> Self {
        Self {
            error: Error::Script(err),
            backtrace: None,
        }
    }
}

impl<'gc> From<RuntimeError> for VmError<'gc> {
    fn from(err: RuntimeError) -> Self {
        Self {
            error: Error::Runtime(err),
            backtrace: None,
        }
    }
}

impl<'gc, E: StdError + Send + Sync + 'static> From<E> for VmError<'gc> {
    fn from(err: E) -> Self {
        Self {
            error: Error::Runtime(err.into()),
            backtrace: None,
        }
    }
}

#[derive(Debug, Clone, Collect)]
#[collect(no_drop)]
pub struct Backtrace<'gc>(Rc<[StackFrame<'gc>]>);

impl<'gc> ops::Deref for Backtrace<'gc> {
    type Target = [StackFrame<'gc>];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'gc> FromIterator<StackFrame<'gc>> for Backtrace<'gc> {
    fn from_iter<T: IntoIterator<Item = StackFrame<'gc>>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'gc> Backtrace<'gc> {
    /// Convert a backtrace into a userdata object.
    pub fn into_userdata(self, mc: &Mutation<'gc>) -> UserData<'gc> {
        UserData::new::<Rootable![Backtrace<'_>]>(mc, self)
    }

    pub fn from_userdata(ud: UserData<'gc>) -> Result<Backtrace<'gc>, BadUserDataType> {
        Ok(ud.downcast::<Rootable![Backtrace<'_>]>()?.clone())
    }

    pub fn to_extern(&self) -> ExternBacktrace {
        self.0.iter().map(|f| f.to_extern()).collect()
    }
}

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct ClosureStackFrame<'gc> {
    pub closure: Closure<'gc>,
    pub instruction: usize,
}

impl<'gc> ClosureStackFrame<'gc> {
    pub fn chunk_name(&self) -> &SharedStr {
        self.closure.prototype().chunk().name()
    }

    pub fn line_number(&self) -> LineNumber {
        let chunk = self.closure.prototype().chunk();
        let prototype = self.closure.prototype();
        let bytecode = prototype.bytecode();
        let span = if self.instruction < bytecode.instruction_len() {
            bytecode.span(self.instruction)
        } else {
            prototype.reference().span().end_span()
        };
        chunk.line_number(span.start())
    }

    pub fn to_extern(&self) -> ExternClosureStackFrame {
        ExternClosureStackFrame {
            closure: RawGc::new(self.closure.into_inner()),
            instruction: self.instruction,
            line_number: self.line_number(),
            chunk_name: self.chunk_name().clone(),
        }
    }
}

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub enum StackFrame<'gc> {
    Closure(ClosureStackFrame<'gc>),
    Callback(Callback<'gc>),
}

impl<'gc> StackFrame<'gc> {
    pub fn to_extern(&self) -> ExternStackFrame {
        match self {
            StackFrame::Closure(closure_backtrace_frame) => {
                ExternStackFrame::Closure(closure_backtrace_frame.to_extern())
            }
            StackFrame::Callback(callback) => {
                ExternStackFrame::Callback(RawGc::new(callback.into_inner()))
            }
        }
    }
}

#[derive(Debug)]
pub struct ExternVmError {
    pub error: ExternError,
    pub backtrace: Option<ExternBacktrace>,
}

impl fmt::Display for ExternVmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.error)?;
        if let Some(backtrace) = &self.backtrace {
            write!(f, "VM backtrace:")?;
            for (i, frame) in backtrace.iter().rev().enumerate() {
                writeln!(f)?;
                write!(f, "{:>4}: ", i)?;
                match frame {
                    ExternStackFrame::Closure(closure_frame) => {
                        write!(
                            f,
                            "{}:{}",
                            closure_frame.chunk_name, closure_frame.line_number
                        )?;
                    }
                    ExternStackFrame::Callback(callback) => {
                        write!(f, "<callback {:p}>", callback)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl StdError for ExternVmError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.error.source()
    }
}

#[derive(Debug, Clone)]
pub struct ExternBacktrace(Arc<[ExternStackFrame]>);

impl ops::Deref for ExternBacktrace {
    type Target = [ExternStackFrame];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromIterator<ExternStackFrame> for ExternBacktrace {
    fn from_iter<T: IntoIterator<Item = ExternStackFrame>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[derive(Debug, Clone)]
pub struct ExternClosureStackFrame {
    pub closure: RawGc,
    pub instruction: usize,
    pub line_number: LineNumber,
    pub chunk_name: SharedStr,
}

#[derive(Debug, Clone)]
pub enum ExternStackFrame {
    Closure(ExternClosureStackFrame),
    Callback(RawGc),
}
