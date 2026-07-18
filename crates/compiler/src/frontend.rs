use std::{hash::Hash, path::Path};

use fabricator_util::index_containers::IndexMap;
use fabricator_vm as vm;
use gc_arena::Gc;
use rustc_hash::{FxHashMap, FxHashSet};
use thiserror::Error;

use crate::{
    analysis::{
        block_simplification::{block_branch_to_jump, merge_blocks, redirect_empty_blocks},
        cleanup::{
            clean_instructions, clean_unreachable_blocks, clean_unused_call_scopes,
            clean_unused_functions, clean_unused_shadow_vars, clean_unused_this_scopes,
            clean_unused_variables,
        },
        constant_folding::fold_constants,
        dead_code_elim::eliminate_dead_code,
        eliminate_copies::eliminate_copies,
        instruction_liveness::{InstructionLiveness, InstructionVerificationError},
        nested_scope_liveness::{
            CallScopeLiveness, CallScopeVerificationError, ThisScopeLiveness,
            ThisScopeVerificationError,
        },
        shadow_liveness::{ShadowLiveness, ShadowVerificationError},
        shadow_reduction::reduce_shadows,
        simplify_branches::simplify_branches,
        ssa_conversion::convert_to_ssa,
        variable_liveness::{VariableLiveness, VariableVerificationError},
        verify_references::{ReferenceVerificationError, verify_references},
        verify_upvars::{UpVarVerificationError, verify_no_root_upvars, verify_upvars},
    },
    code_gen::{Prototype, gen_prototype},
    enums::{EnumError, EnumEvaluationError, EnumResolutionError, EnumSet},
    exports::{DuplicateExportError, Export},
    ir,
    ir_gen::{FreeVarMode, IrGenError, IrGenSettings, VarDict},
    lexer::LexError,
    macros::{MacroError, MacroSet, RecursiveMacro},
    parser::{ParseError, ParseSettings},
    preprocessing::{
        ChunkLexError, LexedChunk, PreprocessError, PreprocessErrorKind, PreprocessOutput,
        Preprocessor, ShadowsSpecialError, SourceChunk,
    },
    string_interner::StringInterner,
};

