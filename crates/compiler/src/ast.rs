use std::{
    borrow,
    fmt::Debug,
    hash,
    ops::{self, ControlFlow},
};

use fabricator_vm::Span;

use crate::constant::Constant;

#[derive(Debug, Clone)]
pub struct Block<S> {
    pub statements: Vec<Statement<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Statement<S> {
    Empty(Span),
    Block(BlockStmt<S>),
    Enum(EnumStmt<S>),
    Function(FunctionStmt<S>),
    Closure(ClosureStmt<S>),
    Var(VarDeclarationStmt<S>),
    Static(VarDeclarationStmt<S>),
    Let(LetDeclarationStmt<S>),
    StaticLet(LetDeclarationStmt<S>),
    GlobalVar(Ident<S>),
    Assignment(AssignmentStmt<S>),
    Return(ReturnStmt<S>),
    If(IfStmt<S>),
    For(ForStmt<S>),
    While(LoopStmt<S>),
    Repeat(LoopStmt<S>),
    Switch(SwitchStmt<S>),
    With(LoopStmt<S>),
    TryCatch(TryCatchStmt<S>),
    Throw(ThrowStmt<S>),
    Call(Call<S>),
    Prefix(Mutation<S>),
    Postfix(Mutation<S>),
    Exit(Span),
    Break(Span),
    Continue(Span),
}

#[derive(Debug, Clone)]
pub struct BlockStmt<S> {
    pub block: Block<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumStmt<S> {
    pub name: Ident<S>,
    pub variants: Vec<(Ident<S>, Option<Expression<S>>)>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionStmt<S> {
    pub name: Ident<S>,
    pub is_constructor: bool,
    pub inherit: Option<Call<S>>,
    pub parameters: ParameterList<S>,
    pub body: Block<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ClosureStmt<S> {
    pub name: Ident<S>,
    pub parameters: ParameterList<S>,
    pub body: Block<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VarDeclarationStmt<S> {
    pub vars: Vec<(Ident<S>, Option<Expression<S>>)>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct LetDeclarationStmt<S> {
    pub vars: Vec<Ident<S>>,
    pub exprs: Vec<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AssignmentStmt<S> {
    pub target: MutableExpr<S>,
    pub op: AssignmentOp,
    pub value: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum MutableExpr<S> {
    Ident(Ident<S>),
    Field(FieldExpr<S>),
    Index(IndexExpr<S>),
}

#[derive(Debug, Clone)]
pub struct ReturnStmt<S> {
    pub values: Vec<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IfStmt<S> {
    pub condition: Box<Expression<S>>,
    pub then_stmt: Box<Statement<S>>,
    pub else_stmt: Option<Box<Statement<S>>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ForStmt<S> {
    pub initializer: Box<Statement<S>>,
    pub condition: Box<Expression<S>>,
    pub iterator: Box<Statement<S>>,
    pub body: Box<Statement<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct LoopStmt<S> {
    pub target: Box<Expression<S>>,
    pub body: Box<Statement<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ThrowStmt<S> {
    pub target: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TryCatchStmt<S> {
    pub try_block: Box<Statement<S>>,
    pub err_ident: Ident<S>,
    pub catch_block: Box<Statement<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SwitchStmt<S> {
    pub target: Box<Expression<S>>,
    pub cases: Vec<SwitchCase<S>>,
    pub default: Option<Block<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SwitchCase<S> {
    pub compare: Expression<S>,
    pub body: Block<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Expression<S> {
    Global(Span),
    This(Span),
    Other(Span),
    Constant(Constant<S>, Span),
    Ident(Ident<S>),
    Group(GroupExpr<S>),
    Object(ObjectExpr<S>),
    Array(ArrayExpr<S>),
    Unary(UnaryExpr<S>),
    Prefix(Mutation<S>),
    Postfix(Mutation<S>),
    Binary(BinaryExpr<S>),
    Ternary(TernaryExpr<S>),
    Function(FunctionExpr<S>),
    Closure(ClosureExpr<S>),
    Call(Call<S>),
    Field(FieldExpr<S>),
    Index(IndexExpr<S>),
    VarArgs(Span),
    Argument(ArgumentExpr<S>),
    ArgumentCount(Span),
}

#[derive(Debug, Clone)]
pub struct GroupExpr<S> {
    pub inner: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ObjectExpr<S> {
    pub fields: Vec<Field<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ArrayExpr<S> {
    pub entries: Vec<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct UnaryExpr<S> {
    pub op: UnaryOp,
    pub target: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Mutation<S> {
    pub op: MutationOp,
    pub target: Box<MutableExpr<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct BinaryExpr<S> {
    pub left: Box<Expression<S>>,
    pub op: BinaryOp,
    pub right: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TernaryExpr<S> {
    pub cond: Box<Expression<S>>,
    pub if_true: Box<Expression<S>>,
    pub if_false: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionExpr<S> {
    pub is_constructor: bool,
    pub inherit: Option<Call<S>>,
    pub parameters: ParameterList<S>,
    pub body: Block<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ClosureExpr<S> {
    pub parameters: ParameterList<S>,
    pub body: Block<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FieldExpr<S> {
    pub base: Box<Expression<S>>,
    pub field: Ident<S>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct IndexExpr<S> {
    pub base: Box<Expression<S>>,
    pub accessor_type: Option<AccessorType>,
    pub indexes: Vec<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ArgumentExpr<S> {
    pub arg_index: Box<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Call<S> {
    pub base: Box<Expression<S>>,
    pub arguments: Vec<Expression<S>>,
    pub has_new: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Parameter<S> {
    pub name: Ident<S>,
    pub default: Option<Expression<S>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ParameterList<S> {
    pub fixed: Vec<Parameter<S>>,
    pub var_args: Option<Span>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Field<S> {
    Value(Ident<S>, Expression<S>),
    Init(Ident<S>),
}

#[derive(Debug, Copy, Clone)]
pub struct Ident<S> {
    pub inner: S,
    pub span: Span,
}

impl<S> Ident<S> {
    pub fn new(inner: S, span: Span) -> Self {
        Self { inner, span }
    }
}

impl<S: PartialEq> PartialEq for Ident<S> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.eq(&other.inner)
    }
}

impl<S: Eq> Eq for Ident<S> {}

impl<S: hash::Hash> hash::Hash for Ident<S> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl<S> ops::Deref for Ident<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<S> borrow::Borrow<S> for Ident<S> {
    fn borrow(&self) -> &S {
        &self.inner
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum UnaryOp {
    Not,
    Minus,
    BitNegate,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum MutationOp {
    Increment,
    Decrement,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum BinaryOp {
    Add,
    Sub,
    Mult,
    Div,
    Mod,
    Rem,
    IDiv,
    Equal,
    NotEqual,
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,
    And,
    Or,
    Xor,
    BitAnd,
    BitOr,
    BitXor,
    BitShiftLeft,
    BitShiftRight,
    NullCoalesce,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum AssignmentOp {
    Equal,
    PlusEqual,
    MinusEqual,
    MultEqual,
    DivEqual,
    RemEqual,
    BitAndEqual,
    BitOrEqual,
    BitXorEqual,
    NullCoalesce,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum AccessorType {
    List,
    Map,
    Grid,
    Array,
    Struct,
}

pub trait Visitor<S>: Sized {
    type Break;

    fn visit_stmt(&mut self, stmt: &Statement<S>) -> ControlFlow<Self::Break> {
        stmt.walk(self)
    }

    fn visit_expr(&mut self, expr: &Expression<S>) -> ControlFlow<Self::Break> {
        expr.walk(self)
    }
}

pub trait VisitorMut<S>: Sized {
    type Break;

    fn visit_stmt_mut(&mut self, stmt: &mut Statement<S>) -> ControlFlow<Self::Break> {
        stmt.walk_mut(self)
    }

    fn visit_expr_mut(&mut self, expr: &mut Expression<S>) -> ControlFlow<Self::Break> {
        expr.walk_mut(self)
    }
}

pub trait Walk<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break>;
}

pub trait WalkMut<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break>;
}

impl<S> Walk<S> for Block<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for stmt in &self.statements {
            visitor.visit_stmt(stmt)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for Block<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for stmt in &mut self.statements {
            visitor.visit_stmt_mut(stmt)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Statement<S> {
    pub fn span(&self) -> Span {
        match self {
            Statement::Empty(span) => *span,
            Statement::Block(block) => block.span,
            Statement::Enum(enum_stmt) => enum_stmt.span,
            Statement::Function(function_stmt) => function_stmt.span,
            Statement::Closure(closure_stmt) => closure_stmt.span,
            Statement::Var(var_stmt) => var_stmt.span,
            Statement::Static(var_stmt) => var_stmt.span,
            Statement::Let(let_stmt) => let_stmt.span,
            Statement::StaticLet(let_stmt) => let_stmt.span,
            Statement::GlobalVar(ident) => ident.span,
            Statement::Assignment(assignment_stmt) => assignment_stmt.span,
            Statement::Return(return_stmt) => return_stmt.span,
            Statement::If(if_stmt) => if_stmt.span,
            Statement::For(for_stmt) => for_stmt.span,
            Statement::While(loop_stmt) => loop_stmt.span,
            Statement::Repeat(loop_stmt) => loop_stmt.span,
            Statement::Switch(switch_stmt) => switch_stmt.span,
            Statement::With(loop_stmt) => loop_stmt.span,
            Statement::TryCatch(try_catch_stmt) => try_catch_stmt.span,
            Statement::Throw(throw) => throw.span,
            Statement::Call(call) => call.span,
            Statement::Prefix(mutation) => mutation.span,
            Statement::Postfix(mutation) => mutation.span,
            Statement::Exit(span) => *span,
            Statement::Break(span) => *span,
            Statement::Continue(span) => *span,
        }
    }
}

impl<S> Walk<S> for Statement<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Statement::Block(block) => block.walk(visitor),
            Statement::Enum(enum_) => enum_.walk(visitor),
            Statement::Function(func_stmt) => func_stmt.walk(visitor),
            Statement::Closure(closure_stmt) => closure_stmt.walk(visitor),
            Statement::Var(decl_stmt) => decl_stmt.walk(visitor),
            Statement::Static(decl_stmt) => decl_stmt.walk(visitor),
            Statement::Let(let_stmt) => let_stmt.walk(visitor),
            Statement::StaticLet(let_stmt) => let_stmt.walk(visitor),
            Statement::Assignment(assignment_stmt) => assignment_stmt.walk(visitor),
            Statement::Return(ret_stmt) => ret_stmt.walk(visitor),
            Statement::If(if_stmt) => if_stmt.walk(visitor),
            Statement::For(for_stmt) => for_stmt.walk(visitor),
            Statement::While(while_stmt) => while_stmt.walk(visitor),
            Statement::Repeat(repeat_stmt) => repeat_stmt.walk(visitor),
            Statement::Switch(switch_stmt) => switch_stmt.walk(visitor),
            Statement::With(with_stmt) => with_stmt.walk(visitor),
            Statement::TryCatch(try_catch_stmt) => try_catch_stmt.walk(visitor),
            Statement::Throw(throw_stmt) => throw_stmt.walk(visitor),
            Statement::Call(call_expr) => call_expr.walk(visitor),
            Statement::Prefix(mutation) => mutation.walk(visitor),
            Statement::Postfix(mutation) => mutation.walk(visitor),
            Statement::GlobalVar(_)
            | Statement::Empty(_)
            | Statement::Exit(_)
            | Statement::Break(_)
            | Statement::Continue(_) => ControlFlow::Continue(()),
        }
    }
}

impl<S> WalkMut<S> for Statement<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Statement::Block(block) => block.walk_mut(visitor),
            Statement::Enum(enum_) => enum_.walk_mut(visitor),
            Statement::Function(func_stmt) => func_stmt.walk_mut(visitor),
            Statement::Closure(closure_stmt) => closure_stmt.walk_mut(visitor),
            Statement::Var(var_stmt) => var_stmt.walk_mut(visitor),
            Statement::Static(decl_stmt) => decl_stmt.walk_mut(visitor),
            Statement::Let(let_stmt) => let_stmt.walk_mut(visitor),
            Statement::StaticLet(let_stmt) => let_stmt.walk_mut(visitor),
            Statement::Assignment(assignment_stmt) => assignment_stmt.walk_mut(visitor),
            Statement::Return(ret_stmt) => ret_stmt.walk_mut(visitor),
            Statement::If(if_stmt) => if_stmt.walk_mut(visitor),
            Statement::For(for_stmt) => for_stmt.walk_mut(visitor),
            Statement::While(while_stmt) => while_stmt.walk_mut(visitor),
            Statement::Repeat(repeat_stmt) => repeat_stmt.walk_mut(visitor),
            Statement::Switch(switch_stmt) => switch_stmt.walk_mut(visitor),
            Statement::With(with_stmt) => with_stmt.walk_mut(visitor),
            Statement::TryCatch(try_catch_stmt) => try_catch_stmt.walk_mut(visitor),
            Statement::Throw(throw_stmt) => throw_stmt.walk_mut(visitor),
            Statement::Call(call_expr) => call_expr.walk_mut(visitor),
            Statement::Prefix(mutation) => mutation.walk_mut(visitor),
            Statement::Postfix(mutation) => mutation.walk_mut(visitor),
            Statement::GlobalVar(_)
            | Statement::Empty(_)
            | Statement::Exit(_)
            | Statement::Break(_)
            | Statement::Continue(_) => ControlFlow::Continue(()),
        }
    }
}

impl<S> Walk<S> for BlockStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.block.walk(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for BlockStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.block.walk_mut(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for EnumStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for (_, expr) in &self.variants {
            if let Some(expr) = expr {
                visitor.visit_expr(expr)?;
            }
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for EnumStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for (_, expr) in &mut self.variants {
            if let Some(expr) = expr {
                visitor.visit_expr_mut(expr)?;
            }
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for FunctionStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        if let Some(call) = &self.inherit {
            call.walk(visitor)?;
        }
        self.parameters.walk(visitor)?;
        self.body.walk(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for FunctionStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        if let Some(call) = &mut self.inherit {
            call.walk_mut(visitor)?;
        }
        self.parameters.walk_mut(visitor)?;
        self.body.walk_mut(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ClosureStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.parameters.walk(visitor)?;
        self.body.walk(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ClosureStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.parameters.walk_mut(visitor)?;
        self.body.walk_mut(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for VarDeclarationStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for var in &self.vars {
            if let Some(value) = &var.1 {
                visitor.visit_expr(value)?;
            }
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for VarDeclarationStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for var in &mut self.vars {
            if let Some(value) = &mut var.1 {
                visitor.visit_expr_mut(value)?;
            }
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for LetDeclarationStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for expr in &self.exprs {
            visitor.visit_expr(expr)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for LetDeclarationStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for expr in &mut self.exprs {
            visitor.visit_expr_mut(expr)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for AssignmentStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.target.walk(visitor)?;
        visitor.visit_expr(&self.value)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for AssignmentStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.target.walk_mut(visitor)?;
        visitor.visit_expr_mut(&mut self.value)?;
        ControlFlow::Continue(())
    }
}

impl<S> MutableExpr<S> {
    pub fn span(&self) -> Span {
        match self {
            MutableExpr::Ident(ident) => ident.span,
            MutableExpr::Field(field_expr) => field_expr.span,
            MutableExpr::Index(index_expr) => index_expr.span,
        }
    }
}

impl<S> Walk<S> for MutableExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            MutableExpr::Ident(_) => {}
            MutableExpr::Field(field_expr) => {
                visitor.visit_expr(&field_expr.base)?;
            }
            MutableExpr::Index(index_expr) => {
                index_expr.walk(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for MutableExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            MutableExpr::Ident(_) => {}
            MutableExpr::Field(field_expr) => {
                visitor.visit_expr_mut(&mut field_expr.base)?;
            }
            MutableExpr::Index(index_expr) => {
                index_expr.walk_mut(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ReturnStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for val in &self.values {
            visitor.visit_expr(val)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ReturnStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for val in &mut self.values {
            visitor.visit_expr_mut(val)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for IfStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.condition)?;
        visitor.visit_stmt(&self.then_stmt)?;
        if let Some(else_stmt) = &self.else_stmt {
            visitor.visit_stmt(else_stmt)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for IfStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.condition)?;
        visitor.visit_stmt_mut(&mut self.then_stmt)?;
        if let Some(else_stmt) = &mut self.else_stmt {
            visitor.visit_stmt_mut(else_stmt)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ForStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_stmt(&self.initializer)?;
        visitor.visit_expr(&self.condition)?;
        visitor.visit_stmt(&self.iterator)?;
        visitor.visit_stmt(&self.body)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ForStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_stmt_mut(&mut self.initializer)?;
        visitor.visit_expr_mut(&mut self.condition)?;
        visitor.visit_stmt_mut(&mut self.iterator)?;
        visitor.visit_stmt_mut(&mut self.body)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for LoopStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.target)?;
        visitor.visit_stmt(&self.body)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for LoopStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.target)?;
        visitor.visit_stmt_mut(&mut self.body)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for TryCatchStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_stmt(&self.try_block)?;
        visitor.visit_stmt(&self.catch_block)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for TryCatchStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_stmt_mut(&mut self.try_block)?;
        visitor.visit_stmt_mut(&mut self.catch_block)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ThrowStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.target)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ThrowStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.target)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for SwitchStmt<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.target)?;
        for case in &self.cases {
            case.walk(visitor)?;
        }
        if let Some(default_block) = &self.default {
            default_block.walk(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for SwitchStmt<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.target)?;
        for case in &mut self.cases {
            case.walk_mut(visitor)?;
        }
        if let Some(default_block) = &mut self.default {
            default_block.walk_mut(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for SwitchCase<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.compare)?;
        self.body.walk(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for SwitchCase<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.compare)?;
        self.body.walk_mut(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> Expression<S> {
    pub fn span(&self) -> Span {
        match self {
            Expression::Global(span) => *span,
            Expression::This(span) => *span,
            Expression::Other(span) => *span,
            Expression::Constant(_, span) => *span,
            Expression::Ident(ident) => ident.span,
            Expression::Group(group_expr) => group_expr.span,
            Expression::Object(object_expr) => object_expr.span,
            Expression::Array(array_expr) => array_expr.span,
            Expression::Unary(unary_expr) => unary_expr.span,
            Expression::Prefix(mutation) => mutation.span,
            Expression::Postfix(mutation) => mutation.span,
            Expression::Binary(binary_expr) => binary_expr.span,
            Expression::Ternary(ternary_expr) => ternary_expr.span,
            Expression::Function(function_expr) => function_expr.span,
            Expression::Closure(closure_expr) => closure_expr.span,
            Expression::Call(call) => call.span,
            Expression::Field(field_expr) => field_expr.span,
            Expression::Index(index_expr) => index_expr.span,
            Expression::VarArgs(span) => *span,
            Expression::Argument(arg_expr) => arg_expr.span,
            Expression::ArgumentCount(span) => *span,
        }
    }
}

impl<S: Eq + Clone> Expression<S> {
    pub fn fold_constant(&self) -> Option<Constant<S>> {
        match self {
            Expression::Constant(c, _) => Some(c.clone()),
            Expression::Group(expr) => expr.fold_constant(),
            Expression::Unary(expr) => expr.fold_constant(),
            Expression::Binary(expr) => expr.fold_constant(),
            Expression::Ternary(expr) => expr.fold_constant(),
            _ => None,
        }
    }
}

impl<S> Walk<S> for Expression<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Expression::Group(group_expr) => group_expr.walk(visitor),
            Expression::Object(object_expr) => object_expr.walk(visitor),
            Expression::Array(array_expr) => array_expr.walk(visitor),
            Expression::Unary(unary_expr) => unary_expr.walk(visitor),
            Expression::Prefix(mutation) => mutation.walk(visitor),
            Expression::Postfix(mutation) => mutation.walk(visitor),
            Expression::Binary(bin_expr) => bin_expr.walk(visitor),
            Expression::Ternary(tern_expr) => tern_expr.walk(visitor),
            Expression::Function(func_expr) => func_expr.walk(visitor),
            Expression::Closure(closure_expr) => closure_expr.walk(visitor),
            Expression::Call(call_expr) => call_expr.walk(visitor),
            Expression::Field(field_expr) => field_expr.walk(visitor),
            Expression::Index(index_expr) => index_expr.walk(visitor),
            Expression::Argument(arg_expr) => arg_expr.walk(visitor),
            Expression::Ident(_)
            | Expression::Global(_)
            | Expression::This(_)
            | Expression::Other(_)
            | Expression::Constant(..)
            | Expression::VarArgs(..)
            | Expression::ArgumentCount(_) => ControlFlow::Continue(()),
        }
    }
}

impl<S> WalkMut<S> for Expression<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Expression::Group(group_expr) => group_expr.walk_mut(visitor),
            Expression::Object(object_expr) => object_expr.walk_mut(visitor),
            Expression::Array(array_expr) => array_expr.walk_mut(visitor),
            Expression::Unary(unary_expr) => unary_expr.walk_mut(visitor),
            Expression::Prefix(mutation) => mutation.walk_mut(visitor),
            Expression::Postfix(mutation) => mutation.walk_mut(visitor),
            Expression::Binary(bin_expr) => bin_expr.walk_mut(visitor),
            Expression::Ternary(tern_expr) => tern_expr.walk_mut(visitor),
            Expression::Function(func_expr) => func_expr.walk_mut(visitor),
            Expression::Closure(closure_expr) => closure_expr.walk_mut(visitor),
            Expression::Call(call_expr) => call_expr.walk_mut(visitor),
            Expression::Field(field_expr) => field_expr.walk_mut(visitor),
            Expression::Index(index_expr) => index_expr.walk_mut(visitor),
            Expression::Argument(arg_expr) => arg_expr.walk_mut(visitor),
            Expression::Ident(_)
            | Expression::Global(_)
            | Expression::This(_)
            | Expression::Other(_)
            | Expression::Constant(..)
            | Expression::VarArgs(..)
            | Expression::ArgumentCount(_) => ControlFlow::Continue(()),
        }
    }
}

impl<S: Eq + Clone> GroupExpr<S> {
    pub fn fold_constant(&self) -> Option<Constant<S>> {
        self.inner.fold_constant()
    }
}

impl<S> Walk<S> for GroupExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.inner)
    }
}

impl<S> WalkMut<S> for GroupExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.inner)
    }
}

impl<S> Walk<S> for ObjectExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for field in &self.fields {
            field.walk(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ObjectExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for field in &mut self.fields {
            field.walk_mut(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ArrayExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for entry in &self.entries {
            visitor.visit_expr(entry)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ArrayExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for entry in &mut self.entries {
            visitor.visit_expr_mut(entry)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S: Eq + Clone> UnaryExpr<S> {
    pub fn fold_constant(&self) -> Option<Constant<S>> {
        match self.op {
            UnaryOp::Not => Some(Constant::Boolean(!self.target.fold_constant()?.cast_bool())),
            UnaryOp::Minus => self.target.fold_constant()?.negate(),
            UnaryOp::BitNegate => Some(Constant::Integer(
                self.target.fold_constant()?.bit_negate()?,
            )),
        }
    }
}

impl<S> Walk<S> for UnaryExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.target)
    }
}

impl<S> WalkMut<S> for UnaryExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.target)
    }
}

impl<S> Walk<S> for Mutation<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.target.walk(visitor)
    }
}

impl<S> WalkMut<S> for Mutation<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.target.walk_mut(visitor)
    }
}

impl<S: Eq + Clone> BinaryExpr<S> {
    pub fn fold_constant(&self) -> Option<Constant<S>> {
        let left = {
            match &*self.left {
                Expression::Constant(c, _) => c,
                _ => &self.left.fold_constant()?,
            }
        };

        let right = {
            match &*self.right {
                Expression::Constant(c, _) => c,
                _ => &self.right.fold_constant()?,
            }
        };

        match self.op {
            BinaryOp::Add => left.add(right),
            BinaryOp::Sub => left.sub(right),
            BinaryOp::Mult => left.mult(right),
            BinaryOp::Div => left.div(right),
            BinaryOp::Mod => left.rem(right),
            BinaryOp::Rem => left.rem(right),
            BinaryOp::IDiv => left.idiv(right).map(Constant::Integer),
            BinaryOp::Equal => Some(Constant::Boolean(left.equal(right))),
            BinaryOp::NotEqual => Some(Constant::Boolean(!left.equal(right))),
            BinaryOp::LessThan => left.less_than(right).map(Constant::Boolean),
            BinaryOp::LessEqual => left.less_equal(right).map(Constant::Boolean),
            BinaryOp::GreaterThan => right.less_than(left).map(Constant::Boolean),
            BinaryOp::GreaterEqual => right.less_equal(left).map(Constant::Boolean),
            BinaryOp::And => Some(Constant::Boolean(left.and(right))),
            BinaryOp::Or => Some(Constant::Boolean(left.or(right))),
            BinaryOp::Xor => Some(Constant::Boolean(left.xor(right))),
            BinaryOp::BitAnd => left.bit_and(right).map(Constant::Integer),
            BinaryOp::BitOr => left.bit_or(right).map(Constant::Integer),
            BinaryOp::BitXor => left.bit_xor(right).map(Constant::Integer),
            BinaryOp::BitShiftLeft => left.bit_shift_left(right).map(Constant::Integer),
            BinaryOp::BitShiftRight => left.bit_shift_right(right).map(Constant::Integer),
            BinaryOp::NullCoalesce => Some(left.null_coalesce(right).clone()),
        }
    }
}

impl<S> Walk<S> for BinaryExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.left)?;
        visitor.visit_expr(&self.right)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for BinaryExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.left)?;
        visitor.visit_expr_mut(&mut self.right)?;
        ControlFlow::Continue(())
    }
}

impl<S: Eq + Clone> TernaryExpr<S> {
    pub fn fold_constant(&self) -> Option<Constant<S>> {
        let cond = self.cond.fold_constant()?;
        if cond.cast_bool() {
            self.if_true.fold_constant()
        } else {
            self.if_false.fold_constant()
        }
    }
}

impl<S> Walk<S> for TernaryExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.cond)?;
        visitor.visit_expr(&self.if_true)?;
        visitor.visit_expr(&self.if_false)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for TernaryExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.cond)?;
        visitor.visit_expr_mut(&mut self.if_true)?;
        visitor.visit_expr_mut(&mut self.if_false)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for FunctionExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        if let Some(call) = &self.inherit {
            call.walk(visitor)?;
        }
        self.parameters.walk(visitor)?;
        self.body.walk(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for FunctionExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        if let Some(call) = &mut self.inherit {
            call.walk_mut(visitor)?;
        }
        self.parameters.walk_mut(visitor)?;
        self.body.walk_mut(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ClosureExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.parameters.walk(visitor)?;
        self.body.walk(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ClosureExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.parameters.walk_mut(visitor)?;
        self.body.walk_mut(visitor)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for FieldExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.base)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for FieldExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.base)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for IndexExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.base)?;
        for expr in &self.indexes {
            visitor.visit_expr(expr)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for IndexExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.base)?;
        for expr in &mut self.indexes {
            visitor.visit_expr_mut(expr)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ArgumentExpr<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.arg_index)?;
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ArgumentExpr<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.arg_index)?;
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for Call<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr(&self.base)?;
        for arg in &self.arguments {
            visitor.visit_expr(arg)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for Call<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_expr_mut(&mut self.base)?;
        for arg in &mut self.arguments {
            visitor.visit_expr_mut(arg)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for Parameter<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        if let Some(default) = &self.default {
            visitor.visit_expr(default)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for Parameter<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        if let Some(default) = &mut self.default {
            visitor.visit_expr_mut(default)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Walk<S> for ParameterList<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        for parameter in &self.fixed {
            parameter.walk(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for ParameterList<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        for parameter in &mut self.fixed {
            parameter.walk_mut(visitor)?;
        }
        ControlFlow::Continue(())
    }
}

impl<S> Field<S> {
    pub fn span(&self) -> Span {
        match self {
            Field::Value(ident, expression) => ident.span.combine(expression.span()),
            Field::Init(ident) => ident.span,
        }
    }
}

impl<S> Walk<S> for Field<S> {
    fn walk<V: Visitor<S>>(&self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Field::Value(_, expr) => visitor.visit_expr(expr)?,
            Field::Init(_) => {}
        }
        ControlFlow::Continue(())
    }
}

impl<S> WalkMut<S> for Field<S> {
    fn walk_mut<V: VisitorMut<S>>(&mut self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Field::Value(_, expr) => visitor.visit_expr_mut(expr)?,
            Field::Init(_) => {}
        }
        ControlFlow::Continue(())
    }
}
