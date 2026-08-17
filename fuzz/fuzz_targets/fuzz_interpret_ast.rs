#![no_main]

use arbitrary::Arbitrary;
use fabricator_cli::TestingStdlibContext as _;
use fabricator_compiler::{self as compiler, ir_gen::IrGenError};
use fabricator_vm as vm;
use gc_arena::{Collect, Gc};
use libfuzzer_sys::fuzz_target;
use thiserror::Error;

use std::{fmt, str};

#[derive(Arbitrary)]
pub enum Statement {
    Block(BlockStmt),
    Function(FunctionStmt),
    Repeat(RepeatStmt),
    With(WithStmt),
    TryCatch(TryCatchStmt),
    Call(Call),
    Throw(Expression),
    Return(Option<Expression>),
    Break,
    Continue,
}

impl fmt::Debug for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::Block(block) => {
                write!(f, "{block}")?;
            }
            Statement::Function(func) => {
                write!(f, "{func}")?;
            }
            Statement::Repeat(rep) => {
                write!(f, "{rep}")?;
            }
            Statement::With(with) => {
                write!(f, "{with}")?;
            }
            Statement::TryCatch(try_catch) => {
                write!(f, "{try_catch}")?;
            }
            Statement::Call(call) => {
                writeln!(f, "{call};")?;
            }
            Statement::Throw(expr) => {
                writeln!(f, "throw {expr};")?;
            }
            Statement::Return(expr) => {
                write!(f, "return")?;
                if let Some(expr) = expr {
                    write!(f, " {expr}")?;
                }
                writeln!(f, ";")?;
            }
            Statement::Break => {
                writeln!(f, "break;")?;
            }
            Statement::Continue => {
                writeln!(f, "continue;")?;
            }
        }
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct BlockStmt {
    pub stmts: Vec<Statement>,
}

impl fmt::Debug for BlockStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for BlockStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{{")?;
        for stmt in &self.stmts {
            write!(f, "{stmt}")?;
        }
        writeln!(f, "}}")?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct FunctionStmt {
    pub name: VarName,
    pub params: Vec<VarName>,
    pub block: BlockStmt,
}

impl fmt::Debug for FunctionStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for FunctionStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "function {}(", self.name)?;
        for (i, vname) in self.params.iter().enumerate() {
            write!(f, "{vname}")?;
            if i + 1 < self.params.len() {
                write!(f, ", ")?;
            }
        }
        write!(f, ") {}", &self.block)?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct RepeatStmt {
    pub target: Box<Expression>,
    pub stmt: Box<Statement>,
}

impl fmt::Debug for RepeatStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for RepeatStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "repeat {} ", &self.target)?;
        write!(f, "{}", &self.stmt)?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct WithStmt {
    pub target: Box<Expression>,
    pub stmt: Box<Statement>,
}

impl fmt::Debug for WithStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for WithStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "with {} ", &self.target)?;
        write!(f, "{}", &self.stmt)?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct TryCatchStmt {
    pub try_block: Box<Statement>,
    pub err_ident: VarName,
    pub catch_block: Box<Statement>,
}

impl fmt::Debug for TryCatchStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for TryCatchStmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "try {}", &self.try_block)?;
        write!(f, "catch({}) {}", &self.err_ident, &self.catch_block)?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub enum Expression {
    True,
    False,
    Num(i8),
    Ident(Ident),
    Call(Call),
    Object(Object),
    Function(FunctionExpr),
}

impl fmt::Debug for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expression::True => {
                write!(f, "(true)")?;
            }
            Expression::False => {
                write!(f, "(false)")?;
            }
            Expression::Num(n) => {
                write!(f, "({n})")?;
            }
            Expression::Ident(ident) => {
                write!(f, "{ident}")?;
            }
            Expression::Call(call) => {
                write!(f, "{call}")?;
            }
            Expression::Object(obj) => {
                write!(f, "({obj})")?;
            }
            Expression::Function(func) => {
                write!(f, "({func})")?;
            }
        }
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct Call {
    pub func: Box<Expression>,
    pub args: Vec<Expression>,
}

impl fmt::Debug for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", &self.func)?;
        for (i, arg) in self.args.iter().enumerate() {
            write!(f, "{arg}")?;
            if i + 1 < self.args.len() {
                write!(f, ", ")?;
            }
        }
        write!(f, ")")?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct Object(Vec<(VarName, Expression)>);

impl fmt::Debug for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{{")?;
        for (k, v) in &self.0 {
            writeln!(f, "{}: {},", k, v)?;
        }
        write!(f, "}}")?;
        Ok(())
    }
}

#[derive(Arbitrary)]
pub struct FunctionExpr {
    pub params: Vec<VarName>,
    pub block: BlockStmt,
}

impl fmt::Debug for FunctionExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for FunctionExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "function(")?;
        for (i, vname) in self.params.iter().enumerate() {
            write!(f, "{vname}")?;
            if i + 1 < self.params.len() {
                write!(f, ", ")?;
            }
        }
        write!(f, ") {}", &self.block)?;
        Ok(())
    }
}