#[derive(Debug, Error)]
pub enum CompileErrorKind {
    #[error("lex error: {0}")]
    Lexing(#[source] LexError),
    #[error("macro error: {0}")]
    Macro(#[source] MacroError),
    #[error("recursive macro: {0}")]
    RecursiveMacro(#[source] RecursiveMacro),
    #[error("parse error: {0}")]
    Parsing(#[source] ParseError),
    #[error("enum error: {0}")]
    Enum(#[source] EnumError),
    #[error("enum error: {0}")]
    EnumResolution(#[source] EnumResolutionError),
    #[error("enum error: {0}")]
    EnumEvaluation(#[source] EnumEvaluationError),
    #[error("duplicate export error: {0}")]
    DuplicateExport(#[source] DuplicateExportError),
    #[error("shadows special error: {0}")]
    ShadowsSpecial(#[source] ShadowsSpecialError),
    #[error("IR gen error: {0}")]
    IrGen(#[source] IrGenError),
}

#[derive(Debug, Error)]
#[error("{kind} at {chunk_name}:{line_number}")]
pub struct CompileError {
    #[source]
    pub kind: CompileErrorKind,
    pub chunk_name: vm::SharedStr,
    pub line_number: vm::LineNumber,
}

impl From<ChunkLexError> for CompileError {
    fn from(err: ChunkLexError) -> Self {
        Self {
            kind: CompileErrorKind::Lexing(err.error),
            chunk_name: err.chunk_name,
            line_number: err.line_number,
        }
    }
}

impl From<PreprocessError> for CompileError {
    fn from(err: PreprocessError) -> Self {
        let kind = match err.kind {
            PreprocessErrorKind::Macro(err) => CompileErrorKind::Macro(err),
            PreprocessErrorKind::RecursiveMacro(err) => CompileErrorKind::RecursiveMacro(err),
            PreprocessErrorKind::Parsing(err) => CompileErrorKind::Parsing(err),
            PreprocessErrorKind::Enum(err) => CompileErrorKind::Enum(err),
            PreprocessErrorKind::EnumResolution(err) => CompileErrorKind::EnumResolution(err),
            PreprocessErrorKind::EnumEvaluation(err) => CompileErrorKind::EnumEvaluation(err),
            PreprocessErrorKind::DuplicateExport(err) => CompileErrorKind::DuplicateExport(err),
            PreprocessErrorKind::ShadowsSpecial(err) => CompileErrorKind::ShadowsSpecial(err),
        };

        Self {
            kind,
            chunk_name: err.chunk_name,
            line_number: err.line_number,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct CompileSettings {
    pub parse: ParseSettings,
    pub ir_gen: IrGenSettings,
    pub optimization_passes: u8,
    pub export_top_level_functions: bool,
    pub verify_ir: bool,
}

impl CompileSettings {
    pub fn compat() -> Self {
        Self {
            parse: ParseSettings::compat(),
            ir_gen: IrGenSettings::compat(),
            optimization_passes: 2,
            export_top_level_functions: true,
            verify_ir: cfg!(debug_assertions),
        }
    }

    pub fn strict() -> Self {
        Self {
            parse: ParseSettings::strict(),
            ir_gen: IrGenSettings::strict(),
            optimization_passes: 2,
            export_top_level_functions: true,
            verify_ir: cfg!(debug_assertions),
        }
    }

    /// If the given path has a (case-insensitive) `.gml` extension, then compile in compat mode,
    /// otherwise strict.
    pub fn from_path(path: &Path) -> Self {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gml"))
        {
            Self::compat()
        } else {
            Self::strict()
        }
    }

    pub fn set_optimization_passes(mut self, passes: u8) -> Self {
        self.optimization_passes = passes;
        self
    }

    pub fn export_top_level_functions(mut self, export_top_funcs: bool) -> Self {
        self.export_top_level_functions = export_top_funcs;
        self
    }

    /// Do extra checks on the produced IR to ensure that it is valid.
    ///
    /// Defaults to `cfg!(debug_assertions)`.
    pub fn verify_ir(mut self, verify_ir: bool) -> Self {
        self.verify_ir = verify_ir;
        self
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ExternalVarMode {
    /// The identifier is an externally defined global variable.
    Global,
    /// The identifier is an externally defined magic variable.
    Magic {
        /// If true, then code is only permitted to read from this magic variable.
        is_read_only: bool,
    },
}

/// Compile FML code.
///
/// Compiles separate code units together in multiple phases to allow for interdependencies.
pub struct Compiler<S, I> {
    preprocessor: Preprocessor<S>,
    string_interner: I,
    ir_compile_settings: Vec<IrCompileSettings>,
}

impl<S, I> Compiler<S, I>
where
    S: Eq + Hash + Clone + AsRef<str>,
    I: StringInterner<String = S>,
{
    /// Create a new instance of the `Compiler` for compiling a single compilation unit.
    ///
    /// The provided `macros` and `enums` are assumed to be externally defined and will be available
    /// to all compiled chunks and are merged into the final macros and enums output.
    ///
    /// The provided `external_var_mode` predicate will be used to determine if unknown references
    /// to free variables should be interpreted as either externally defined global variables or
    /// magic variables.
    pub fn new(string_interner: I) -> Self {
        let preprocessor = Preprocessor::default();
        Self {
            preprocessor,
            string_interner,
            ir_compile_settings: Vec::new(),
        }
    }

    pub fn add_chunk(
        &mut self,
        settings: CompileSettings,
        chunk_name: impl Into<vm::SharedStr>,
        code: &str,
    ) -> Result<(), CompileError> {
        self.preprocessor.add_chunk(
            &mut self.string_interner,
            settings.parse,
            settings.export_top_level_functions,
            chunk_name,
            code,
        )?;

        self.ir_compile_settings.push(IrCompileSettings {
            ir_gen: settings.ir_gen,
            optimization_passes: settings.optimization_passes,
            verify_ir: settings.verify_ir,
        });

        Ok(())
    }

    pub fn add_lexed_chunk(&mut self, lexed_chunk: LexedChunk<S>, settings: CompileSettings) {
        self.preprocessor.add_lexed_chunk(
            lexed_chunk,
            settings.parse,
            settings.export_top_level_functions,
        );

        self.ir_compile_settings.push(IrCompileSettings {
            ir_gen: settings.ir_gen,
            optimization_passes: settings.optimization_passes,
            verify_ir: settings.verify_ir,
        });
    }

    pub fn chunk_len(&self) -> usize {
        self.preprocessor.chunk_len()
    }

    pub fn compile(
        self,
        config: S,
        external_macros: &MacroSet<S>,
        external_enums: &EnumSet<S>,
        external_var_mode: impl Fn(&S) -> Option<ExternalVarMode>,
    ) -> Result<CompileOutput<S>, CompileError> {
        let Self {
            preprocessor,
            mut string_interner,
            ir_compile_settings,
        } = self;

        let PreprocessOutput {
            preprocessed_chunks,
            macros,
            enums,
            exports,
            export_chunk_indexes,
            ..
        } = preprocessor.preprocess(config, external_macros, external_enums, |ident| {
            external_var_mode(ident).is_some()
        })?;

        let mut global_vars = FxHashSet::default();
        for export in exports.iter() {
            if let Export::GlobalVar(ident) = export {
                global_vars.insert(ident.inner.clone());
            }
        }

        assert_eq!(preprocessed_chunks.len(), ir_compile_settings.len());
        let compiling_chunks = preprocessed_chunks
            .into_iter()
            .zip(ir_compile_settings.into_iter())
            .map(|((block, chunk), compile_settings)| (chunk, block, compile_settings))
            .collect::<Vec<_>>();

        // Internally defined exported functions and globals also have special var modes in addition
        // to externally defined ones.

        let var_mode = |ident: &S| {
            if let Some(sm) = external_var_mode(ident) {
                Some(sm)
            } else if let Some(idx) = exports.find(ident) {
                Some(match exports.get(idx).unwrap() {
                    Export::Function(_) => ExternalVarMode::Magic { is_read_only: true },
                    Export::GlobalVar(_) => ExternalVarMode::Global,
                })
            } else {
                None
            }
        };

        // No declaration is permitted that shadows an enum name.

        let is_reserved = |ident: &S| enums.find(ident).is_some();

        // Set up a global dictionary for magic variable names to a unique MagicIdx

        let mut magic_var_list = Vec::new();
        let mut magic_index_dict: FxHashMap<S, usize> = FxHashMap::default();
        let mut get_magic_index = |ident: &S| {
            if matches!(var_mode(ident), Some(ExternalVarMode::Magic { .. })) {
                if let Some(&idx) = magic_index_dict.get(ident) {
                    Some(idx)
                } else {
                    let idx = magic_var_list.len();
                    magic_var_list.push(ident.clone());
                    magic_index_dict.insert(ident.clone(), idx);
                    Some(idx)
                }
            } else {
                None
            }
        };

        let mut optimize_and_generate_proto =
            |compile_settings: IrCompileSettings, ir: &mut ir::Function<S>| -> Prototype<S> {
                if compile_settings.verify_ir {
                    if let Err(err) = verify_ir(ir) {
                        panic!("Internal IR Generation Error: {err}\nIR: {ir:?}");
                    }
                }

                for _ in 0..compile_settings.optimization_passes {
                    optimize_ir(ir);
                }

                if compile_settings.verify_ir && compile_settings.optimization_passes != 0 {
                    if let Err(err) = verify_ir(ir) {
                        panic!("Internal IR Optimization Error: {err}\nIR: {ir:?}");
                    }
                }

                match gen_prototype(&ir, &mut get_magic_index) {
                    Ok(proto) => proto,
                    Err(err) => {
                        panic!("Internal Codegen Error: {err}\nIR: {ir:?}");
                    }
                }
            };

        // Compile all exported functions

        let mut exported_functions = FxHashMap::default();

        for (i, export) in exports.iter().enumerate() {
            let chunk_index = match export_chunk_indexes.binary_search_by(|j| j.cmp(&i)) {
                Ok(i) => i,
                Err(i) => i.checked_sub(1).unwrap(),
            };
            let (ref chunk, _, compile_settings) = compiling_chunks[chunk_index];

            if let Export::Function(func_stmt) = export {
                let mut ir = compile_settings
                    .ir_gen
                    .gen_func_stmt_ir(
                        &mut string_interner,
                        func_stmt,
                        &CompilerVarDict {
                            is_reserved,
                            var_mode,
                        },
                    )
                    .map_err(|e| {
                        let line_number = chunk.line_numbers.line(e.span.start());
                        CompileError {
                            kind: CompileErrorKind::IrGen(e),
                            chunk_name: chunk.name.clone(),
                            line_number,
                        }
                    })?;

                let prototype = optimize_and_generate_proto(compile_settings, &mut ir);

                exported_functions.insert(
                    func_stmt.name.inner.clone(),
                    FunctionOutput {
                        chunk_index,
                        ir,
                        prototype,
                    },
                );
            }
        }

        // Compile the top-level chunks

        let mut chunks = Vec::new();

        for (chunk, block, compile_settings) in compiling_chunks {
            let mut ir = compile_settings
                .ir_gen
                .gen_chunk_ir(
                    &mut string_interner,
                    &block,
                    &CompilerVarDict {
                        is_reserved,
                        var_mode,
                    },
                )
                .map_err(|e| {
                    let line_number = chunk.line_numbers.line(e.span.start());
                    CompileError {
                        kind: CompileErrorKind::IrGen(e),
                        chunk_name: chunk.name.clone(),
                        line_number,
                    }
                })?;

            let prototype = optimize_and_generate_proto(compile_settings, &mut ir);
            let func_output = FunctionOutput {
                chunk_index: chunks.len(),
                ir,
                prototype,
            };
            chunks.push((chunk, func_output));
        }

        Ok(CompileOutput {
            macros,
            enums,
            global_vars,
            magic_vars: magic_var_list,
            exported_functions,
            chunks,
        })
    }
}

/// The compiler output for a single top-level function.
pub struct FunctionOutput<S> {
    /// The index for the source chunk which defines this function.
    pub chunk_index: usize,
    /// IR for the function *after* any optimization.
    pub ir: ir::Function<S>,
    /// Prototype generated from the IR.
    pub prototype: Prototype<S>,
}

/// All output from a single compilation unit.
pub struct CompileOutput<S> {
    pub macros: MacroSet<S>,
    pub enums: EnumSet<S>,

    /// All variables declared as globals with `globalvar`.
    pub global_vars: FxHashSet<S>,

    /// A dictionary for the name of magic variables referenced by instructions in all prototypes.
    pub magic_vars: Vec<S>,

    /// Contains all top-level function exports in this compilation unit. All references to any
    /// exported function are always interpreted as references to magic variables.
    pub exported_functions: FxHashMap<S, FunctionOutput<S>>,

    /// A pair of the chunk identifier and function output per input chunk, in the order provided to
    /// the [`Compiler`].
    pub chunks: Vec<(SourceChunk, FunctionOutput<S>)>,
}

#[derive(Debug, Error)]
#[error("prototype references magic var {0:?} which is not in the extern lib or function exports")]
pub struct MissingMagicVar(vm::SharedStr);

impl<S> CompileOutput<S> {
    /// A version of [`CompileOutput::vm_prototypes`] which allows converting string types to the
    /// required [`vm::String`].
    pub fn vm_prototypes_with_strings<'gc>(
        &self,
        ctx: vm::Context<'gc>,
        extern_lib: vm::MagicSet<'gc>,
        into_vm_string: impl Fn(&S) -> vm::String<'gc>,
    ) -> Result<(Gc<'gc, vm::MagicSet<'gc>>, Vec<Gc<'gc, vm::Prototype<'gc>>>), MissingMagicVar>
    {
        let mut new_magic = extern_lib;

        // Gather all chunk descriptors

        let chunks = self
            .chunks
            .iter()
            .map(|(chunk, _)| chunk.clone().into_vm(&ctx))
            .collect::<Vec<_>>();

        // Gather all magic variable names

        let magic_vars = self
            .magic_vars
            .iter()
            .map(&into_vm_string)
            .collect::<Vec<_>>();

        // Insert a read-only *stub* magic variable for each function export

        let mut exported_function_magic_indexes = IndexMap::new();

        let stub_magic = vm::MagicConstant::new_ptr(&ctx, vm::Value::Undefined);

        for (i, name) in self.exported_functions.keys().enumerate() {
            let index = new_magic.insert(into_vm_string(name), stub_magic).0;
            exported_function_magic_indexes.insert(i, index);
        }

        // Convert all exported functions into VM prototypes and replace the stub magic variables

        let new_magic = Gc::new(&ctx, new_magic);
        let magic_write = Gc::write(&ctx, new_magic);

        for (i, output) in self.exported_functions.values().enumerate() {
            let magic_index = exported_function_magic_indexes[i];

            let proto = output
                .prototype
                .clone_with_map_string(&into_vm_string)
                .map_magic_idx(|idx| {
                    new_magic
                        .find(magic_vars[idx.index()])
                        .unwrap()
                        .try_into()
                        .unwrap()
                });

            let vm_proto = proto.into_vm(&ctx, chunks[output.chunk_index], new_magic);
            let closure = vm::Closure::new(&ctx, vm_proto, vm::Value::Undefined).unwrap();

            vm::MagicSet::replace(
                magic_write,
                magic_index,
                vm::MagicConstant::new_ptr(&ctx, closure),
            )
            .unwrap();
        }

        // Convert all chunk prototypes into VM prototypes

        let mut chunk_prototypes = Vec::new();

        for (_, output) in &self.chunks {
            let proto = output
                .prototype
                .clone_with_map_string(&into_vm_string)
                .map_magic_idx(|idx| {
                    new_magic
                        .find(magic_vars[idx.index()])
                        .unwrap()
                        .try_into()
                        .unwrap()
                });
            let vm_proto = proto.into_vm(&ctx, chunks[output.chunk_index], new_magic);
            chunk_prototypes.push(vm_proto);
        }

        Ok((new_magic, chunk_prototypes))
    }
}

impl<'gc> CompileOutput<vm::String<'gc>> {
    /// Convert compiler output into a set of [`vm::Prototype`]s.
    ///
    /// A new [`vm::MagicSet`] is created which merges all exported functions on top of the
    /// provided external `MagicSet`. This new `MagicSet` will become the one used for every new
    /// `vm::Prototype`.
    ///
    /// Returns a [`MissingMagicVar`] error if the provided `extern_lib` does not contain a magic
    /// variable with a name that was declared as magic during compilation.
    ///
    /// # Panics
    ///
    /// May panic if stored data was externally modified after compilation.
    pub fn vm_prototypes(
        &self,
        ctx: vm::Context<'gc>,
        extern_lib: vm::MagicSet<'gc>,
    ) -> Result<(Gc<'gc, vm::MagicSet<'gc>>, Vec<Gc<'gc, vm::Prototype<'gc>>>), MissingMagicVar>
    {
        self.vm_prototypes_with_strings(ctx, extern_lib, |s| *s)
    }
}

#[derive(Debug, Error)]
pub enum IrVerificationError {
    #[error("{0}")]
    ReferenceVerification(#[from] ReferenceVerificationError),
    #[error("{0}")]
    UpVarVerification(#[from] UpVarVerificationError),
    #[error("{0}")]
    InstructionVerification(#[from] InstructionVerificationError),
    #[error("{0}")]
    ShadowVerification(#[from] ShadowVerificationError),
    #[error("{0}")]
    VariableVerification(#[from] VariableVerificationError),
    #[error("{0}")]
    ThisScopeVerification(#[from] ThisScopeVerificationError),
    #[error("{0}")]
    CallScopeVerification(#[from] CallScopeVerificationError),
}

/// Verify that IR is well-formed.
pub fn verify_ir<S: Clone>(ir: &ir::Function<S>) -> Result<(), IrVerificationError> {
    fn inner_verify_ir<S: Clone>(ir: &ir::Function<S>) -> Result<(), IrVerificationError> {
        verify_references(ir)?;
        verify_upvars(ir)?;
        InstructionLiveness::compute(ir)?;
        ShadowLiveness::compute(ir)?;
        VariableLiveness::compute(ir)?;
        ThisScopeLiveness::compute(ir)?;
        CallScopeLiveness::compute(ir)?;

        for func in ir.functions.values() {
            inner_verify_ir(func)?;
        }

        Ok(())
    }

    verify_no_root_upvars(ir)?;
    inner_verify_ir(ir)?;

    Ok(())
}

/// Run optimization passes on IR.
///
/// # Panics
///
/// May panic if the provided IR is not well-formed.
pub fn optimize_ir<S: Eq + Clone>(ir: &mut ir::Function<S>) {
    // Optimize all child functions first, which may remove variable references to this parent
    // function, allowing for more SSA conversion.
    for func in ir.functions.values_mut() {
        optimize_ir(func);
    }

    convert_to_ssa(ir);
    reduce_shadows(ir).unwrap();
    fold_constants(ir);
    eliminate_copies(ir);
    simplify_branches(ir);
    eliminate_dead_code(ir);

    clean_unreachable_blocks(ir);
    block_branch_to_jump(ir);
    redirect_empty_blocks(ir);
    merge_blocks(ir);

    clean_unreachable_blocks(ir);
    clean_unused_functions(ir);
    clean_unused_variables(ir);
    clean_unused_shadow_vars(ir);
    clean_unused_this_scopes(ir);
    clean_unused_call_scopes(ir);
    clean_instructions(ir);
}

#[derive(Debug, Copy, Clone)]
struct IrCompileSettings {
    ir_gen: IrGenSettings,
    optimization_passes: u8,
    verify_ir: bool,
}

struct CompilerVarDict<R, M> {
    is_reserved: R,
    var_mode: M,
}

impl<S, R, M> VarDict<S> for CompilerVarDict<R, M>
where
    R: Fn(&S) -> bool,
    M: Fn(&S) -> Option<ExternalVarMode>,
{
    fn is_reserved(&self, name: &S) -> bool {
        (self.is_reserved)(name)
    }

    fn free_var_mode(&self, ident: &S) -> FreeVarMode {
        match (self.var_mode)(ident) {
            Some(ExternalVarMode::Global) => FreeVarMode::GlobalVar,
            Some(ExternalVarMode::Magic { is_read_only }) => FreeVarMode::Magic { is_read_only },
            None => FreeVarMode::This,
        }
    }
}
