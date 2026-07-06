use std::path::Path;

use fabricator_util::index_containers::IndexMap;
use fabricator_vm as vm;
use gc_arena::{Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable};
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
        verify_arguments::{ArgumentVerificationError, verify_arguments},
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
        Preprocessor, ShadowsSpecialError,
    },
    string_interner::VmInterner,
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

#[derive(Debug, Error)]
pub enum IrVerificationError {
    #[error("{0}")]
    ReferenceVerification(#[from] ReferenceVerificationError),
    #[error("{0}")]
    ArgumentVerification(#[from] ArgumentVerificationError),
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
        verify_arguments(ir)?;
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
    clean_instructions(ir);

    block_branch_to_jump(ir);
    redirect_empty_blocks(ir);
    merge_blocks(ir);

    clean_unreachable_blocks(ir);
    clean_unused_variables(ir);
    clean_unused_shadow_vars(ir);
    clean_unused_this_scopes(ir);
    clean_unused_call_scopes(ir);
    clean_unused_functions(ir);
}

/// Items shared across compilation units.
///
/// These will be accumulated by the compiler during a compilation unit and are part of the compiler
/// output.
///
/// These can be shared across different instances of a [`Compiler`] to control sharing between
/// different logical sets of FML scripts (compilation units).
#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct ImportItems<'gc> {
    pub macros: Gc<'gc, MacroSet<vm::String<'gc>>>,
    pub enums: Gc<'gc, EnumSet<vm::String<'gc>>>,
    pub global_vars: Gc<'gc, vm::StringSet<'gc>>,
    pub magic: Gc<'gc, vm::MagicSet<'gc>>,
}

impl<'gc> ImportItems<'gc> {
    pub fn with_magic(mc: &Mutation<'gc>, stdlib: Gc<'gc, vm::MagicSet<'gc>>) -> Self {
        Self {
            macros: Gc::new(mc, MacroSet::new()),
            enums: Gc::new(mc, EnumSet::new()),
            global_vars: Gc::new(mc, vm::StringSet::default()),
            magic: stdlib,
        }
    }
}

#[derive(Clone)]
pub struct StashedImportItems {
    macros: DynamicRoot<Rootable![MacroSet<vm::String<'_>>]>,
    enums: DynamicRoot<Rootable![EnumSet<vm::String<'_>>]>,
    global_vars: DynamicRoot<Rootable![vm::StringSet<'_>]>,
    magic: vm::StashedMagicSet,
}

impl<'gc> vm::Stashable<'gc> for ImportItems<'gc> {
    type Stashed = StashedImportItems;

    fn stash(self, mc: &Mutation<'gc>, roots: DynamicRootSet<'gc>) -> Self::Stashed {
        StashedImportItems {
            macros: roots.stash::<Rootable![MacroSet<vm::String<'_>>]>(mc, self.macros),
            enums: roots.stash::<Rootable![EnumSet<vm::String<'_>>]>(mc, self.enums),
            global_vars: roots.stash::<Rootable![vm::StringSet<'_>]>(mc, self.global_vars),
            magic: vm::Stashable::stash(self.magic, mc, roots),
        }
    }
}

impl vm::Fetchable for StashedImportItems {
    type Fetched<'gc> = ImportItems<'gc>;

    fn fetch<'gc>(&self, roots: DynamicRootSet<'gc>) -> ImportItems<'gc> {
        ImportItems {
            macros: roots.fetch(&self.macros),
            enums: roots.fetch(&self.enums),
            global_vars: roots.fetch(&self.global_vars),
            magic: self.magic.fetch(roots),
        }
    }
}

/// Compile FML code.
///
/// Compiles separate code units together in multiple phases to allow for interdependencies.
pub struct Compiler<'gc> {
    ctx: vm::Context<'gc>,
    preprocessor: Preprocessor<'gc>,
    global_vars: vm::StringSet<'gc>,
    magic: vm::MagicSet<'gc>,
    ir_compile_settings: Vec<IrCompileSettings>,
}

impl<'gc> Compiler<'gc> {
    /// Compile a single chunk.
    ///
    /// Returns the chunk prototype as well as a merged `ImportItems` set. This is a convenience
    /// method for creating a `Compiler` instance and compiling only a single chunk.
    pub fn compile_chunk(
        ctx: vm::Context<'gc>,
        config: impl Into<String>,
        imports: ImportItems<'gc>,
        compile_settings: CompileSettings,
        chunk_name: impl Into<vm::SharedStr>,
        code: &str,
    ) -> Result<ChunkOutput<'gc>, CompileError> {
        let mut this = Self::new(ctx, config, imports);
        this.add_chunk(compile_settings, chunk_name, code)?;
        let output = this.compile()?;
        Ok(ChunkOutput {
            exported_imports: output.exported_imports,
            chunk_prototype: output.chunks[0],
            all_prototypes: output.all_prototypes,
        })
    }

    pub fn new(
        ctx: vm::Context<'gc>,
        config: impl Into<String>,
        imports: ImportItems<'gc>,
    ) -> Self {
        let macros = imports.macros.as_ref().clone();
        let enums = imports.enums.as_ref().clone();
        let global_vars = imports.global_vars.as_ref().clone();
        let magic = imports.magic.as_ref().clone();

        let preprocessor = Preprocessor::new(ctx, config, macros, enums);

        Self {
            ctx,
            preprocessor,
            global_vars,
            magic,
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

    pub fn add_lexed_chunk(&mut self, lexed_chunk: LexedChunk<'gc>, settings: CompileSettings) {
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

    pub fn compile(self) -> Result<CompileOutput<'gc>, CompileError> {
        fn optimize_and_generate_proto<'gc>(
            compile_settings: IrCompileSettings,
            ir: &mut ir::Function<vm::String<'gc>>,
            magic: &vm::MagicSet<'gc>,
        ) -> Prototype<vm::String<'gc>> {
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

            match gen_prototype(&ir, |n| magic.find(*n)) {
                Ok(proto) => proto,
                Err(err) => {
                    panic!("Internal Codegen Error: {err}\nIR: {ir:?}");
                }
            }
        }

        let Self {
            ctx,
            preprocessor,
            mut global_vars,
            mut magic,
            ir_compile_settings,
        } = self;

        let PreprocessOutput {
            preprocessed_chunks,
            macros,
            enums,
            exports,
            export_chunk_indexes,
            ..
        } = preprocessor
            .preprocess(|ident| global_vars.contains(&ident) || magic.find(ident).is_some())?;

        assert_eq!(preprocessed_chunks.len(), ir_compile_settings.len());
        let compiling_chunks = preprocessed_chunks
            .into_iter()
            .zip(ir_compile_settings.into_iter())
            .map(|((block, chunk), compile_settings)| (chunk, block, compile_settings))
            .collect::<Vec<_>>();

        // Produce a read-only *stub* magic variable for each export.

        // The magic index for each exported item.
        let mut export_magic_indexes = IndexMap::new();

        let stub_magic = vm::MagicConstant::new_ptr(&ctx, vm::Value::Undefined);

        for (i, export) in exports.iter().enumerate() {
            let export_name = export.name();

            match export {
                Export::Function(_) => {
                    let index = magic.insert(export_name.clone(), stub_magic).0;
                    export_magic_indexes.insert(i, index);
                }
                Export::GlobalVar(ident) => {
                    global_vars.insert(ident.inner);
                }
            }
        }

        let magic = Gc::new(&ctx, magic);
        let magic_write = Gc::write(&ctx, magic);

        let mut all_prototypes = Vec::new();

        // Compile each exported function and place the result into the reserved stub magic
        // variable.

        for (i, export) in exports.iter().enumerate() {
            let chunk_index = match export_chunk_indexes.binary_search_by(|j| j.cmp(&i)) {
                Ok(i) => i,
                Err(i) => i.checked_sub(1).unwrap(),
            };
            let (chunk, _, compile_settings) = compiling_chunks[chunk_index];

            if let Export::Function(func_stmt) = export {
                let magic_index = export_magic_indexes[i];

                let mut ir = compile_settings
                    .ir_gen
                    .gen_func_stmt_ir(
                        &mut VmInterner::new(ctx),
                        func_stmt,
                        &CompilerVarDict {
                            enums: &enums,
                            global_vars: &global_vars,
                            magic: &magic,
                        },
                    )
                    .map_err(|e| {
                        let line_number = chunk.line_number(e.span.start());
                        CompileError {
                            kind: CompileErrorKind::IrGen(e),
                            chunk_name: chunk.name().clone(),
                            line_number,
                        }
                    })?;

                let proto = optimize_and_generate_proto(compile_settings, &mut ir, &magic);
                let vm_proto = proto.into_vm(&ctx, chunk, magic);
                let closure = vm::Closure::new(&ctx, vm_proto, vm::Value::Undefined).unwrap();

                all_prototypes.push((ir, vm_proto));

                vm::MagicSet::replace(
                    magic_write,
                    magic_index,
                    vm::MagicConstant::new_ptr(&ctx, closure),
                )
                .unwrap();
            }
        }

        // Compile the top-level chunks

        let mut chunks = Vec::new();

        for (chunk, block, compile_settings) in compiling_chunks {
            let mut ir = compile_settings
                .ir_gen
                .gen_chunk_ir(
                    &mut VmInterner::new(self.ctx),
                    &block,
                    &CompilerVarDict {
                        enums: &enums,
                        global_vars: &global_vars,
                        magic: &magic,
                    },
                )
                .map_err(|e| {
                    let line_number = chunk.line_number(e.span.start());
                    CompileError {
                        kind: CompileErrorKind::IrGen(e),
                        chunk_name: chunk.name().clone(),
                        line_number,
                    }
                })?;

            let proto = optimize_and_generate_proto(compile_settings, &mut ir, &magic);
            let vm_proto = proto.into_vm(&ctx, chunk, magic);
            all_prototypes.push((ir, vm_proto));
            chunks.push(vm_proto);
        }

        let imports = ImportItems {
            macros: Gc::new(&ctx, macros),
            enums: Gc::new(&ctx, enums),
            global_vars: Gc::new(&ctx, global_vars),
            magic,
        };

        Ok(CompileOutput {
            exported_imports: imports,
            chunks,
            all_prototypes,
        })
    }
}

pub struct CompileOutput<'gc> {
    /// Provided `ImportItems` merged with all of the exports in the current compilation unit, may
    /// be used in subsequent compilation units.
    pub exported_imports: ImportItems<'gc>,

