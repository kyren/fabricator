use fabricator_vm as vm;
use gc_arena::{Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable};

use crate::{
    enums::EnumSet,
    frontend::{CompileError, CompileSettings, Compiler, ExternalVarMode},
    macros::MacroSet,
    string_interner::VmInterner,
};

#[derive(Debug, Copy, Clone, Collect)]
#[collect(no_drop)]
pub struct ChunkImports<'gc> {
    pub macros: Gc<'gc, MacroSet<vm::String<'gc>>>,
    pub enums: Gc<'gc, EnumSet<vm::String<'gc>>>,
    pub global_vars: Gc<'gc, vm::StringSet<'gc>>,
    pub magic: Gc<'gc, vm::MagicSet<'gc>>,
}

impl<'gc> ChunkImports<'gc> {
    pub fn with_magic(mc: &Mutation<'gc>, magic: Gc<'gc, vm::MagicSet<'gc>>) -> Self {
        Self {
            macros: Gc::new(mc, MacroSet::new()),
            enums: Gc::new(mc, EnumSet::new()),
            global_vars: Gc::new(mc, vm::StringSet::default()),
            magic,
        }
    }
}

#[derive(Clone)]
pub struct StashedChunkImports {
    macros: DynamicRoot<Rootable![MacroSet<vm::String<'_>>]>,
    enums: DynamicRoot<Rootable![EnumSet<vm::String<'_>>]>,
    global_vars: DynamicRoot<Rootable![vm::StringSet<'_>]>,
    magic: vm::StashedMagicSet,
}

impl<'gc> vm::Stashable<'gc> for ChunkImports<'gc> {
    type Stashed = StashedChunkImports;

    fn stash(self, mc: &Mutation<'gc>, roots: DynamicRootSet<'gc>) -> Self::Stashed {
        StashedChunkImports {
            macros: roots.stash::<Rootable![MacroSet<vm::String<'_>>]>(mc, self.macros),
            enums: roots.stash::<Rootable![EnumSet<vm::String<'_>>]>(mc, self.enums),
            global_vars: roots.stash::<Rootable![vm::StringSet<'_>]>(mc, self.global_vars),
            magic: vm::Stashable::stash(self.magic, mc, roots),
        }
    }
}

impl vm::Fetchable for StashedChunkImports {
    type Fetched<'gc> = ChunkImports<'gc>;

    fn fetch<'gc>(&self, roots: DynamicRootSet<'gc>) -> ChunkImports<'gc> {
        ChunkImports {
            macros: roots.fetch(&self.macros),
            enums: roots.fetch(&self.enums),
            global_vars: roots.fetch(&self.global_vars),
            magic: self.magic.fetch(roots),
        }
    }
}

/// Compile a single chunk.
///
/// Returns the chunk prototype as well as a merged `ChunkImports` set.
pub fn compile_chunk<'gc>(
    ctx: vm::Context<'gc>,
    config: &str,
    imports: ChunkImports<'gc>,
    compile_settings: CompileSettings,
    chunk_name: vm::SharedStr,
    code: &str,
) -> Result<(Gc<'gc, vm::Prototype<'gc>>, ChunkImports<'gc>), CompileError> {
    let mut compiler = Compiler::new(VmInterner::new(ctx));

    compiler.add_chunk(compile_settings, chunk_name, code)?;
    let output = compiler.compile(ctx.intern(config), &imports.macros, &imports.enums, |&s| {
        if imports.global_vars.contains(&s) {
            Some(ExternalVarMode::Global)
        } else if let Some(idx) = imports.magic.find(s) {
            Some(ExternalVarMode::Magic {
                is_read_only: imports.magic.get(idx).unwrap().read_only(),
            })
        } else {
            None
        }
    })?;

    let (magic, mut chunk_protos) = output.vm_prototypes(ctx, (*imports.magic).clone()).unwrap();
    assert_eq!(chunk_protos.len(), 1);
    let chunk_proto = chunk_protos.remove(0);

    let mut macros = (*imports.macros).clone();
    macros.merge(output.macros);

    let mut enums = (*imports.enums).clone();
    enums.merge(output.enums);

    let mut global_vars = (*imports.global_vars).clone();
    global_vars.extend(output.global_vars.iter().cloned());

    let exported_imports = ChunkImports {
        macros: Gc::new(&ctx, macros),
        enums: Gc::new(&ctx, enums),
        global_vars: Gc::new(&ctx, global_vars),
        magic,
    };

    Ok((chunk_proto, exported_imports))
}

pub struct ChunkOutput<'gc> {
    pub exported_imports: ChunkImports<'gc>,
    pub chunk_prototype: Gc<'gc, vm::Prototype<'gc>>,
}