#[derive(Copy, Clone, Arbitrary)]
pub enum Ident {
    Method,
    Pcall,
    ArrayCreateExt,
    Var(VarName),
}

impl Ident {
    pub fn as_str(self) -> &'static str {
        match self {
            Ident::Method => "method",
            Ident::Pcall => "pcall",
            Ident::ArrayCreateExt => "array_create_ext",
            Ident::Var(var_name) => var_name.as_str(),
        }
    }
}

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Copy, Clone, Arbitrary)]
pub enum VarName {
    A,
    B,
    C,
    D,
    E,
}

impl VarName {
    pub fn all() -> impl Iterator<Item = VarName> {
        [Self::A, Self::B, Self::C, Self::D, Self::E].into_iter()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VarName::A => "a",
            VarName::B => "b",
            VarName::C => "c",
            VarName::D => "d",
            VarName::E => "e",
        }
    }
}

impl fmt::Debug for VarName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for VarName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

fuzz_target!(|block: BlockStmt| {
    const FRAME_LIMIT: u32 = 64;
    const INST_LIMIT: u32 = 16384;

    let code = format!("{block}");

    let interpreter = vm::Interpreter::new();
    let settings = compiler::CompileSettings::compat();

    #[derive(Debug, Error)]
    #[error("vm limit reached")]
    struct VmLimitError;

    #[derive(Default, Collect)]
    #[collect(require_static)]
    struct VmLimiter {
        frames_available: u32,
        insts_available: u32,
    }

    impl VmLimiter {
        fn new() -> Self {
            Self {
                frames_available: FRAME_LIMIT,
                insts_available: INST_LIMIT,
            }
        }
    }

    impl<'gc> vm::Hook<'gc> for VmLimiter {
        fn on_call(
            &mut self,
            _ctx: vm::Context<'gc>,
            _backtrace: vm::Backtrace<'gc, '_>,
        ) -> Result<(), vm::RuntimeError> {
            if let Some(dec) = self.frames_available.checked_sub(1) {
                self.frames_available = dec;
                Ok(())
            } else {
                Err(VmLimitError.into())
            }
        }

        fn on_return(&mut self, _ctx: vm::Context<'gc>, _backtrace: vm::Backtrace<'gc, '_>) {
            self.frames_available += 1;
        }

        fn on_step(
            &mut self,
            _ctx: vm::Context<'gc>,
            instruction_count: u32,
        ) -> Result<u32, vm::RuntimeError> {
            self.insts_available = self.insts_available.saturating_sub(instruction_count);
            if self.insts_available == 0 {
                Err(VmLimitError.into())
            } else {
                Ok(self.insts_available)
            }
        }
    }

    let stdlib = interpreter.enter(|ctx| {
        let mut lib = vm::MagicSet::new();
        lib.merge(&ctx.testing_stdlib());

        for n in VarName::all() {
            lib.insert(
                ctx.intern(n.as_str()),
                vm::magic::MagicConstant::new_ptr(&ctx, vm::Value::Boolean(true)),
            );
        }

        ctx.stash(Gc::new(&ctx, lib))
    });

    let thread = interpreter.enter(|ctx| {
        let thread = vm::Thread::new(&ctx);
        thread.set_hook(&ctx, VmLimiter::new());
        ctx.stash(thread)
    });

    let closure = interpreter.enter(|ctx| -> Result<_, compiler::CompileError> {
        let output = compiler::Compiler::compile_chunk(
            ctx,
            "",
            compiler::ImportItems::with_magic(&ctx, ctx.fetch(&stdlib)),
            settings,
            "<fuzzer input>",
            &code,
        )?;
        let closure = vm::Closure::new(&ctx, output.chunk_prototype, vm::Value::Undefined).unwrap();
        Ok(ctx.stash(closure))
    });

    let closure = match closure {
        Ok(closure) => closure,
        Err(compiler::CompileError {
            kind:
                compiler::compiler::CompileErrorKind::IrGen(IrGenError {
                    kind:
                        compiler::ir_gen::IrGenErrorKind::BreakWithNoTarget
                        | compiler::ir_gen::IrGenErrorKind::ContinueWithNoTarget,
                    ..
                }),
            ..
        }) => {
            return;
        }
        Err(err) => panic!("compiler error: {err:?}"),
    };

    if let Err(
        vm::CallError::Vm {
            error: vm::ExternError::Runtime(err),
            ..
        }
        | vm::CallError::Runtime(err),
    ) = interpreter.enter(|ctx| ctx.fetch(&thread).run(ctx, ctx.fetch(&closure)))
    {
        if let Some(op_err) = err.downcast_ref::<vm::thread::OpError>() {
            match op_err {
                vm::thread::OpError::NoStackFrame { .. } => panic!("{op_err:?}"),
                _ => {}
            }
        }
    }
});
