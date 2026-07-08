#[macro_use]
mod instruction;
mod bytecode;

pub use self::{
    bytecode::{ByteCode, ByteCodeEncodingError, Dispatch, Dispatcher},
    instruction::{
        ConstIdx, HeapIdx, IndexType, InstIdx, Instruction, MagicIdx, ProtoIdx, RegIdx, StackIdx,
    },
};
