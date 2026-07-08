use std::{array, collections::hash_map, hash::Hash};

use fabricator_vm::{BuiltIns, FunctionRef, SharedStr, Span};
use rustc_hash::FxHashMap;
use thiserror::Error;

use crate::{ast, constant::Constant, ir, string_interner::StringInterner};

pub enum FreeVarMode {
    /// Free variable name is interpreted as an accessor to the implicit `self`.
    ///
    /// This should be the default.
    This,
    /// Free variable name is always interpreted as a global variable.
    GlobalVar,
    /// Free variable name is a magic variable.
    Magic {
        /// If true, then it is only permitted to read from this magic variable.
        is_read_only: bool,
    },
}

pub trait VarDict<S> {
    /// Should return false if ir-gen should permit this name for a variable or parameter
    /// declaration.
    ///
    /// Return false for names which have other meanings which should not be allowed to be shadowed.
    fn is_reserved(&self, name: &S) -> bool;

    /// Return the type of free variable for the given identifier.
    fn free_var_mode(&self, ident: &S) -> FreeVarMode;
}

#[derive(Debug, Error)]
pub enum IrGenErrorKind {
    #[error("export statements are only allowed at the top-level")]
    MisplacedExport,
    #[error("function scope variable declaration not permitted")]
    FunctionScopeVarNotAllowed,
    #[error("non-closure-functions are not permitted")]
    NonClosureFunctionsNotAllowed,
    #[error("constructor functions are not permitted")]
    ConstructorsNotAllowed,
    #[error("try / catch blocks are not permitted")]
    TryCatchNotAllowed,
    #[error("free variables as implicit self are not permitted")]
    ImplicitSelfNotAllowed,
    #[error("function-scope variables are not permitted to shadow block-scope variables")]
    FunctionScopeCannotShadowBlockScope,
    #[error(
        "function-scope variable is re-declared with a different kind or has a kind which cannot be re-declared"
    )]
    BadFunctionScopeVarRedeclaration,
    #[error("variable declaration uses a reserved name")]
    DeclaredNameIsReserved,
    #[error("assignment to read-only magic value")]
    ReadOnlyMagic,
    #[error("static variables in constructors must be at the top-level of the function block")]
    ConstructorStaticNotTopLevel,
    #[error("static variables in constructors must be initialized")]
    ConstructorStaticNotInitialized,
    #[error("function not allowed to have a return statement with an argument")]
    CannotReturnValue,
    #[error("break statement with no target")]
    BreakWithNoTarget,
    #[error("continue statement with no target")]
    ContinueWithNoTarget,
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(&'static str),
}

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct IrGenError {
    #[source]
    pub kind: IrGenErrorKind,
    pub span: Span,
}

#[derive(Debug, Copy, Clone)]
pub struct IrGenSettings {
    /// Allow `var` and `static` variable declarations, which do not use function scoping.
    ///
    /// # Block scoping and closures
    ///
    /// Closing over a variable which is declared in the body of a loop will act differently
    /// depending on whether it is declared using block scoping or function scoping. With block
    /// scoping, each variable in a loop iteration is independent, without it, every variable in
    /// the body of a loop is always the same instance. This matches the behavior of ECMAScript
    /// variables declared with `let` vs `var`.
    ///
    /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide/Closures#creating_closures_in_loops_a_common_mistake>
    pub allow_function_scope_vars: bool,

    /// Allow `function` style function expressions and function statements.
    ///
    /// Function statements both create a named variable *and* magically insert a named value into
    /// `self`, which is confusing.
    ///
    /// Function style expressions do not close over surrounding variables and automatically bind
    /// their surrounding `self` value, which is also confusing.
    ///
    /// Both of these can be accomplished with `closure`, which has neither of these behaviors and
    /// is much easier to reason about.
    ///
    /// This rule does not affect top-level function export statements, which are always allowed.
    pub allow_non_closure_functions: bool,

    /// Allow `constructor` annotated function statements with optional inheritance.
    ///
    /// These desugar to the equivalent manual implementation creating a new `self` object and a
    /// static variable holding the shared super object.
    ///
    /// All `static` variables must be declared at the top level of the function with unique names,
    /// and are interpreted as fields on a shared super object. Declaring a `constructor` function
    /// *disables* normal function statics, and all statics not at the top-level trigger errors.
    /// Though these variables are synthetic, they may be referenced as real variables, but they
    /// desugar to accessing a named variable in the super table.
    ///
    /// This option is included for compatibility purposes only, it is more straightforward to
    /// handle constructors and inheritance manually, and doing so does not disable normal static
    /// variables.
    pub allow_constructors: bool,

    /// Allow `try {} catch(e) {}` blocks.
    ///
    /// These desugar to the equivalent of creating an inner closure and calling it with `pcall`.
    /// Inside the `try` block, the difference between a normal exit, a thrown error, and `break` /
    /// `continue` / `return` are handled by setting a flag which is checked at `pcall` exit.
    pub allow_try_catch_blocks: bool,

    /// Allow free variables, if they are not imports from somewhere else, to implicitly refer to
    /// `self.{var}`.
    pub allow_implicit_self: bool,
}

impl IrGenSettings {
    pub fn strict() -> Self {
        IrGenSettings {
            allow_function_scope_vars: false,
            allow_non_closure_functions: false,
            allow_constructors: false,
            allow_try_catch_blocks: false,
            allow_implicit_self: false,
        }
    }

    pub fn compat() -> Self {
        IrGenSettings {
            allow_function_scope_vars: true,
            allow_non_closure_functions: true,
            allow_constructors: true,
            allow_try_catch_blocks: true,
            allow_implicit_self: true,
        }
    }

    pub fn gen_chunk_ir<S>(
        self,
        interner: &mut dyn StringInterner<String = S>,
        block: &ast::Block<S>,
        var_dict: &dyn VarDict<S>,
    ) -> Result<ir::Function<S>, IrGenError>
    where
        S: Eq + Hash + Clone + AsRef<str>,
    {
        let mut compiler = FunctionCompiler::new(self, interner, FunctionRef::Chunk, var_dict);
        compiler.block(block)?;
        Ok(compiler.finish())
    }

    pub fn gen_func_stmt_ir<S>(
        self,
        interner: &mut dyn StringInterner<String = S>,
        func_stmt: &ast::FunctionStmt<S>,
        var_dict: &dyn VarDict<S>,
    ) -> Result<ir::Function<S>, IrGenError>
    where
        S: Eq + Hash + Clone + AsRef<str>,
    {
        let mut compiler = FunctionCompiler::new(
            self,
            interner,
            FunctionRef::Named(SharedStr::new(func_stmt.name.as_ref()), func_stmt.span),
            var_dict,
        );
        compiler.declare_parameters(&func_stmt.parameters)?;
        if func_stmt.is_constructor {
            if !self.allow_constructors {
                return Err(IrGenError {
                    kind: IrGenErrorKind::ConstructorsNotAllowed,
                    span: func_stmt.span,
                });
            }
            compiler.constructor(func_stmt.inherit.as_ref(), &func_stmt.body)
        } else {
            compiler.block(&func_stmt.body)?;
            Ok(compiler.finish())
        }
    }
}

struct FunctionCompiler<'a, S> {
    settings: IrGenSettings,
    interner: &'a mut dyn StringInterner<String = S>,
    var_dict: &'a dyn VarDict<S>,

    function: ir::Function<S>,

    func_type: FunctionType,

    /// This will be `Some` if the current block is unfinished and can be appended to.
    current_block: Option<ir::BlockId>,

    break_target_stack: Vec<NonLocalJump>,
    continue_target_stack: Vec<NonLocalJump>,
    function_bind_mode: Vec<FunctionBindMode>,

    /// Function-scope variable declarations.
    function_scope_vars: FxHashMap<ast::Ident<S>, VariableType<S>>,

    /// Block scopes containing block variable declarations.
    block_scopes: Vec<BlockScope<S>>,

    /// Maps in-scope block-scope variable names to a list of indexes for block scopes which contain
    /// variables with this name.
    ///
    /// This list is always kept in block scope stack order, so the top entry in the list is always
    /// the variable currently visible for this name.
    block_variable_lookup: FxHashMap<ast::Ident<S>, Vec<usize>>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum FunctionBindMode {
    /// Functions bind the ambient `self` and `other`.
    BindDefault,
    /// For any created functions, open a temporary "this" scope to bind the given `self` object for
    /// that constructor only. This is used inside struct literals.
    BindNewThis(ir::InstId),
    /// Make all functions unbound. This is the default in static initializers.
    BindNothing,
}

#[derive(Debug, Copy, Clone)]
enum TryCatchExitCode {
    /// Return a value from the outer function.
    Return = 0,
    /// Break an outer loop.
    Break = 1,
    /// Continue an outer loop.
    Continue = 2,
}

#[derive(Debug, Copy, Clone)]
enum FunctionType {
    /// The function is a normal function.
    Normal,
    /// All function returns, including the end of the function, implicitly return the `self` value.
    /// No function return may return an explicit value.
    Constructor {
        this: ir::InstId,
        parent: ir::InstId,
        parent_var: ir::VarId,
    },
    /// The function is a desugared `try {} catch(e) {}` block.
    ///
    /// The function must set a special variable to a value of `TryCatchReturnCode` before
    /// returning.
    TryCatch {
        /// Allow top-level `break` within the function and desugar this into setting the `Break`
        /// return code followed by a return.
        allow_break: bool,
        /// Allow top-level `continue` within the function and desugar this into setting the
        /// `Continue` return code followed by a return.
        allow_continue: bool,
        /// Allow top-level `return` with an arguemnt within the function and desugar this into
        /// setting the `Return` return code followed by returning a value.
        allow_return_with_arg: bool,
        /// The variable which must hold the `TryCatchReturnCode`. If the variable is not set, then
        /// the outer function will always proceed with normal execution.
        exit_code: ir::VarId,
    },
}

/// Requested declaration type for a function-scope variable.
///
/// Function-scope variables can be:
///   1) Function Parameters
///   2) Variables declared with `var` or `static`.
///   3) Closed over variables of any type from upper functions.
#[derive(Debug, Clone)]
enum FunctionVarDecl<S> {
    /// This variable is a normal IR variable.
    Normal(ir::Variable<S>),
    /// This variable is a constructor static and is inside the `parent` object of a constructor
    /// under the stored field name.
    ConstructorStatic(S),
    /// This variable references a captured constructor static in a closure.
    UpperConstructorStatic { parent: ir::VarId, field: S },
}

impl<S> From<ir::Variable<S>> for FunctionVarDecl<S> {
    fn from(value: ir::Variable<S>) -> Self {
        Self::Normal(value)
    }
}

/// Declared variables can be normal IR variables or have special access modes.
#[derive(Debug, Clone)]
enum VariableType<S> {
    /// This is a normal IR variable.
    Normal(ir::VarId),
    /// This variable is an owned constructor static.
    ConstructorStatic(S),
    /// This variable references a captured constructor static in a closure.
    UpperConstructorStatic { parent: ir::VarId, field: S },
}

impl<S> From<ir::VarId> for VariableType<S> {
    fn from(var_id: ir::VarId) -> Self {
        VariableType::Normal(var_id)
    }
}

#[derive(Debug, Clone)]
enum FoundVariable<S> {
    /// This is a normal variable owned by this function.
    Owned {
        var_id: ir::VarId,
        is_static: bool,
        has_block_scope: bool,
    },
    /// This variable references a captured variable in a closure.
    Upper(ir::VarId),
    /// This variable is an owned constructor static.
    ConstructorStatic(S),
    /// This variable references a captured constructor static in a closure.
    UpperConstructorStatic { parent: ir::VarId, field: S },
}

impl<S> Into<VariableType<S>> for FoundVariable<S> {
    fn into(self) -> VariableType<S> {
        match self {
            FoundVariable::Owned { var_id, .. } => VariableType::Normal(var_id),
            FoundVariable::Upper(var_id) => VariableType::Normal(var_id),
            FoundVariable::ConstructorStatic(field) => VariableType::ConstructorStatic(field),
            FoundVariable::UpperConstructorStatic { parent, field } => {
                VariableType::UpperConstructorStatic { parent, field }
            }
        }
    }
}

struct BlockScope<S> {
    /// All variables to close when this block ends.
    to_close: Vec<ir::VarId>,

