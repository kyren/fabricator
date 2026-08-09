use std::{error::Error as StdError, fmt};

use crate::{
    callback::Callback,
    closure::Closure,
    debug::LineNumber,
    error::{Error, ExternError, RawGc, RuntimeError, ScriptError},
    string::SharedStr,
};

#[derive(Debug)]
pub struct VmError<'gc> {
    pub error: Error<'gc>,
    pub backtrace: Option<Box<[BacktraceFrame<'gc>]>>,
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
                    BacktraceFrame::Closure(closure_frame) => {
                        write!(
                            f,
                            "{}:{}",
                            closure_frame.chunk_name(),
                            closure_frame.line_number()
                        )?;
                    }
                    BacktraceFrame::Callback(callback) => {
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
            backtrace: self
                .backtrace
                .map(|b| b.into_iter().map(|f| f.to_extern()).collect()),
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

#[derive(Debug)]
pub struct ExternVmError {
    pub error: ExternError,
    pub backtrace: Option<Box<[ExternBacktraceFrame]>>,
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
                    ExternBacktraceFrame::Closure(closure_frame) => {
                        write!(
                            f,
                            "{}:{}",
                            closure_frame.chunk_name, closure_frame.line_number
                        )?;
                    }
                    ExternBacktraceFrame::Callback(callback) => {
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

#[derive(Debug, Copy, Clone)]
pub struct ClosureBacktraceFrame<'gc> {
    pub closure: Closure<'gc>,
    pub instruction: usize,
}

impl<'gc> ClosureBacktraceFrame<'gc> {
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

    pub fn to_extern(&self) -> ExternClosureBacktraceFrame {
        ExternClosureBacktraceFrame {
            closure: RawGc::new(self.closure.into_inner()),
            instruction: self.instruction,
            line_number: self.line_number(),
            chunk_name: self.chunk_name().clone(),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum BacktraceFrame<'gc> {
    Closure(ClosureBacktraceFrame<'gc>),
    Callback(Callback<'gc>),
}

impl<'gc> BacktraceFrame<'gc> {
    pub fn to_extern(&self) -> ExternBacktraceFrame {
        match self {
            BacktraceFrame::Closure(closure_backtrace_frame) => {
                ExternBacktraceFrame::Closure(closure_backtrace_frame.to_extern())
            }
            BacktraceFrame::Callback(callback) => {
                ExternBacktraceFrame::Callback(RawGc::new(callback.into_inner()))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternClosureBacktraceFrame {
    pub closure: RawGc,
    pub instruction: usize,
    pub line_number: LineNumber,
    pub chunk_name: SharedStr,
}

#[derive(Debug, Clone)]
pub enum ExternBacktraceFrame {
    Closure(ExternClosureBacktraceFrame),
    Callback(RawGc),
}