    /// One generated prototype per input chunk, in the order provided to [`Compiler`].
    pub chunks: Vec<Gc<'gc, vm::Prototype<'gc>>>,

    /// A prototype for every input chunk and function export, paired with the final IR used to
    /// generate the prototype.
    pub all_prototypes: Vec<(ir::Function<vm::String<'gc>>, Gc<'gc, vm::Prototype<'gc>>)>,
}

/// A version of [`CompilerOutput`] for a single chunk.
pub struct ChunkOutput<'gc> {
    pub exported_imports: ImportItems<'gc>,
    pub chunk_prototype: Gc<'gc, vm::Prototype<'gc>>,
    pub all_prototypes: Vec<(ir::Function<vm::String<'gc>>, Gc<'gc, vm::Prototype<'gc>>)>,
}

#[derive(Debug, Copy, Clone)]
struct IrCompileSettings {
    ir_gen: IrGenSettings,
    optimization_passes: u8,
    verify_ir: bool,
}

struct CompilerVarDict<'gc, 'a> {
    enums: &'a EnumSet<vm::String<'gc>>,
    global_vars: &'a vm::StringSet<'gc>,
    magic: &'a vm::MagicSet<'gc>,
}

impl<'gc, 'a> VarDict<vm::String<'gc>> for CompilerVarDict<'gc, 'a> {
    fn is_reserved(&self, name: &vm::String<'gc>) -> bool {
        self.enums.find(name).is_some()
    }

    fn free_var_mode(&self, ident: &vm::String<'gc>) -> FreeVarMode {
        if let Some(index) = self.magic.find(*ident) {
            FreeVarMode::Magic {
                is_read_only: self.magic.get(index).unwrap().read_only(),
            }
        } else if self.global_vars.contains(ident) {
            FreeVarMode::GlobalVar
        } else {
            FreeVarMode::This
        }
    }
}
