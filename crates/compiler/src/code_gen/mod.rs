mod generate;
mod heap_alloc;
mod prototype;
mod register_alloc;
mod upsilon_reachability;

pub use self::{
    generate::gen_prototype,
    prototype::{HeapVarDescriptor, Prototype},
};

use fabricator_vm::instructions;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoGenError {
    #[error("{0}")]
    ByteCodeEncoding(#[from] instructions::ByteCodeEncodingError),
    #[error("too many registers used")]
    RegisterOverflow,
    #[error("too many heap variables used")]
    HeapVarOverflow,
    #[error("too many constants used")]
    ConstantOverflow,
    #[error("too many sub-functions")]
    PrototypeOverflow,
    #[error("stack index out of range")]
    StackIndexOutOfRange,
    #[error("missing magic value")]
    NoSuchMagic,
    #[error("magic value index too large")]
    MagicIndexOutOfRange,
}
