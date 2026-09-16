pub mod analysis;
pub mod ast;
pub mod chunk_compiler;
pub mod code_gen;
pub mod constant;
pub mod enums;
pub mod exports;
pub mod frontend;
pub mod graph;
pub mod ir;
pub mod ir_gen;
pub mod lexer;
pub mod line_numbers;
pub mod macros;
pub mod parser;
pub mod preprocessing;
pub mod string_interner;
pub mod tokens;

pub use self::{
    chunk_compiler::{ChunkImports, ChunkOutput, StashedChunkImports, compile_chunk},
    frontend::{CompileError, CompileSettings},
};