    /// All variable declarations for this scope which have not been shadowed. When a new
    /// declaration shadows another, the new declaration will replace the old in this map.
    visible: FxHashMap<ast::Ident<S>, ir::VarId>,
}

#[derive(Debug, Copy, Clone)]
struct NonLocalJump {
    target: ir::BlockId,
    // Stack index to which we are jumping. Non-local jumps need to close any variables between the
    // top-level scope and this.
    pop_vars_to: usize,
}

impl<S> Default for BlockScope<S> {
    fn default() -> Self {
        Self {
            to_close: Vec::new(),
            visible: FxHashMap::default(),
        }
    }
}

#[derive(Debug, Clone)]
enum MutableTarget<S> {
    Var(VariableType<S>),
    This {
        key: ir::InstId,
    },
    Globals {
        key: ir::InstId,
    },
    Field {
        target: ir::InstId,
        key: ir::InstId,
    },
    Index {
        target: ir::InstId,
        indexes: Vec<ir::InstId>,
    },
    Magic(S),
}

enum CallTarget {
    Function(ir::InstId),
    Method { func: ir::InstId, this: ir::InstId },
}

impl CallTarget {
    fn func(&self) -> ir::InstId {
        match *self {
            CallTarget::Function(func) => func,
            CallTarget::Method { func, .. } => func,
        }
    }

    fn this(&self) -> Option<ir::InstId> {
        match *self {
            CallTarget::Function(_) => None,
            CallTarget::Method { this: target, .. } => Some(target),
        }
    }
}

impl<'a, S> FunctionCompiler<'a, S>
where
    S: Eq + Hash + Clone + AsRef<str>,
{
    fn new(
        settings: IrGenSettings,
        interner: &'a mut dyn StringInterner<String = S>,
        reference: FunctionRef,
        var_dict: &'a dyn VarDict<S>,
    ) -> Self {
        let instructions = ir::InstructionMap::new();
        let mut blocks = ir::BlockMap::new();
        let variables = ir::VariableMap::new();
        let shadow_vars = ir::ShadowVarSet::new();
        let this_scopes = ir::ThisScopeSet::new();
        let call_scopes = ir::CallScopeSet::new();
        let functions = ir::FunctionMap::new();

        // We leave the start block completely empty except for a jump to the "real" start block.
        //
        // This is so that we can use the start block as a place to put function-scope variable
        // declaration instructions that need to be before any other generated IR, and avoid being
        // O(n^2) in the number of variable declarations.
        let start_block = blocks.insert(ir::Block::default());
        let first_block = blocks.insert(ir::Block::default());
        blocks[start_block].exit = ir::Exit {
            kind: ir::ExitKind::Jump(first_block),
            span: reference.span().start_span(),
        };

        let function = ir::Function {
            reference,
            num_parameters: 0,
            instructions,
            blocks,
            variables,
            shadow_vars,
            this_scopes,
            call_scopes,
            functions,
            start_block,
        };

        Self {
            settings,
            interner,
            var_dict,
            function,
            func_type: FunctionType::Normal,
            current_block: Some(first_block),
            continue_target_stack: Vec::new(),
            break_target_stack: Vec::new(),
            function_bind_mode: Vec::new(),
            function_scope_vars: FxHashMap::default(),
            block_scopes: Vec::new(),
            block_variable_lookup: FxHashMap::default(),
        }
    }

    fn declare_parameters(&mut self, parameters: &[ast::Parameter<S>]) -> Result<(), IrGenError> {
        self.function.num_parameters = parameters.len();

        for (param_index, param) in parameters.iter().enumerate() {
            let arg_var =
                self.declare_function_var(param.name.clone(), ir::Variable::Heap.into())?;

            let mut value =
                self.push_instruction(param.span, ir::InstructionKind::FixedArgument(param_index));

            if let Some(default) = &param.default {
                let cond = self.push_instruction(
                    param.span,
                    ir::InstructionKind::UnOp {
                        op: ir::UnOp::IsUndefined,
                        source: value,
                    },
                );

                value = self.if_expr(
                    param.span,
                    cond,
                    |this| this.expression(default),
                    |_| Ok(value),
                )?;
            };

            self.set_var(param.name.span, arg_var.into(), value);
        }

        Ok(())
    }

    fn constructor(
        mut self,
        inherit: Option<&ast::Call<S>>,
        body: &ast::Block<S>,
    ) -> Result<ir::Function<S>, IrGenError> {
        // We need the `init_constructor_super`, `get_constructor_super`, and `set_super`
        // intrinsics.

        let init_constructor_super_name = self.interner.intern(BuiltIns::INIT_CONSTRUCTOR_SUPER);
        let init_constructor_super = self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::GetMagic(init_constructor_super_name),
        );

        let get_constructor_super_name = self.interner.intern(BuiltIns::GET_CONSTRUCTOR_SUPER);
        let get_constructor_super = self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::GetMagic(get_constructor_super_name),
        );

        let set_super_name = self.interner.intern(BuiltIns::SET_SUPER);
        let set_super = self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::GetMagic(set_super_name),
        );

        // First, get this prototype's super object.

        let our_closure =
            self.push_instruction(body.span.start_span(), ir::InstructionKind::CurrentClosure);
        let [our_super] = self.call_function::<1>(
            body.span.start_span(),
            init_constructor_super,
            None,
            [our_closure],
        );

        let our_super_var = self.function.variables.insert(ir::Variable::Heap);
        self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::OpenVariable(our_super_var),
        );
        self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::SetVariable(our_super_var, our_super),
        );

        let inherit_func = if let Some(inherit) = inherit {
            Some(self.expression(&inherit.base)?)
        } else {
            None
        };

        // Create a new `self` object, if we are inheriting from another constructor, then use that
        // constructor's return value as the `self`.

        let this = if let Some(inherit_func) = inherit_func {
            let Some(inherit) = inherit else {
                unreachable!();
            };

            let mut args = Vec::new();
            for arg in &inherit.arguments {
                args.push(self.expression(arg)?);
            }

            let [ret] = self.call_function::<1>(inherit.span, inherit_func, None, args);
            ret
        } else {
            self.push_instruction(body.span.start_span(), ir::InstructionKind::NewObject)
        };

        // Set the super-table as the parent object for the `self` value.
        self.call_function::<0>(body.span.start_span(), set_super, None, [this, our_super]);

        // We must set this up early, because constructor statics may rely on each other, as long as
        // it is in-order.

        self.func_type = FunctionType::Constructor {
            this,
            parent: our_super,
            parent_var: our_super_var,
        };

        let init_block = self.new_block();
        let main_block = self.new_block();

        let our_super_is_initialized = self
            .function
            .variables
            .insert(ir::Variable::Static(Constant::Boolean(false)));

        let check_initialized = self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::GetVariable(our_super_is_initialized),
        );
        self.end_current_block(
            body.span.start_span(),
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(check_initialized),
                if_true: main_block,
                if_false: init_block,
            },
        );

        self.start_new_block(init_block);

        if inherit.is_some() {
            // We're explicitly allowing inheriting from a non-constructor here, if there is no
            // initialized inherited super object, then this constructor will not have a super-super
            // object.

            let [inherited_super] = self.call_function::<1>(
                body.span.start_span(),
                get_constructor_super,
                None,
                [inherit_func.unwrap()],
            );

            self.call_function::<0>(
                body.span.start_span(),
                set_super,
                None,
                [our_super, inherited_super],
            );
        }

        // All static initializers have a "this" scope of the current super object
        let super_scope = self.open_this_scope(body.span.start_span());
        self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::SetThis(super_scope, our_super),
        );

        // All function expressions within static initializers are *unbound*.
        self.push_function_bind_mode(FunctionBindMode::BindNothing);

        for stmt in &body.statements {
            match stmt {
                ast::Statement::Static(decls) => {
                    for (decl_name, decl_value) in &decls.vars {
                        let key = self.push_instruction(
                            decls.span,
                            ir::InstructionKind::Constant(Constant::String(
                                decl_name.inner.clone(),
                            )),
                        );
                        let value = self.expression(decl_value.as_ref().ok_or(IrGenError {
                            kind: IrGenErrorKind::ConstructorStaticNotInitialized,
                            span: decls.span,
                        })?)?;

                        self.push_instruction(
                            decls.span,
                            ir::InstructionKind::SetField {
                                target: our_super,
                                key,
                                value,
                            },
                        );

                        self.declare_function_var(
                            decl_name.clone(),
                            FunctionVarDecl::ConstructorStatic(decl_name.inner.clone()),
                        )?;
                    }
                }
                _ => {}
            }
        }

        self.pop_closure_bind_mode(FunctionBindMode::BindNothing);

        self.close_this_scope(body.span.start_span(), super_scope);

        let true_ = self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::Constant(Constant::Boolean(true)),
        );
        self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::SetVariable(our_super_is_initialized, true_),
        );

        self.end_current_block(body.span.start_span(), ir::ExitKind::Jump(main_block));

        self.start_new_block(main_block);

        // Our constructor body exists inside a "this" scope for the constructed object.
        let this_scope = self.open_this_scope(body.span.start_span());
        self.push_instruction(
            body.span.start_span(),
            ir::InstructionKind::SetThis(this_scope, this),
        );

        self.push_scope();

        for stmt in &body.statements {
            match stmt {
                ast::Statement::Static(_) => {
                    // We have already handled all static statements.
                }
                _ => {
                    self.statement(stmt)?;
                }
            }
        }

        self.pop_scope();

        let ret_scope = self.open_call_scope(body.span.end_span());
        self.push_stack_values(body.span.end_span(), ret_scope, [this]);
        self.end_current_block(
            body.span.end_span(),
            ir::ExitKind::Return {
                call_scope: ret_scope,
                stack_base: 0,
            },
        );

        Ok(self.finish())
    }

    fn finish(mut self) -> ir::Function<S> {
        self.end_current_block(
            self.function.reference.span().end_span(),
            ir::ExitKind::Exit,
        );

        assert!(self.break_target_stack.is_empty());
        assert!(self.continue_target_stack.is_empty());

        self.function
    }

    fn block(&mut self, block: &ast::Block<S>) -> Result<(), IrGenError> {
        self.push_scope();

        for statement in &block.statements {
            self.statement(statement)?;
        }

        self.pop_scope();
        Ok(())
    }

    fn statement(&mut self, statement: &ast::Statement<S>) -> Result<(), IrGenError> {
        match statement {
            ast::Statement::Empty(_) => Ok(()),
            ast::Statement::Block(block_stmt) => self.block(&block_stmt.block),
            ast::Statement::Enum(enum_stmt) => Err(IrGenError {
                kind: IrGenErrorKind::MisplacedExport,
                span: enum_stmt.span,
            }),
            ast::Statement::Function(func_stmt) => {
                if !self.settings.allow_non_closure_functions {
                    return Err(IrGenError {
                        kind: IrGenErrorKind::NonClosureFunctionsNotAllowed,
                        span: func_stmt.span,
                    });
                }

                let allow_constructors = self.settings.allow_constructors;
                let mut compiler = self.start_inner_function(
                    FunctionRef::Named(SharedStr::new(func_stmt.name.as_ref()), func_stmt.span),
                    false,
                );

                compiler.declare_parameters(&func_stmt.parameters)?;
                let function = if func_stmt.is_constructor {
                    if !allow_constructors {
                        return Err(IrGenError {
                            kind: IrGenErrorKind::ConstructorsNotAllowed,
                            span: func_stmt.span,
                        });
                    }
                    compiler.constructor(func_stmt.inherit.as_ref(), &func_stmt.body)?
                } else {
                    compiler.block(&func_stmt.body)?;
                    compiler.finish()
                };

                let func_id = self.function.functions.insert(function);
                let func = self.new_bound_function(func_stmt.span, func_id);

                // Function statements in GML both create a function-scope variable *and* insert a
                // named value into `self`.

                let var =
                    self.declare_function_var(func_stmt.name.clone(), ir::Variable::Heap.into())?;
                self.set_var(func_stmt.span, var.into(), func);

                let this = self.push_instruction(func_stmt.span, ir::InstructionKind::This);
                let key = self.push_instruction(
                    func_stmt.span,
                    ir::InstructionKind::Constant(Constant::String(func_stmt.name.inner.clone())),
                );
                self.push_instruction(
                    func_stmt.span,
                    ir::InstructionKind::SetField {
                        target: this,
                        key,
                        value: func,
                    },
                );
                Ok(())
            }
            ast::Statement::Closure(closure_stmt) => {
                let mut compiler = self.start_inner_function(
                    FunctionRef::Named(
                        SharedStr::new(closure_stmt.name.as_ref()),
                        closure_stmt.span,
                    ),
                    true,
                );

                compiler.declare_parameters(&closure_stmt.parameters)?;
                compiler.block(&closure_stmt.body)?;
                let function = compiler.finish();

                let func_id = self.function.functions.insert(function);
                let func = self.push_instruction(
                    closure_stmt.span,
                    ir::InstructionKind::Closure {
                        func: func_id,
                        bind_this: false,
                    },
                );

                let var = self
                    .declare_function_var(closure_stmt.name.clone(), ir::Variable::Heap.into())?;
                self.set_var(closure_stmt.span, var.into(), func);
                Ok(())
            }
            ast::Statement::Var(var_decls) => {
                if !self.settings.allow_function_scope_vars {
                    return Err(IrGenError {
                        kind: IrGenErrorKind::FunctionScopeVarNotAllowed,
                        span: var_decls.span,
                    });
                }

                for (name, value) in &var_decls.vars {
                    self.var_decl(name, value.as_ref())?;
                }
                Ok(())
            }
            ast::Statement::Static(static_decls) => {
                if matches!(self.func_type, FunctionType::Constructor { .. }) {
                    return Err(IrGenError {
                        kind: IrGenErrorKind::ConstructorStaticNotTopLevel,
                        span: static_decls.span,
                    });
                } else if !self.settings.allow_function_scope_vars {
                    return Err(IrGenError {
                        kind: IrGenErrorKind::FunctionScopeVarNotAllowed,
                        span: static_decls.span,
                    });
                }

                for (name, value) in &static_decls.vars {
                    self.static_decl(static_decls.span, name, value.as_ref())?;
                }
                Ok(())
            }
            ast::Statement::Let(let_decls) => self.let_decl(let_decls),
            ast::Statement::StaticLet(let_decls) => self.static_let_decl(let_decls),
            ast::Statement::GlobalVar(ident) => Err(IrGenError {
                kind: IrGenErrorKind::MisplacedExport,
                span: ident.span,
            }),
            ast::Statement::Assignment(assignment_statement) => {
                self.assignment_stmt(assignment_statement)
            }
            ast::Statement::Return(return_stmt) => {
                if return_stmt.values.is_empty() {
                    self.do_exit(return_stmt.span);
                } else {
                    let ret_scope =
                        self.open_call_arg_exprs(return_stmt.span, &return_stmt.values)?;
                    self.do_return(return_stmt.span, ret_scope, 0)?;
                }
                Ok(())
            }
            ast::Statement::If(if_stmt) => self.if_stmt(if_stmt),
            ast::Statement::For(for_stmt) => self.for_stmt(for_stmt),
            ast::Statement::While(while_stmt) => self.while_stmt(while_stmt),
            ast::Statement::Repeat(repeat_stmt) => self.repeat_stmt(repeat_stmt),
            ast::Statement::Switch(switch_stmt) => self.switch_stmt(switch_stmt),
            ast::Statement::With(with_stmt) => self.with_stmt(with_stmt),
            ast::Statement::TryCatch(try_catch_stmt) => self.try_catch_stmt(try_catch_stmt),
            ast::Statement::Throw(throw_stmt) => {
                let target = self.expression(&throw_stmt.target)?;
                self.end_current_block(throw_stmt.span, ir::ExitKind::Throw(target));
                Ok(())
            }
            ast::Statement::Call(function_call) => {
                let call_scope = self.open_call_expr(function_call)?;
                self.close_call_scope(function_call.span, call_scope);
                Ok(())
            }
            ast::Statement::Prefix(mutation) => {
                self.mutation_op(mutation)?;
                Ok(())
            }
            ast::Statement::Postfix(mutation) => {
                self.mutation_op(mutation)?;
                Ok(())
            }
            ast::Statement::Exit(span) => {
                self.do_exit(*span);
                Ok(())
            }
            ast::Statement::Break(span) => self.do_break(*span),
            ast::Statement::Continue(span) => self.do_continue(*span),
        }
    }

    fn var_decl(
        &mut self,
        name: &ast::Ident<S>,
        value: Option<&ast::Expression<S>>,
    ) -> Result<(), IrGenError> {
        let value = value
            .map(|value| Ok((self.expression(value)?, value.span())))
            .transpose()?;
        let var = self.declare_function_var(name.clone(), ir::Variable::Heap.into())?;
        if let Some((inst_id, span)) = value {
            self.set_var(span, var.into(), inst_id);
        }
        Ok(())
    }

    fn static_decl(
        &mut self,
        span: Span,
        name: &ast::Ident<S>,
        value: Option<&ast::Expression<S>>,
    ) -> Result<(), IrGenError> {
        if let Some(value) = value {
            if let Some(constant) = value.clone().fold_constant() {
                // If our static is a constant, then we can just initialize it when the prototype is
                // created.
                self.declare_function_var(name.clone(), ir::Variable::Static(constant).into())?;
            } else {
                // Otherwise, we need to initialize two static variables, a hidden one for the
                // initialization state and a visible one to hold the initialized value.

                // Create a hidden static variable to hold whether the real static is initialized.
                let is_initialized = self
                    .function
                    .variables
                    .insert(ir::Variable::Static(Constant::Boolean(false)));
                // Create a normal static variable that holds the real value.
                let var = self.declare_function_var(
                    name.clone(),
                    ir::Variable::Static(Constant::Undefined).into(),
                )?;

                let init_block = self.new_block();
                let successor = self.new_block();

                let check_initialized =
                    self.push_instruction(span, ir::InstructionKind::GetVariable(is_initialized));
                self.end_current_block(
                    span,
                    ir::ExitKind::Branch {
                        cond: ir::BranchCondition::IsTrue(check_initialized),
                        if_true: successor,
                        if_false: init_block,
                    },
                );

                self.start_new_block(init_block);

                let value = self.expression(value)?;
                self.set_var(span, var.into(), value);

                let true_ = self
                    .push_instruction(span, ir::InstructionKind::Constant(Constant::Boolean(true)));
                self.push_instruction(
                    span,
                    ir::InstructionKind::SetVariable(is_initialized, true_),
                );

                self.end_current_block(span, ir::ExitKind::Jump(successor));

                self.start_new_block(successor);
            }
        } else {
            // If our static has no value then it is just initialized as `Undefined`.
            self.declare_function_var(
                name.clone(),
                ir::Variable::Static(Constant::Undefined).into(),
            )?;
        }

        Ok(())
    }

    fn let_decl(&mut self, let_decl_stmt: &ast::LetDeclarationStmt<S>) -> Result<(), IrGenError> {
        // Declare all `let` variables, with special behavior for the final expression. If the final
        // expression is an expression with multiple return values, then set the remaining variables
        // to the corresponding return of the final evaluated expression.

        let mut let_vars = Vec::new();

        for vname in &let_decl_stmt.vars {
            let_vars.push((
                vname,
                self.open_owned_block_var(vname.span, ir::Variable::Heap),
            ));
        }

        for i in 0..let_decl_stmt.exprs.len() {
            match &let_decl_stmt.exprs[i] {
                ast::Expression::Call(call) if i + 1 == let_decl_stmt.exprs.len() => {
                    let call_scope = self.open_call_expr(call)?;
                    for j in i..let_vars.len() {
                        let (ident, var_id) = let_vars[j];
                        let value = self.push_instruction(
                            call.span,
                            ir::InstructionKind::GetStack(call_scope, j - i),
                        );
                        self.push_instruction(
                            ident.span,
                            ir::InstructionKind::SetVariable(var_id, value),
                        );
                    }
                    self.close_call_scope(call.span, call_scope);
                }
                expr => {
                    let value = self.expression(expr)?;
                    if let Some((ident, var_id)) = let_vars.get(i).copied() {
                        self.push_instruction(
                            ident.span,
                            ir::InstructionKind::SetVariable(var_id, value),
                        );
                    }
                }
            }
        }

        for (vname, var_id) in let_vars {
            self.declare_block_var(vname.clone(), var_id)?;
        }

        Ok(())
    }

    fn static_let_decl(
        &mut self,
        let_decl_stmt: &ast::LetDeclarationStmt<S>,
    ) -> Result<(), IrGenError> {
        let mut const_values = let_decl_stmt
            .exprs
            .iter()
            .map(|e| e.fold_constant())
            .collect::<Vec<_>>();

        if const_values.iter().all(|c| c.is_some()) {
            // If all of our static values are constant, we can just initialize all of them when the
            // prototype is created.

            for (i, vname) in let_decl_stmt.vars.iter().enumerate() {
                let var_id = self.open_owned_block_var(
                    vname.span,
                    ir::Variable::Static(const_values[i].take().unwrap()),
                );
                self.declare_block_var(vname.clone(), var_id)?;
            }
        } else {
            // Otherwise, we need to initialize an extra hidden static variable for the
            // initialization state. Since `let` variables are not in scope with each other in the
            // same declaration, we can use a single initialization variable for everything.

            // Create a hidden static variable to hold the initialization state.
            let is_initialized = self
                .function
                .variables
                .insert(ir::Variable::Static(Constant::Boolean(false)));

            let init_block = self.new_block();
            let successor = self.new_block();

            let check_initialized = self.push_instruction(
                let_decl_stmt.span,
                ir::InstructionKind::GetVariable(is_initialized),
            );
            self.end_current_block(
                let_decl_stmt.span,
                ir::ExitKind::Branch {
                    cond: ir::BranchCondition::IsTrue(check_initialized),
                    if_true: successor,
                    if_false: init_block,
                },
            );

            self.start_new_block(init_block);

            // Declare all `let` variables, with special behavior for the final expression. If the
            // final expression is an expression with multiple return values, then set the remaining
            // variables to the corresponding return of the final evaluated expression.

            let mut let_vars = Vec::new();

            for vname in &let_decl_stmt.vars {
                let_vars.push((
                    vname,
                    self.open_owned_block_var(
                        vname.span,
                        ir::Variable::Static(Constant::Undefined),
                    ),
                ));
            }

            for i in 0..let_decl_stmt.exprs.len() {
                match &let_decl_stmt.exprs[i] {
                    ast::Expression::Call(call) if i + 1 == let_decl_stmt.exprs.len() => {
                        let call_scope = self.open_call_expr(call)?;
                        for j in i..let_vars.len() {
                            let (ident, var_id) = let_vars[j];
                            let value = self.push_instruction(
                                call.span,
                                ir::InstructionKind::GetStack(call_scope, j - i),
                            );
                            self.push_instruction(
                                ident.span,
                                ir::InstructionKind::SetVariable(var_id, value),
                            );
                        }
                        self.close_call_scope(call.span, call_scope);
                    }
                    expr => {
                        let value = self.expression(expr)?;
                        if let Some((ident, var_id)) = let_vars.get(i).copied() {
                            self.push_instruction(
                                ident.span,
                                ir::InstructionKind::SetVariable(var_id, value),
                            );
                        }
                    }
                }
            }

            for (vname, var_id) in let_vars {
                self.declare_block_var(vname.clone(), var_id)?;
            }

            let true_ = self.push_instruction(
                let_decl_stmt.span,
                ir::InstructionKind::Constant(Constant::Boolean(true)),
            );
            self.push_instruction(
                let_decl_stmt.span,
                ir::InstructionKind::SetVariable(is_initialized, true_),
            );

            self.end_current_block(let_decl_stmt.span, ir::ExitKind::Jump(successor));

            self.start_new_block(successor);
        }

        Ok(())
    }

    fn assignment_stmt(&mut self, assign_stmt: &ast::AssignmentStmt<S>) -> Result<(), IrGenError> {
        let target = self.mutable_target(&assign_stmt.target)?;
        let val = self.expression(&assign_stmt.value)?;

        let assign = match assign_stmt.op {
            ast::AssignmentOp::Equal => val,
            ast::AssignmentOp::PlusEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::Add,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::MinusEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::Sub,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::MultEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::Mult,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::DivEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::Div,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::RemEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::Rem,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::BitAndEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::BitAnd,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::BitOrEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::BitOr,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::BitXorEqual => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::BitXor,
                        right: val,
                    },
                )
            }
            ast::AssignmentOp::NullCoalesce => {
                let prev = self.read_mutable_target(assign_stmt.span, target.clone());
                self.push_instruction(
                    assign_stmt.span,
                    ir::InstructionKind::BinOp {
                        left: prev,
                        op: ir::BinOp::NullCoalesce,
                        right: val,
                    },
                )
            }
        };

        self.write_mutable_target(assign_stmt.span, target, assign);

        Ok(())
    }

    fn if_stmt(&mut self, if_statement: &ast::IfStmt<S>) -> Result<(), IrGenError> {
        let cond = self.expression(&if_statement.condition)?;
        let then_block = self.new_block();
        let else_block = self.new_block();
        let successor = self.new_block();

        self.end_current_block(
            if_statement.condition.span(),
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(cond),
                if_true: then_block,
                if_false: else_block,
            },
        );

        self.start_new_block(then_block);
        self.push_scope();
        self.statement(&if_statement.then_stmt)?;
        self.end_current_block(
            if_statement.then_stmt.span().end_span(),
            ir::ExitKind::Jump(successor),
        );
        self.pop_scope();

        self.start_new_block(else_block);
        if let Some(else_stmt) = &if_statement.else_stmt {
            self.push_scope();
            self.statement(else_stmt)?;
            self.pop_scope();
        }
        self.end_current_block(if_statement.span.end_span(), ir::ExitKind::Jump(successor));

        self.start_new_block(successor);
        Ok(())
    }

    fn for_stmt(&mut self, for_statement: &ast::ForStmt<S>) -> Result<(), IrGenError> {
        self.push_scope();
        self.statement(&for_statement.initializer)?;

        let cond_block = self.new_block();
        let body_block = self.new_block();
        let iter_block = self.new_block();
        let successor_block = self.new_block();

        self.end_current_block(
            for_statement.span.start_span(),
            ir::ExitKind::Jump(cond_block),
        );
        self.start_new_block(cond_block);

        let cond = self.expression(&for_statement.condition)?;
        self.end_current_block(
            for_statement.condition.span(),
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(cond),
                if_true: body_block,
                if_false: successor_block,
            },
        );

        self.push_break_target(successor_block);
        self.push_continue_target(iter_block);

        self.start_new_block(body_block);
        self.push_scope();
        self.statement(&for_statement.body)?;
        self.pop_scope();

        self.pop_continue_target(iter_block);
        self.pop_break_target(successor_block);

        self.end_current_block(
            for_statement.body.span().end_span(),
            ir::ExitKind::Jump(iter_block),
        );
        self.start_new_block(iter_block);

        self.push_scope();
        self.statement(&for_statement.iterator)?;
        self.pop_scope();

        self.end_current_block(
            for_statement.iterator.span().end_span(),
            ir::ExitKind::Jump(cond_block),
        );

        self.start_new_block(successor_block);

        self.pop_scope();

        Ok(())
    }

    fn while_stmt(&mut self, while_stmt: &ast::LoopStmt<S>) -> Result<(), IrGenError> {
        let cond_block = self.new_block();
        let body_block = self.new_block();
        let successor_block = self.new_block();

        self.end_current_block(while_stmt.span.start_span(), ir::ExitKind::Jump(cond_block));
        self.start_new_block(cond_block);

        let cond = self.expression(&while_stmt.target)?;
        self.end_current_block(
            while_stmt.target.span(),
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(cond),
                if_true: body_block,
                if_false: successor_block,
            },
        );

        self.push_break_target(successor_block);
        self.push_continue_target(cond_block);

        self.start_new_block(body_block);
        self.push_scope();
        self.statement(&while_stmt.body)?;
        self.pop_scope();

        self.pop_continue_target(cond_block);
        self.pop_break_target(successor_block);

        self.end_current_block(
            while_stmt.body.span().end_span(),
            ir::ExitKind::Jump(cond_block),
        );

        self.start_new_block(successor_block);

        Ok(())
    }

    fn repeat_stmt(&mut self, repeat_stmt: &ast::LoopStmt<S>) -> Result<(), IrGenError> {
        let times = self.expression(&repeat_stmt.target)?;

        let dec_var = self.function.variables.insert(ir::Variable::Heap);
        self.push_instruction(repeat_stmt.span, ir::InstructionKind::OpenVariable(dec_var));
        self.push_instruction(
            repeat_stmt.span,
            ir::InstructionKind::SetVariable(dec_var, times),
        );

        let cond_block = self.new_block();
        let body_block = self.new_block();
        let successor_block = self.new_block();

        self.end_current_block(
            repeat_stmt.span.start_span(),
            ir::ExitKind::Jump(cond_block),
        );
        self.start_new_block(cond_block);

        let prev =
            self.push_instruction(repeat_stmt.span, ir::InstructionKind::GetVariable(dec_var));
        self.end_current_block(
            repeat_stmt.body.span().start_span(),
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(prev),
                if_true: body_block,
                if_false: successor_block,
            },
        );

        self.start_new_block(body_block);

        let dec = self.push_instruction(
            repeat_stmt.span,
            ir::InstructionKind::UnOp {
                op: ir::UnOp::Decrement,
                source: prev,
            },
        );
        self.push_instruction(
            repeat_stmt.span,
            ir::InstructionKind::SetVariable(dec_var, dec),
        );

        self.push_break_target(successor_block);
        self.push_continue_target(cond_block);

        self.push_scope();
        self.statement(&repeat_stmt.body)?;
        self.pop_scope();

        self.pop_continue_target(cond_block);
        self.pop_break_target(successor_block);

        self.end_current_block(
            repeat_stmt.body.span().end_span(),
            ir::ExitKind::Jump(cond_block),
        );

        self.start_new_block(successor_block);

        self.push_instruction(
            repeat_stmt.span,
            ir::InstructionKind::CloseVariable(dec_var),
        );

        Ok(())
    }

    fn switch_stmt(&mut self, switch_stmt: &ast::SwitchStmt<S>) -> Result<(), IrGenError> {
        let target = self.expression(&switch_stmt.target)?;

        let mut body_blocks = Vec::new();
        body_blocks.resize_with(switch_stmt.cases.len(), || self.new_block());

        let successor_block = self.new_block();
        self.push_break_target(successor_block);

        for (i, case) in switch_stmt.cases.iter().enumerate() {
            let compare = self.expression(&case.compare)?;

            let body_block = body_blocks[i];
            let next_block = self.new_block();
            self.end_current_block(
                case.span,
                ir::ExitKind::Branch {
                    cond: ir::BranchCondition::Equal(target, compare),
                    if_true: body_block,
                    if_false: next_block,
                },
            );

            self.start_new_block(body_block);
            self.block(&case.body)?;

            // Handle switch case fall-through, if there is a subsequent case (and there has been no
            // `break;`), then we jump to its body directly.
            //
            // Fall-through to the default case is handled by the jump to `next_block`.
            if i + 1 < body_blocks.len() {
                self.end_current_block(
                    case.body.span.end_span(),
                    ir::ExitKind::Jump(body_blocks[i + 1]),
                );
            } else {
                self.end_current_block(case.body.span.end_span(), ir::ExitKind::Jump(next_block));
            }

            self.start_new_block(next_block);
        }

        if let Some(default) = &switch_stmt.default {
            self.block(default)?;
        }

        self.pop_break_target(successor_block);

        self.end_current_block(
            switch_stmt.span.end_span(),
            ir::ExitKind::Jump(successor_block),
        );

        self.start_new_block(successor_block);

        Ok(())
    }

    fn with_stmt(&mut self, with_stmt: &ast::LoopStmt<S>) -> Result<(), IrGenError> {
        let target = self.expression(&with_stmt.target)?;

        let control_start_span = with_stmt.body.span().start_span();

        let control_var = self.function.variables.insert(ir::Variable::Heap);
        self.push_instruction(
            control_start_span,
            ir::InstructionKind::OpenVariable(control_var),
        );

        let with_loop_iter_name = self.interner.intern(BuiltIns::WITH_LOOP_ITER);
        let with_loop_iter = self.push_instruction(
            control_start_span,
            ir::InstructionKind::GetMagic(with_loop_iter_name),
        );

        // The iteration protocol is to call the iter init function and expect three returns: an
        // iterator function, a state value, and a control value.
        //
        // At the beginning of a loop, the iteration function is called with the state value and
        // current control value as parameters. The iteration function is expected to return the
        // new value for the control value followed by all of the iteration results for that loop.
        //
        // If the returned control value is `Value::Undefined`, then the loop immediately stops (and
        // all following return values are ignored).
        //
        // The iterator function will always be called at least once, even if the initial control
        // value is `Value::Undefined`.

        let [iter_fn, state, init_control] =
            self.call_function::<3>(control_start_span, with_loop_iter, None, [target]);

        self.push_instruction(
            control_start_span,
            ir::InstructionKind::SetVariable(control_var, init_control),
        );

        let this_scope = self.open_this_scope(control_start_span);

        let check_block = self.new_block();
        let body_block = self.new_block();
        let successor_block = self.new_block();

        self.end_current_block(control_start_span, ir::ExitKind::Jump(check_block));

        self.start_new_block(check_block);

        let cur_control = self.push_instruction(
            control_start_span,
            ir::InstructionKind::GetVariable(control_var),
        );

        let [next_control, iter_val] =
            self.call_function::<2>(control_start_span, iter_fn, None, [state, cur_control]);

        self.end_current_block(
            control_start_span,
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsUndefined(next_control),
                if_true: successor_block,
                if_false: body_block,
            },
        );

        self.start_new_block(body_block);

        self.push_instruction(
            control_start_span,
            ir::InstructionKind::SetVariable(control_var, next_control),
        );

        self.push_instruction(
            control_start_span,
            ir::InstructionKind::SetThis(this_scope, iter_val),
        );

        self.push_continue_target(check_block);
        self.push_break_target(successor_block);

        self.push_scope();
        self.statement(&with_stmt.body)?;
        self.pop_scope();

        self.pop_break_target(successor_block);
        self.pop_continue_target(check_block);

        let control_end_span = with_stmt.body.span().end_span();

        self.end_current_block(control_end_span, ir::ExitKind::Jump(check_block));

        self.start_new_block(successor_block);

        self.close_this_scope(control_end_span, this_scope);
        self.push_instruction(
            control_end_span,
            ir::InstructionKind::CloseVariable(control_var),
        );

        Ok(())
    }

    fn try_catch_stmt(&mut self, try_catch_stmt: &ast::TryCatchStmt<S>) -> Result<(), IrGenError> {
        if !self.settings.allow_try_catch_blocks {
            return Err(IrGenError {
                kind: IrGenErrorKind::TryCatchNotAllowed,
                span: try_catch_stmt.span,
            });
        }

        // Desugar try / catch statements as `pcall` around an inner closure.

        let allow_break = !self.break_target_stack.is_empty();
        let allow_continue = !self.continue_target_stack.is_empty();

        let closure_span = try_catch_stmt.try_block.span();

        let pcall_name = self.interner.intern(BuiltIns::PCALL);
        let pcall = self.push_instruction(closure_span, ir::InstructionKind::GetMagic(pcall_name));

        let exit_code_var = self.function.variables.insert(ir::Variable::Heap);
        self.push_instruction(
            closure_span,
            ir::InstructionKind::OpenVariable(exit_code_var),
        );

        let allow_return_with_arg = match self.func_type {
            FunctionType::Normal => true,
            FunctionType::Constructor { .. } => false,
            FunctionType::TryCatch {
                allow_return_with_arg,
                ..
            } => allow_return_with_arg,
        };

        let mut compiler = self.start_inner_function(FunctionRef::Expression(closure_span), true);
        let inner_exit_code_var = compiler
            .function
            .variables
            .insert(ir::Variable::Upper(exit_code_var));
        compiler.func_type = FunctionType::TryCatch {
            allow_break,
            allow_continue,
            allow_return_with_arg,
            exit_code: inner_exit_code_var,
        };
        compiler.statement(&try_catch_stmt.try_block)?;

        let function = compiler.finish();
        let func_id = self.function.functions.insert(function);

        let inner_closure = self.push_instruction(
            closure_span,
            ir::InstructionKind::Closure {
                func: func_id,
                bind_this: true,
            },
        );

        let call_scope = self.open_call_scope(closure_span);
        self.push_stack_values(closure_span, call_scope, [inner_closure]);
        self.push_instruction(
            closure_span,
            ir::InstructionKind::Call {
                scope: call_scope,
                stack_base: 0,
                func: pcall,
                this: None,
            },
        );
        let [success, maybe_err] = self.get_stack_values(closure_span, call_scope, 0);

        let success_block = self.new_block();
        let err_block = self.new_block();

        self.end_current_block(
            closure_span,
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(success),
                if_true: success_block,
                if_false: err_block,
            },
        );

        self.start_new_block(success_block);

        let exit_code = self.push_instruction(
            closure_span,
            ir::InstructionKind::GetVariable(exit_code_var),
        );
        self.push_instruction(
            closure_span,
            ir::InstructionKind::CloseVariable(exit_code_var),
        );

        let return_code = self.push_instruction(
            closure_span,
            ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Return as i64)),
        );

        let do_return = self.new_block();
        let no_return = self.new_block();
        self.end_current_block(
            closure_span,
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::Equal(exit_code, return_code),
                if_true: do_return,
                if_false: no_return,
            },
        );

        self.start_new_block(do_return);
        if allow_return_with_arg {
            // Return all remaining returns from the inner function.
            self.do_return(closure_span, call_scope, 1).unwrap();
        } else {
            // The inner function should have no returns anyway.
            self.do_exit(closure_span);
        }

        self.start_new_block(no_return);
        self.close_call_scope(closure_span, call_scope);

        if allow_break {
            let break_code = self.push_instruction(
                closure_span,
                ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Break as i64)),
            );

            let do_break = self.new_block();
            let no_break = self.new_block();
            self.end_current_block(
                closure_span,
                ir::ExitKind::Branch {
                    cond: ir::BranchCondition::Equal(exit_code, break_code),
                    if_true: do_break,
                    if_false: no_break,
                },
            );

            self.start_new_block(do_break);
            self.do_break(closure_span)?;

            self.start_new_block(no_break);
        }

        if allow_continue {
            let continue_code = self.push_instruction(
                closure_span,
                ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Continue as i64)),
            );

            let do_continue = self.new_block();
            let no_continue = self.new_block();
            self.end_current_block(
                closure_span,
                ir::ExitKind::Branch {
                    cond: ir::BranchCondition::Equal(exit_code, continue_code),
                    if_true: do_continue,
                    if_false: no_continue,
                },
            );

            self.start_new_block(do_continue);
            self.do_continue(closure_span)?;

            self.start_new_block(no_continue);
        }

        let successor_block = self.new_block();
        self.end_current_block(
            try_catch_stmt.try_block.span().end_span(),
            ir::ExitKind::Jump(successor_block),
        );

        self.start_new_block(err_block);
        self.close_call_scope(closure_span, call_scope);
        self.push_instruction(
            closure_span,
            ir::InstructionKind::CloseVariable(exit_code_var),
        );

        self.push_scope();

        let err_var = self.open_owned_block_var(try_catch_stmt.err_ident.span, ir::Variable::Heap);
        self.declare_block_var(try_catch_stmt.err_ident.clone(), err_var)?;
        self.set_var(try_catch_stmt.err_ident.span, err_var.into(), maybe_err);

        self.statement(&try_catch_stmt.catch_block)?;

        self.pop_scope();

        self.end_current_block(
            try_catch_stmt.catch_block.span().end_span(),
            ir::ExitKind::Jump(successor_block),
        );

        self.start_new_block(successor_block);

        Ok(())
    }

    /// Do a function return with some non-zero number of return values on the stack.
    fn do_return(
        &mut self,
        span: Span,
        ret_scope: ir::CallScope,
        stack_base: usize,
    ) -> Result<(), IrGenError> {
        let return_code = self.push_instruction(
            span,
            ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Return as i64)),
        );

        match self.func_type {
            FunctionType::Normal => {
                self.end_current_block(
                    span,
                    ir::ExitKind::Return {
                        call_scope: ret_scope,
                        stack_base,
                    },
                );
                Ok(())
            }
            FunctionType::TryCatch {
                allow_return_with_arg: true,
                exit_code,
                ..
            } => {
                self.push_instruction(
                    span,
                    ir::InstructionKind::SetVariable(exit_code, return_code),
                );
                self.end_current_block(
                    span,
                    ir::ExitKind::Return {
                        call_scope: ret_scope,
                        stack_base,
                    },
                );
                Ok(())
            }
            FunctionType::Constructor { .. }
            | FunctionType::TryCatch {
                allow_return_with_arg: false,
                ..
            } => Err(IrGenError {
                kind: IrGenErrorKind::CannotReturnValue,
                span,
            }),
        }
    }

    /// Do a function return with zero return values.
    fn do_exit(&mut self, span: Span) {
        let return_code = self.push_instruction(
            span,
            ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Return as i64)),
        );

        match self.func_type {
            FunctionType::Normal => {
                self.end_current_block(span, ir::ExitKind::Exit);
            }
            FunctionType::Constructor { this, .. } => {
                let ret_scope = self.open_call_scope(span);
                self.push_stack_values(span, ret_scope, [this]);
                self.end_current_block(
                    span,
                    ir::ExitKind::Return {
                        call_scope: ret_scope,
                        stack_base: 0,
                    },
                );
            }
            FunctionType::TryCatch { exit_code, .. } => {
                self.push_instruction(
                    span,
                    ir::InstructionKind::SetVariable(exit_code, return_code),
                );
                self.end_current_block(span, ir::ExitKind::Exit);
            }
        }
    }

    fn do_break(&mut self, span: Span) -> Result<(), IrGenError> {
        if let Some(&jump) = self.break_target_stack.last() {
            self.non_local_jump(span, jump)
        } else if let FunctionType::TryCatch {
            allow_break,
            exit_code,
            ..
        } = self.func_type
            && allow_break
        {
            let return_code = self.push_instruction(
                span,
                ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Break as i64)),
            );
            self.push_instruction(
                span,
                ir::InstructionKind::SetVariable(exit_code, return_code),
            );
            self.end_current_block(span, ir::ExitKind::Exit);
            Ok(())
        } else {
            Err(IrGenError {
                kind: IrGenErrorKind::BreakWithNoTarget,
                span,
            })
        }
    }

    fn do_continue(&mut self, span: Span) -> Result<(), IrGenError> {
        if let Some(&jump) = self.continue_target_stack.last() {
            self.non_local_jump(span, jump)
        } else if let FunctionType::TryCatch {
            allow_continue,
            exit_code,
            ..
        } = self.func_type
            && allow_continue
        {
            let return_code = self.push_instruction(
                span,
                ir::InstructionKind::Constant(Constant::Integer(TryCatchExitCode::Continue as i64)),
            );
            self.push_instruction(
                span,
                ir::InstructionKind::SetVariable(exit_code, return_code),
            );
            self.end_current_block(span, ir::ExitKind::Exit);
            Ok(())
        } else {
            Err(IrGenError {
                kind: IrGenErrorKind::ContinueWithNoTarget,
                span,
            })
        }
    }

    fn non_local_jump(&mut self, span: Span, jump: NonLocalJump) -> Result<(), IrGenError> {
        // We may be jumping to an outer scope which should cause variables declared in inner scopes
        // to close. Generate a cleanup block to close all of the variables between this scope and
        // the target scope.
        let cleanup_block = self.new_block();
        self.end_current_block(span, ir::ExitKind::Jump(cleanup_block));
        self.start_new_block(cleanup_block);

        for i in (jump.pop_vars_to + 1..self.block_scopes.len()).rev() {
            for &var_id in &self.block_scopes[i].to_close {
                let inst_id = self.function.instructions.insert(ir::Instruction {
                    kind: ir::InstructionKind::CloseVariable(var_id),
                    span,
                });
                self.function.blocks[cleanup_block]
                    .instructions
                    .push(inst_id);
            }
        }

        self.end_current_block(span, ir::ExitKind::Jump(jump.target));
        Ok(())
    }

    fn expression(&mut self, expr: &ast::Expression<S>) -> Result<ir::InstId, IrGenError> {
        Ok(match expr {
            ast::Expression::Constant(c, span) => {
                self.push_instruction(*span, ir::InstructionKind::Constant(c.clone()))
            }
            ast::Expression::Ident(s) => self.ident_expr(s)?,
            ast::Expression::Global(span) => {
                self.push_instruction(*span, ir::InstructionKind::Globals)
            }
            ast::Expression::This(span) => self.push_instruction(*span, ir::InstructionKind::This),
            ast::Expression::Other(span) => {
                self.push_instruction(*span, ir::InstructionKind::Other)
            }
            ast::Expression::Group(expr) => self.expression(&expr.inner)?,
            ast::Expression::Object(fields) => {
                let object = self.push_instruction(fields.span, ir::InstructionKind::NewObject);

                for field in &fields.fields {
                    let field_span = field.span();
                    match field {
                        ast::Field::Value(name, value) => {
                            // Within a struct literal, closures always bind `self` to the struct
                            // currently being created.
                            self.push_function_bind_mode(FunctionBindMode::BindNewThis(object));

                            let value = self.expression(value)?;
                            self.push_instruction(
                                field_span,
                                ir::InstructionKind::SetFieldConst {
                                    target: object,
                                    key: Constant::String(name.inner.clone()),
                                    value,
                                },
                            );

                            self.pop_closure_bind_mode(FunctionBindMode::BindNewThis(object));
                        }
                        ast::Field::Init(name) => {
                            let value = self.ident_expr(name)?;
                            self.push_instruction(
                                field_span,
                                ir::InstructionKind::SetFieldConst {
                                    target: object,
                                    key: Constant::String(name.inner.clone()),
                                    value,
                                },
                            );
                        }
                    }
                }

                object
            }
            ast::Expression::Array(elements) => {
                let array = self.push_instruction(elements.span, ir::InstructionKind::NewArray);
                for (i, value) in elements.entries.iter().enumerate() {
                    let value = self.expression(value)?;
                    self.push_instruction(
                        elements.span,
                        ir::InstructionKind::SetIndexConst {
                            target: array,
                            index: Constant::Integer(i as i64),
                            value,
                        },
                    );
                }
                array
            }
            ast::Expression::Unary(unary_expr) => {
                let inst = ir::InstructionKind::UnOp {
                    op: match unary_expr.op {
                        ast::UnaryOp::Not => ir::UnOp::Not,
                        ast::UnaryOp::Minus => ir::UnOp::Negate,
                        ast::UnaryOp::BitNegate => ir::UnOp::BitNegate,
                    },
                    source: self.expression(&unary_expr.target)?,
                };
                self.push_instruction(unary_expr.span, inst)
            }
            ast::Expression::Prefix(mutation) => self.mutation_op(mutation)?.1,
            ast::Expression::Postfix(mutation) => self.mutation_op(mutation)?.0,
            ast::Expression::Binary(bin_expr) => match bin_expr.op {
                ast::BinaryOp::Add => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Add,
                            right,
                        },
                    )
                }
                ast::BinaryOp::Sub => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Sub,
                            right,
                        },
                    )
                }
                ast::BinaryOp::Mult => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Mult,
                            right,
                        },
                    )
                }
                ast::BinaryOp::Div => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Div,
                            right,
                        },
                    )
                }
                ast::BinaryOp::Mod | ast::BinaryOp::Rem => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Rem,
                            right,
                        },
                    )
                }
                ast::BinaryOp::IDiv => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::IDiv,
                            right,
                        },
                    )
                }
                ast::BinaryOp::Equal => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Equal,
                            right,
                        },
                    )
                }
                ast::BinaryOp::NotEqual => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::NotEqual,
                            right,
                        },
                    )
                }
                ast::BinaryOp::LessThan => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::LessThan,
                            right,
                        },
                    )
                }
                ast::BinaryOp::LessEqual => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::LessEqual,
                            right,
                        },
                    )
                }
                ast::BinaryOp::GreaterThan => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::GreaterThan,
                            right,
                        },
                    )
                }
                ast::BinaryOp::GreaterEqual => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::GreaterEqual,
                            right,
                        },
                    )
                }
                ast::BinaryOp::And => {
                    self.short_circuit_and(bin_expr.span, &bin_expr.left, &bin_expr.right)?
                }
                ast::BinaryOp::Or => {
                    self.short_circuit_or(bin_expr.span, &bin_expr.left, &bin_expr.right)?
                }
                ast::BinaryOp::Xor => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::Xor,
                            right,
                        },
                    )
                }
                ast::BinaryOp::BitAnd => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::BitAnd,
                            right,
                        },
                    )
                }
                ast::BinaryOp::BitOr => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::BitOr,
                            right,
                        },
                    )
                }
                ast::BinaryOp::BitXor => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::BitXor,
                            right,
                        },
                    )
                }
                ast::BinaryOp::BitShiftLeft => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::BitShiftLeft,
                            right,
                        },
                    )
                }
                ast::BinaryOp::BitShiftRight => {
                    let left = self.expression(&bin_expr.left)?;
                    let right = self.expression(&bin_expr.right)?;
                    self.push_instruction(
                        bin_expr.span,
                        ir::InstructionKind::BinOp {
                            left,
                            op: ir::BinOp::BitShiftRight,
                            right,
                        },
                    )
                }
                ast::BinaryOp::NullCoalesce => self.short_circuit_null_coalesce(
                    bin_expr.span,
                    &bin_expr.left,
                    &bin_expr.right,
                )?,
            },
            ast::Expression::Ternary(tern_expr) => {
                let cond = self.expression(&tern_expr.cond)?;
                self.if_expr(
                    tern_expr.span,
                    cond,
                    |this| this.expression(&tern_expr.if_true),
                    |this| this.expression(&tern_expr.if_false),
                )?
            }
            ast::Expression::Function(func_expr) => {
                if !self.settings.allow_non_closure_functions {
                    return Err(IrGenError {
                        kind: IrGenErrorKind::NonClosureFunctionsNotAllowed,
                        span: func_expr.span,
                    });
                }

                let allow_constructors = self.settings.allow_constructors;
                let mut compiler =
                    self.start_inner_function(FunctionRef::Expression(func_expr.span), false);

                compiler.declare_parameters(&func_expr.parameters)?;
                let function = if func_expr.is_constructor {
                    if !allow_constructors {
                        return Err(IrGenError {
                            kind: IrGenErrorKind::ConstructorsNotAllowed,
                            span: func_expr.span,
                        });
                    }
                    compiler.constructor(func_expr.inherit.as_ref(), &func_expr.body)?
                } else {
                    compiler.block(&func_expr.body)?;
                    compiler.finish()
                };

                let func_id = self.function.functions.insert(function);
                self.new_bound_function(func_expr.span, func_id)
            }
            ast::Expression::Closure(closure_expr) => {
                let mut compiler =
                    self.start_inner_function(FunctionRef::Expression(closure_expr.span), true);

                compiler.declare_parameters(&closure_expr.parameters)?;
                compiler.block(&closure_expr.body)?;
                let function = compiler.finish();

                let func_id = self.function.functions.insert(function);
                self.push_instruction(
                    closure_expr.span,
                    ir::InstructionKind::Closure {
                        func: func_id,
                        bind_this: false,
                    },
                )
            }
            ast::Expression::Call(call) => {
                let call_scope = self.open_call_expr(call)?;
                let [ret] = self.get_stack_values::<1>(call.span, call_scope, 0);
                self.close_call_scope(call.span, call_scope);
                ret
            }
            ast::Expression::Field(field_expr) => {
                let target = self.expression(&field_expr.base)?;
                let field = self.push_instruction(
                    field_expr.span,
                    ir::InstructionKind::Constant(Constant::String(field_expr.field.inner.clone())),
                );
                self.push_instruction(
                    field_expr.span,
                    ir::InstructionKind::GetField { target, key: field },
                )
            }
            ast::Expression::Index(index_expr) => {
                let target = self.expression(&index_expr.base)?;
                let mut indexes = Vec::new();
                for index in &index_expr.indexes {
                    indexes.push(self.expression(index)?);
                }
                self.get_index(index_expr.span, target, &indexes)
            }
            ast::Expression::Argument(arg_expr) => {
                let arg_index = self.expression(&arg_expr.arg_index)?;
                self.push_instruction(arg_expr.span, ir::InstructionKind::Argument(arg_index))
            }
            ast::Expression::ArgumentCount(span) => {
                self.push_instruction(*span, ir::InstructionKind::ArgumentCount)
            }
        })
    }

    /// Evaluate the call target and all arguments, then open a call scope and perform the function
    /// call on it.
    fn open_call_expr(&mut self, call: &ast::Call<S>) -> Result<ir::CallScope, IrGenError> {
        let call_target = self.call_target_expr(&call.base)?;
        let call_scope = self.open_call_arg_exprs(call.span, &call.arguments)?;
        self.push_instruction(
            call.span,
            ir::InstructionKind::Call {
                scope: call_scope,
                stack_base: 0,
                func: call_target.func(),
                this: call_target.this(),
            },
        );

        Ok(call_scope)
    }

    /// Evaluate all arguments, open a new call scope, then push all arguments to it.
    ///
    /// Trailing function calls are treated special, the trailing function call is called as a
    /// special multi-return call, and this is done recursively for all nested trailing function
    /// calls.
    fn open_call_arg_exprs(
        &mut self,
        call_span: Span,
        args: &[ast::Expression<S>],
    ) -> Result<ir::CallScope, IrGenError> {
        enum Entry {
            Argument(Span, ir::InstId),
            TrailingCall(Span, CallTarget),
        }

        // First, evaluate every argument, all the way down the chain of trailing calls. We push a
        // `TrailingCall` separator when we encounter a function call as the final argument.

        let mut entries = Vec::new();

        let mut current_args = args;
        loop {
            match current_args {
                [] => break,
                [simple_args @ .., last_arg] => {
                    for arg in simple_args {
                        entries.push(Entry::Argument(arg.span(), self.expression(arg)?));
                    }
                    match last_arg {
                        ast::Expression::Call(call) => {
                            entries.push(Entry::TrailingCall(
                                call.span,
                                self.call_target_expr(&call.base)?,
                            ));
                            current_args = &call.arguments;
                        }
                        _ => {
                            entries
                                .push(Entry::Argument(last_arg.span(), self.expression(last_arg)?));
                            break;
                        }
                    }
                }
            }
        }

        let call_scope = self.open_call_scope(call_span);

        // Now push all arguments to the stack for every function call we need to execute, keeping
        // track of where there are trailing function calls that need to be made.

        let mut pending_calls = Vec::new();
        let mut stack_base = 0;
        for entry in entries {
            match entry {
                Entry::Argument(span, inst_id) => {
                    self.push_stack_values(span, call_scope, [inst_id]);
                    stack_base += 1;
                }
                Entry::TrailingCall(span, call_target) => {
                    pending_calls.push((stack_base, span, call_target));
                }
            }
        }

        // Now call every trailing function call. The final stack should contain all arguments to
        // the outermost function call.

        for (stack_base, span, call_target) in pending_calls.into_iter().rev() {
            self.push_instruction(
                span,
                ir::InstructionKind::Call {
                    scope: call_scope,
                    stack_base,
                    func: call_target.func(),
                    this: call_target.this(),
                },
            );
        }

        Ok(call_scope)
    }

    fn call_target_expr(&mut self, target: &ast::Expression<S>) -> Result<CallTarget, IrGenError> {
        // Function calls on fields are interpreted as "methods", and implicitly bind the containing
        // object as `self` for the function call.
        Ok(if let ast::Expression::Field(field_expr) = target {
            let target = self.expression(&field_expr.base)?;
            let key = self.push_instruction(
                field_expr.field.span,
                ir::InstructionKind::Constant(Constant::String(field_expr.field.inner.clone())),
            );
            let func = self.push_instruction(
                field_expr.span,
                ir::InstructionKind::GetField { target, key },
            );
            CallTarget::Method { func, this: target }
        } else {
            CallTarget::Function(self.expression(target)?)
        })
    }

    fn if_expr(
        &mut self,
        span: Span,
        cond: ir::InstId,
        if_true: impl FnOnce(&mut Self) -> Result<ir::InstId, IrGenError>,
        if_false: impl FnOnce(&mut Self) -> Result<ir::InstId, IrGenError>,
    ) -> Result<ir::InstId, IrGenError> {
        let res_var = self.function.variables.insert(ir::Variable::Heap);
        self.push_instruction(span, ir::InstructionKind::OpenVariable(res_var));

        let if_true_block = self.new_block();
        let if_false_block = self.new_block();
        let successor = self.new_block();

        self.end_current_block(
            span,
            ir::ExitKind::Branch {
                cond: ir::BranchCondition::IsTrue(cond),
                if_true: if_true_block,
                if_false: if_false_block,
            },
        );

        self.start_new_block(if_true_block);
        let if_true_res = if_true(self)?;
        self.push_instruction(span, ir::InstructionKind::SetVariable(res_var, if_true_res));
        self.end_current_block(span, ir::ExitKind::Jump(successor));

        self.start_new_block(if_false_block);
        let if_false_res = if_false(self)?;
        self.push_instruction(
            span,
            ir::InstructionKind::SetVariable(res_var, if_false_res),
        );
        self.end_current_block(span, ir::ExitKind::Jump(successor));

        self.start_new_block(successor);
        let res = self.push_instruction(span, ir::InstructionKind::GetVariable(res_var));
        self.push_instruction(span, ir::InstructionKind::CloseVariable(res_var));

        Ok(res)
    }

    fn ident_expr(&mut self, ident: &ast::Ident<S>) -> Result<ir::InstId, IrGenError> {
        Ok(if let Some(var) = self.find_var(ident) {
            self.get_var(ident.span, var.into())
        } else {
            match self.var_dict.free_var_mode(ident) {
                FreeVarMode::This => {
                    if !self.settings.allow_implicit_self {
                        return Err(IrGenError {
                            kind: IrGenErrorKind::ImplicitSelfNotAllowed,
                            span: ident.span,
                        });
                    }

                    let this = self.push_instruction(ident.span, ir::InstructionKind::This);
                    let key = self.push_instruction(
                        ident.span,
                        ir::InstructionKind::Constant(Constant::String(ident.inner.clone())),
                    );
                    self.push_instruction(
                        ident.span,
                        ir::InstructionKind::GetField { target: this, key },
                    )
                }
                FreeVarMode::GlobalVar => {
                    let globals = self.push_instruction(ident.span, ir::InstructionKind::Globals);
                    let key = self.push_instruction(
                        ident.span,
                        ir::InstructionKind::Constant(Constant::String(ident.inner.clone())),
                    );
                    self.push_instruction(
                        ident.span,
                        ir::InstructionKind::GetField {
                            target: globals,
                            key,
                        },
                    )
                }
                FreeVarMode::Magic { .. } => self.push_instruction(
                    ident.span,
                    ir::InstructionKind::GetMagic(ident.inner.clone()),
                ),
            }
        })
    }

    fn short_circuit_and(
        &mut self,
        span: Span,
        left: &ast::Expression<S>,
        right: &ast::Expression<S>,
    ) -> Result<ir::InstId, IrGenError> {
        let left = self.expression(left)?;
        self.if_expr(
            span,
            left,
            |this| {
                let right = this.expression(right)?;
                Ok(this.push_instruction(
                    span,
                    ir::InstructionKind::BinOp {
                        left,
                        op: ir::BinOp::And,
                        right,
                    },
                ))
            },
            |this| {
                Ok(this.push_instruction(
                    span,
                    ir::InstructionKind::Constant(Constant::Boolean(false)),
                ))
            },
        )
    }

    fn short_circuit_or(
        &mut self,
        span: Span,
        left: &ast::Expression<S>,
        right: &ast::Expression<S>,
    ) -> Result<ir::InstId, IrGenError> {
        let left = self.expression(left)?;
        self.if_expr(
            span,
            left,
            |this| {
                Ok(this
                    .push_instruction(span, ir::InstructionKind::Constant(Constant::Boolean(true))))
            },
            |this| {
                let right = this.expression(right)?;
                Ok(this.push_instruction(
                    span,
                    ir::InstructionKind::BinOp {
                        left,
                        op: ir::BinOp::Or,
                        right,
                    },
                ))
            },
        )
    }

    fn short_circuit_null_coalesce(
        &mut self,
        span: Span,
        left: &ast::Expression<S>,
        right: &ast::Expression<S>,
    ) -> Result<ir::InstId, IrGenError> {
        let left = self.expression(left)?;
        let cond = self.push_instruction(
            span,
            ir::InstructionKind::UnOp {
                op: ir::UnOp::IsUndefined,
                source: left,
            },
        );
        self.if_expr(span, cond, |this| this.expression(right), |_| Ok(left))
    }

    /// Evaluate a `MutationOp` on a `MutableExpr`.
    ///
    /// Returns a tuple of the old and new values for the `MutableExpr`.
    fn mutation_op(
        &mut self,
        mutation: &ast::Mutation<S>,
    ) -> Result<(ir::InstId, ir::InstId), IrGenError> {
        let target = self.mutable_target(&mutation.target)?;
        let old = self.read_mutable_target(mutation.span, target.clone());
        let op = match mutation.op {
            ast::MutationOp::Increment => ir::UnOp::Increment,
            ast::MutationOp::Decrement => ir::UnOp::Decrement,
        };
        let new =
            self.push_instruction(mutation.span, ir::InstructionKind::UnOp { op, source: old });
        self.write_mutable_target(mutation.span, target, new);
        Ok((old, new))
    }

    fn mutable_target(
        &mut self,
        target: &ast::MutableExpr<S>,
    ) -> Result<MutableTarget<S>, IrGenError> {
        Ok(match target {
            ast::MutableExpr::Ident(ident) => {
                if let Some(var) = self.find_var(ident) {
                    MutableTarget::Var(var.into())
                } else {
                    match self.var_dict.free_var_mode(ident) {
                        FreeVarMode::This => {
                            if !self.settings.allow_implicit_self {
                                return Err(IrGenError {
                                    kind: IrGenErrorKind::ImplicitSelfNotAllowed,
                                    span: ident.span,
                                });
                            }

                            let key = self.push_instruction(
                                target.span(),
                                ir::InstructionKind::Constant(Constant::String(
                                    ident.inner.clone(),
                                )),
                            );
                            MutableTarget::This { key }
                        }
                        FreeVarMode::GlobalVar => {
                            let key = self.push_instruction(
                                target.span(),
                                ir::InstructionKind::Constant(Constant::String(
                                    ident.inner.clone(),
                                )),
                            );
                            MutableTarget::Globals { key }
                        }
                        FreeVarMode::Magic { is_read_only } => {
                            if is_read_only {
                                return Err(IrGenError {
                                    kind: IrGenErrorKind::ReadOnlyMagic,
                                    span: target.span(),
                                });
                            }

                            MutableTarget::Magic(ident.inner.clone())
                        }
                    }
                }
            }
            ast::MutableExpr::Field(field_expr) => {
                let target = self.expression(&field_expr.base)?;
                let key = self.push_instruction(
                    field_expr.field.span,
                    ir::InstructionKind::Constant(Constant::String(field_expr.field.inner.clone())),
                );
                MutableTarget::Field { target, key }
            }
            ast::MutableExpr::Index(index_expr) => {
                let target = self.expression(&index_expr.base)?;
                let mut indexes = Vec::new();
                for index in &index_expr.indexes {
                    indexes.push(self.expression(index)?);
                }
                MutableTarget::Index { target, indexes }
            }
        })
    }

    fn read_mutable_target(&mut self, span: Span, target: MutableTarget<S>) -> ir::InstId {
        match target {
            MutableTarget::Var(var) => self.get_var(span, var),
            MutableTarget::This { key } => {
                let this = self.push_instruction(span, ir::InstructionKind::This);
                self.push_instruction(span, ir::InstructionKind::GetField { target: this, key })
            }
            MutableTarget::Globals { key } => {
                let globals = self.push_instruction(span, ir::InstructionKind::Globals);
                self.push_instruction(
                    span,
                    ir::InstructionKind::GetField {
                        target: globals,
                        key,
                    },
                )
            }
            MutableTarget::Field { target, key } => {
                self.push_instruction(span, ir::InstructionKind::GetField { target, key })
            }
            MutableTarget::Index { target, indexes } => self.get_index(span, target, &indexes),
            MutableTarget::Magic(name) => {
                self.push_instruction(span, ir::InstructionKind::GetMagic(name))
            }
        }
    }

    fn write_mutable_target(&mut self, span: Span, target: MutableTarget<S>, value: ir::InstId) {
        match target {
            MutableTarget::Var(var) => {
                self.set_var(span, var, value);
            }
            MutableTarget::This { key } => {
                let this = self.push_instruction(span, ir::InstructionKind::This);
                self.push_instruction(
                    span,
                    ir::InstructionKind::SetField {
                        target: this,
                        key,
                        value,
                    },
                );
            }
            MutableTarget::Globals { key } => {
                let globals = self.push_instruction(span, ir::InstructionKind::Globals);
                self.push_instruction(
                    span,
                    ir::InstructionKind::SetField {
                        target: globals,
                        key,
                        value,
                    },
                );
            }
            MutableTarget::Field { target, key } => {
                self.push_instruction(span, ir::InstructionKind::SetField { target, key, value });
            }
            MutableTarget::Index { target, indexes } => {
                self.set_index(span, target, &indexes, value);
            }
            MutableTarget::Magic(magic) => {
                self.push_instruction(span, ir::InstructionKind::SetMagic(magic, value));
            }
        }
    }

    fn get_index(&mut self, span: Span, target: ir::InstId, indexes: &[ir::InstId]) -> ir::InstId {
        if indexes.len() == 1 {
            self.push_instruction(
                span,
                ir::InstructionKind::GetIndex {
                    target,
                    index: indexes[0],
                },
            )
        } else {
            let get_multi_index_name = self.interner.intern(BuiltIns::GET_MULTI_INDEX);
            let get_multi_index =
                self.push_instruction(span, ir::InstructionKind::GetMagic(get_multi_index_name));

            let [ret] = self.call_function::<1>(
                span,
                get_multi_index,
                None,
                [target].into_iter().chain(indexes.iter().copied()),
            );
            ret
        }
    }

    fn set_index(
        &mut self,
        span: Span,
        target: ir::InstId,
        indexes: &[ir::InstId],
        value: ir::InstId,
    ) {
        if indexes.len() == 1 {
            self.push_instruction(
                span,
                ir::InstructionKind::SetIndex {
                    target,
                    index: indexes[0],
                    value,
                },
            );
        } else {
            let set_multi_index_name = self.interner.intern(BuiltIns::SET_MULTI_INDEX);
            let set_multi_index =
                self.push_instruction(span, ir::InstructionKind::GetMagic(set_multi_index_name));

            self.call_function::<0>(
                span,
                set_multi_index,
                None,
                [target, value].into_iter().chain(indexes.iter().copied()),
            );
        }
    }

    fn start_inner_function(
        &mut self,
        reference: FunctionRef,
        capture_outer: bool,
    ) -> FunctionCompiler<'_, S> {
        let mut compiler =
            FunctionCompiler::new(self.settings, self.interner, reference, self.var_dict);

        if capture_outer {
            let mut upper_constructor_parents = FxHashMap::default();

            // Takes a `VarId` in this function and returns an upvar in the inner function.
            //
            // Used to reference constructor parents in the outer function without duplicate
            // declarations.
            let mut make_constructor_parent_upvar =
                |compiler: &mut FunctionCompiler<S>, parent_var: ir::VarId, field: &S| {
                    let parent = match upper_constructor_parents.entry(parent_var) {
                        hash_map::Entry::Occupied(occupied) => *occupied.get(),
                        hash_map::Entry::Vacant(vacant) => {
                            let parent_var = compiler
                                .function
                                .variables
                                .insert(ir::Variable::Upper(parent_var));
                            *vacant.insert(parent_var)
                        }
                    };

                    FunctionVarDecl::UpperConstructorStatic {
                        parent,
                        field: field.clone(),
                    }
                };

            for (name, var) in &self.function_scope_vars {
                // Don't close over function-scope variables if they are shadowed by a block-scope
                // variable.
                if self.block_variable_lookup.contains_key(name) {
                    continue;
                }

                match var {
                    &VariableType::Normal(var_id) => {
                        compiler
                            .declare_function_var(name.clone(), ir::Variable::Upper(var_id).into())
                            .unwrap();
                    }
                    VariableType::ConstructorStatic(field) => {
                        let FunctionType::Constructor { parent_var, .. } = self.func_type else {
                            panic!("constructor static var in non-constructor function")
                        };

                        let var_type =
                            make_constructor_parent_upvar(&mut compiler, parent_var, field);
                        compiler
                            .declare_function_var(name.clone(), var_type)
                            .unwrap();
                    }
                    &VariableType::UpperConstructorStatic { parent, ref field } => {
                        let var_type = make_constructor_parent_upvar(&mut compiler, parent, field);
                        compiler
                            .declare_function_var(name.clone(), var_type)
                            .unwrap();
                    }
                }
            }

            for (name, scope_list) in &self.block_variable_lookup {
                let &scope_index = scope_list.last().unwrap();
                let var_id = self.block_scopes[scope_index].visible[name];
                compiler
                    .declare_function_var(name.clone(), ir::Variable::Upper(var_id).into())
                    .unwrap();
            }
        }

        compiler
    }

    fn push_scope(&mut self) {
        self.block_scopes.push(BlockScope::default());
    }

    fn pop_scope(&mut self) {
        if let Some(popped_scope) = self.block_scopes.pop() {
            // Close every variable in the popped scope.
            for var_id in popped_scope.to_close {
                self.push_instruction(Span::null(), ir::InstructionKind::CloseVariable(var_id));
            }

            // Remove visible variables in the popped scope from the var lookup map.
            for (vname, _) in popped_scope.visible {
                let hash_map::Entry::Occupied(mut entry) = self.block_variable_lookup.entry(vname)
                else {
                    unreachable!();
                };

                let scope_index = entry.get_mut().pop().unwrap();
                // The var lookup map should contain every visible variable in this scope.
                assert!(scope_index == self.block_scopes.len());

                // Just remove the variable entry entirely if there are no variables with this
                // name visible.
                if entry.get().is_empty() {
                    entry.remove();
                }
            }
        }
    }

    fn declare_function_var(
        &mut self,
        vname: ast::Ident<S>,
        var_decl: FunctionVarDecl<S>,
    ) -> Result<VariableType<S>, IrGenError> {
        if let Some(shadowed_var) = self.find_var(&vname) {
            // Function-scope variable shadowing within a single function is disallowed.
            //
            // Upper variables may not shadow nothing else, as they are expected to be unique and
            // declared first before all other variable types. After this, an upper variable may be
            // shadowed by any variable type. This matches the behavior of JS, where shadowing is
            // always allowed across function boundaries.
            //
            // Owned, function-scope variables are forbidden from shadowing existing block-scope
            // variables.
            //
            // An owned, function-scope variable declaration that shares the name with another
            // does not actually shadow, it is instead a re-declaration of the *same* variable.
            // Re-declaration with another kind of variable is always an error.
            //
            // We forbid static variables from being re-declared at all, because multiple static
            // initializers doesn't make much sense.
            //
            // Constructor static declarations already cannot share the same name because they are
            // object fields, and cannot be re-declared as a different kind.

            if matches!(
                var_decl,
                FunctionVarDecl::Normal(ir::Variable::Upper(_))
                    | FunctionVarDecl::UpperConstructorStatic { .. }
            ) {
                // Upper variables are supposed to be unique and must be declared first.
                panic!("upper variables should not shadow any other variable");
            }

            match shadowed_var {
                FoundVariable::Owned {
                    var_id: shadowed_var_id,
                    has_block_scope,
                    is_static,
                } => {
                    if has_block_scope {
                        // Function-scope variables are forbidden from shadowing existing
                        // block-scope variables.
                        return Err(IrGenError {
                            kind: IrGenErrorKind::FunctionScopeCannotShadowBlockScope,
                            span: vname.span,
                        });
                    } else {
                        if !is_static
                            && matches!(var_decl, FunctionVarDecl::Normal(ir::Variable::Heap))
                        {
                            // Normal owned variable re-declarations simply modify the same
                            // variable.
                            return Ok(VariableType::Normal(shadowed_var_id));
                        } else {
                            // Everything else is an invalid re-declaration.
                            return Err(IrGenError {
                                kind: IrGenErrorKind::BadFunctionScopeVarRedeclaration,
                                span: vname.span,
                            });
                        }
                    }
                }
                FoundVariable::ConstructorStatic(_) => {
                    // Redeclaring a constructor static var is always a bad re-declaration.
                    return Err(IrGenError {
                        kind: IrGenErrorKind::BadFunctionScopeVarRedeclaration,
                        span: vname.span,
                    });
                }
                FoundVariable::Upper(_) | FoundVariable::UpperConstructorStatic { .. } => {
                    // Allow new declarations to shadow any existing upper variables.
                }
            }
        }

        let var_type = match var_decl {
            FunctionVarDecl::Normal(variable) => {
                if self.var_dict.is_reserved(&vname) {
                    return Err(IrGenError {
                        kind: IrGenErrorKind::DeclaredNameIsReserved,
                        span: vname.span,
                    });
                }

                let is_heap = variable.is_heap();
                let var_id = self.function.variables.insert(variable);

                if is_heap {
                    // This is a new heap variable, so we need to open it.
                    //
                    // Since we're not using block scoping, just open every variable at the very
                    // start of the function. This keeps the IR well-formed even with no block
                    // scoping and no explicit `CloseVariable` instructions.
                    //
                    // We push this instruction to `start_block`, which is kept otherwise empty for
                    // this purpose.
                    let inst_id = self.function.instructions.insert(ir::Instruction {
                        kind: ir::InstructionKind::OpenVariable(var_id),
                        span: vname.span,
                    });
                    self.function.blocks[self.function.start_block]
                        .instructions
                        .push(inst_id);
                }

                VariableType::Normal(var_id)
            }
            // Permit any name for constructor statics, since they can be used as field names (which
            // are unrestricted).
            FunctionVarDecl::ConstructorStatic(field) => VariableType::ConstructorStatic(field),
            FunctionVarDecl::UpperConstructorStatic { parent, field } => {
                VariableType::UpperConstructorStatic { parent, field }
            }
        };

        assert!(
            self.function_scope_vars
                .insert(vname, var_type.clone())
                .is_none()
        );

        Ok(var_type)
    }

    fn open_owned_block_var(&mut self, span: Span, variable: ir::Variable<S>) -> ir::VarId {
        assert!(!matches!(variable, ir::Variable::Upper(_)));

        let top_scope = self.block_scopes.last_mut().unwrap();

        let is_heap = variable.is_heap();
        let var_id = self.function.variables.insert(variable);

        if is_heap {
            // This is a new owned variable, so we need to open it.
            //
            // Since we're using block scoping, we open it in the current block scope and close it
            // when the scope ends.
            top_scope.to_close.push(var_id);
            self.push_instruction(span, ir::InstructionKind::OpenVariable(var_id));
        }

        var_id
    }

    fn declare_block_var(
        &mut self,
        vname: ast::Ident<S>,
        var_id: ir::VarId,
    ) -> Result<(), IrGenError> {
        let top_scope_index = self.block_scopes.len() - 1;
        let top_scope = self.block_scopes.last_mut().unwrap();

        if self.var_dict.is_reserved(&vname) {
            return Err(IrGenError {
                kind: IrGenErrorKind::DeclaredNameIsReserved,
                span: vname.span,
            });
        }

        let shadowing = top_scope.visible.insert(vname.clone(), var_id).is_some();

        let scope_list = self.block_variable_lookup.entry(vname).or_default();
        if shadowing {
            // The new variable shadows a previous one, so there should already be an existing entry
            // at the top of the scope list for the top-level scope.
            assert_eq!(*scope_list.last().unwrap(), top_scope_index);
        } else {
            // If we are not shadowing, we expect any current active entry to be of an outer scope.
            assert!(scope_list.last().is_none_or(|&ind| ind < top_scope_index));
            scope_list.push(top_scope_index);
        }

        Ok(())
    }

    fn find_var(&mut self, vname: &S) -> Option<FoundVariable<S>> {
        if let Some(block_scope_index) = self
            .block_variable_lookup
            .get(vname)
            .and_then(|l| l.last().copied())
        {
            let var_id = self.block_scopes[block_scope_index].visible[vname];
            Some(match self.function.variables[var_id] {
                ir::Variable::Heap => FoundVariable::Owned {
                    var_id,
                    is_static: false,
                    has_block_scope: true,
                },
                ir::Variable::Static(_) => FoundVariable::Owned {
                    var_id,
                    is_static: true,
                    has_block_scope: true,
                },
                ir::Variable::Upper(_) => FoundVariable::Upper(var_id),
            })
        } else if let Some(function_var_decl) = self.function_scope_vars.get(vname).cloned() {
            Some(match function_var_decl {
                VariableType::Normal(var_id) => match self.function.variables[var_id] {
                    ir::Variable::Heap => FoundVariable::Owned {
                        var_id,
                        is_static: false,
                        has_block_scope: false,
                    },
                    ir::Variable::Static(_) => FoundVariable::Owned {
                        var_id,
                        is_static: true,
                        has_block_scope: false,
                    },
                    ir::Variable::Upper(_) => FoundVariable::Upper(var_id),
                },
                VariableType::ConstructorStatic(field) => FoundVariable::ConstructorStatic(field),
                VariableType::UpperConstructorStatic { parent, field } => {
                    FoundVariable::UpperConstructorStatic { parent, field }
                }
            })
        } else {
            None
        }
    }

    fn get_var(&mut self, span: Span, var: VariableType<S>) -> ir::InstId {
        match var {
            VariableType::Normal(var_id) => {
                self.push_instruction(span, ir::InstructionKind::GetVariable(var_id))
            }
            VariableType::ConstructorStatic(field) => {
                let FunctionType::Constructor { parent, .. } = self.func_type else {
                    panic!("constructor static var in non-constructor function")
                };

                self.push_instruction(
                    span,
                    ir::InstructionKind::GetFieldConst {
                        target: parent,
                        key: Constant::String(field),
                    },
                )
            }
            VariableType::UpperConstructorStatic { parent, field } => {
                let parent = self.push_instruction(span, ir::InstructionKind::GetVariable(parent));
                self.push_instruction(
                    span,
                    ir::InstructionKind::GetFieldConst {
                        target: parent,
                        key: Constant::String(field),
                    },
                )
            }
        }
    }

    fn set_var(&mut self, span: Span, var: VariableType<S>, value: ir::InstId) {
        match var {
            VariableType::Normal(var_id) => {
                self.push_instruction(span, ir::InstructionKind::SetVariable(var_id, value));
            }
            VariableType::ConstructorStatic(field) => {
                let FunctionType::Constructor { parent, .. } = self.func_type else {
                    panic!("constructor static var in non-constructor function")
                };

                self.push_instruction(
                    span,
                    ir::InstructionKind::SetFieldConst {
                        target: parent,
                        key: Constant::String(field),
                        value,
                    },
                );
            }
            VariableType::UpperConstructorStatic { parent, field } => {
                let parent = self.push_instruction(span, ir::InstructionKind::GetVariable(parent));
                self.push_instruction(
                    span,
                    ir::InstructionKind::SetFieldConst {
                        target: parent,
                        key: Constant::String(field),
                        value,
                    },
                );
            }
        }
    }

    fn new_block(&mut self) -> ir::BlockId {
        self.function.blocks.insert(ir::Block::default())
    }

    /// If there is a current block, finishes it with the given `Exit`.
    fn end_current_block(&mut self, span: Span, kind: ir::ExitKind) {
        if let Some(current_block) = self.current_block {
            self.function.blocks[current_block].exit = ir::Exit { kind, span };
            self.current_block = None;
        }
    }

    /// Start a new block.
    ///
    /// There must not currently be an active block.
    fn start_new_block(&mut self, block_id: ir::BlockId) {
        assert!(
            self.current_block.is_none(),
            "cannot start new block when current block is not finished"
        );
        self.current_block = Some(block_id);
    }

    fn push_break_target(&mut self, target_block: ir::BlockId) {
        self.break_target_stack.push(NonLocalJump {
            target: target_block,
            pop_vars_to: self.block_scopes.len() - 1,
        });
    }

    fn pop_break_target(&mut self, block_id: ir::BlockId) {
        assert!(
            self.break_target_stack
                .pop()
                .is_some_and(|j| j.target == block_id),
            "mismatched break target pop"
        );
    }

    fn push_continue_target(&mut self, target_block: ir::BlockId) {
        self.continue_target_stack.push(NonLocalJump {
            target: target_block,
            pop_vars_to: self.block_scopes.len() - 1,
        });
    }

    fn pop_continue_target(&mut self, block_id: ir::BlockId) {
        assert!(
            self.continue_target_stack
                .pop()
                .is_some_and(|j| j.target == block_id),
            "mismatched continue target pop"
        );
    }

    fn push_function_bind_mode(&mut self, mode: FunctionBindMode) {
        self.function_bind_mode.push(mode);
    }

    fn pop_closure_bind_mode(&mut self, mode: FunctionBindMode) {
        assert!(
            self.function_bind_mode.pop().is_some_and(|m| m == mode),
            "mismatched closure bind mode pop"
        );
    }

    /// Create a new inner function with the current function bind mode
    fn new_bound_function(&mut self, span: Span, func_id: ir::FuncId) -> ir::InstId {
        match self
            .function_bind_mode
            .last()
            .copied()
            .unwrap_or(FunctionBindMode::BindDefault)
        {
            FunctionBindMode::BindDefault => self.push_instruction(
                span,
                ir::InstructionKind::Closure {
                    func: func_id,
                    bind_this: true,
                },
            ),
            FunctionBindMode::BindNewThis(this) => {
                let this_scope = self.open_this_scope(span);
                self.push_instruction(span, ir::InstructionKind::SetThis(this_scope, this));
                let closure = self.push_instruction(
                    span,
                    ir::InstructionKind::Closure {
                        func: func_id,
                        bind_this: true,
                    },
                );
                self.close_this_scope(span, this_scope);
                closure
            }
            FunctionBindMode::BindNothing => self.push_instruction(
                span,
                ir::InstructionKind::Closure {
                    func: func_id,
                    bind_this: false,
                },
            ),
        }
    }

    /// Pushes instructions to the IR to call a function with a fixed number of arguments and
    /// returns.
    fn call_function<const RET: usize>(
        &mut self,
        span: Span,
        func: ir::InstId,
        this: Option<ir::InstId>,
        args: impl IntoIterator<Item = ir::InstId>,
    ) -> [ir::InstId; RET] {
        let call_scope = self.open_call_scope(span);
        self.push_stack_values(span, call_scope, args);
        self.push_instruction(
            span,
            ir::InstructionKind::Call {
                scope: call_scope,
                stack_base: 0,
                func,
                this,
            },
        );
        let rets = self.get_stack_values(span, call_scope, 0);
        self.close_call_scope(span, call_scope);
        rets
    }

    fn open_call_scope(&mut self, span: Span) -> ir::CallScope {
        let call_scope = self.function.call_scopes.insert(());
        self.push_instruction(span, ir::InstructionKind::OpenCallScope(call_scope));
        call_scope
    }

    fn close_call_scope(&mut self, span: Span, call_scope: ir::CallScope) {
        self.push_instruction(span, ir::InstructionKind::CloseCallScope(call_scope));
    }

    fn push_stack_values(
        &mut self,
        span: Span,
        call_scope: ir::CallScope,
        args: impl IntoIterator<Item = ir::InstId>,
    ) {
        for arg in args {
            self.push_instruction(span, ir::InstructionKind::PushStack(call_scope, arg));
        }
    }

    fn get_stack_values<const RET: usize>(
        &mut self,
        span: Span,
        call_scope: ir::CallScope,
        stack_base: usize,
    ) -> [ir::InstId; RET] {
        array::from_fn(|i| {
            self.push_instruction(
                span,
                ir::InstructionKind::GetStack(call_scope, stack_base + i),
            )
        })
    }

    fn open_this_scope(&mut self, span: Span) -> ir::ThisScope {
        let this_scope = self.function.this_scopes.insert(());
        self.push_instruction(span, ir::InstructionKind::OpenThisScope(this_scope));
        this_scope
    }

    fn close_this_scope(&mut self, span: Span, this_scope: ir::ThisScope) {
        self.push_instruction(span, ir::InstructionKind::CloseThisScope(this_scope));
    }

    fn push_instruction(&mut self, span: Span, kind: ir::InstructionKind<S>) -> ir::InstId {
        let current_block = if let Some(current) = self.current_block {
            current
        } else {
            // If we do not have an active block, create a new orphan one.
            //
            // Blocks can abruptly end due to statements like break, continue, and return, so this
            // will create a new *most likely unreachable* block to place instructions.
            let dead_block = self.new_block();
            self.current_block = Some(dead_block);
            dead_block
        };

        let inst_id = self
            .function
            .instructions
            .insert(ir::Instruction { kind, span });
        self.function.blocks[current_block]
            .instructions
            .push(inst_id);
        inst_id
    }
}
