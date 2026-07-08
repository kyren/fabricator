use std::{collections::hash_map, hash::Hash};

use fabricator_util::typed_id_map::SecondaryMap;
use fabricator_vm::{
    self as vm,
    instructions::{self, InstIdx, Instruction},
};
use rustc_hash::FxHashMap;

use crate::{
    analysis::{
        instruction_liveness::InstructionLiveness, shadow_liveness::ShadowLiveness,
        variable_liveness::VariableLiveness,
    },
    code_gen::{
        ProtoGenError, heap_alloc::HeapAllocation, prototype::Prototype,
        register_alloc::RegisterAllocation,
    },
    constant::Constant,
    graph::dfs::topological_order,
    ir,
};

/// Generate a [`Prototype`] from IR.
///
/// # Panics
///
/// May panic if the provided IR is not well-formed.
pub fn gen_prototype<S: Clone + Eq + Hash>(
    ir: &ir::Function<S>,
    magic_index: impl Fn(&S) -> Option<usize>,
) -> Result<Prototype<S>, ProtoGenError> {
    codegen_function(ir, &magic_index, &SecondaryMap::new())
}

fn codegen_function<S: Clone + Eq + Hash>(
    ir: &ir::Function<S>,
    magic_index: &impl Fn(&S) -> Option<usize>,
    parent_heap_indexes: &SecondaryMap<ir::VarId, instructions::HeapIdx>,
) -> Result<Prototype<S>, ProtoGenError> {
    let instruction_liveness = InstructionLiveness::compute(ir).unwrap();
    let shadow_liveness = ShadowLiveness::compute(ir).unwrap();
    let variable_liveness = VariableLiveness::compute(ir).unwrap();

    let reg_alloc = RegisterAllocation::allocate(ir, &instruction_liveness, &shadow_liveness)?;
    let heap_alloc = HeapAllocation::allocate(ir, &variable_liveness, parent_heap_indexes)?;

    let mut prototypes = Vec::new();
    let mut prototype_indexes: SecondaryMap<ir::FuncId, instructions::ProtoIdx> =
        SecondaryMap::new();
    for (func_id, func) in ir.functions.iter() {
        prototype_indexes.insert(
            func_id,
            prototypes
                .len()
                .try_into()
                .map_err(|_| ProtoGenError::PrototypeOverflow)?,
        );
        prototypes.push(codegen_function(
            func,
            magic_index,
            &heap_alloc.heap_indexes,
        )?);
    }

    let mut constants = Vec::new();
    let mut constant_indexes = FxHashMap::<Constant<S>, instructions::ConstIdx>::default();

    let mut get_const_index = |c: &Constant<S>| -> Result<instructions::ConstIdx, ProtoGenError> {
        Ok(match constant_indexes.entry(c.clone()) {
            hash_map::Entry::Vacant(vacant) => {
                let idx = constants
                    .len()
                    .try_into()
                    .map_err(|_| ProtoGenError::ConstantOverflow)?;
                vacant.insert(idx);
                constants.push(c.clone());
                idx
            }
            hash_map::Entry::Occupied(occupied) => *occupied.get(),
        })
    };

    let block_order = topological_order(ir.start_block, |id| ir.blocks[id].exit.kind.successors());

    let block_order_indexes: FxHashMap<ir::BlockId, usize> = block_order
        .iter()
        .copied()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();

    let mut vm_instructions = Vec::new();
    let mut block_vm_starts = SecondaryMap::<ir::BlockId, usize>::new();
    let mut block_vm_jumps = Vec::new();

    for (order_index, &block_id) in block_order.iter().enumerate() {
        let block = &ir.blocks[block_id];
        block_vm_starts.insert(block_id, vm_instructions.len());

        let mut inst_iter = block.instructions.iter().enumerate().peekable();

        while let Some((inst_index, &inst_id)) = inst_iter.next() {
            let inst = &ir.instructions[inst_id];

            match inst.kind {
                ir::InstructionKind::PushStack(first_call_scope, first_source) => {
                    // Consolidate all consecutive `PushStack` instructions of the same call scope
                    // into minimal instructions.

                    let mut sources = vec![(first_source, inst.span)];
                    while let Some(&(_, &inst_id)) = inst_iter.peek() {
                        let inst = &ir.instructions[inst_id];
                        let ir::InstructionKind::PushStack(scope, source) = inst.kind else {
                            break;
                        };
                        if scope != first_call_scope {
                            break;
                        }
                        inst_iter.next();
                        sources.push((source, inst.span));
                    }

                    for chunk in sources.chunks(4) {
                        match *chunk {
                            [] => {}
                            [(a, aspan)] => {
                                vm_instructions.push((
                                    Instruction::StackPush {
                                        source: reg_alloc.instruction_registers[a],
                                    },
                                    aspan,
                                ));
                            }
                            [(a, aspan), (b, bspan)] => {
                                vm_instructions.push((
                                    Instruction::StackPush2 {
                                        source_a: reg_alloc.instruction_registers[a],
                                        source_b: reg_alloc.instruction_registers[b],
                                    },
                                    aspan.combine(bspan),
                                ));
                            }
                            [(a, aspan), (b, bspan), (c, cspan)] => {
                                vm_instructions.push((
                                    Instruction::StackPush3 {
                                        source_a: reg_alloc.instruction_registers[a],
                                        source_b: reg_alloc.instruction_registers[b],
                                        source_c: reg_alloc.instruction_registers[c],
                                    },
                                    aspan.combine(bspan).combine(cspan),
                                ));
                            }
                            [(a, aspan), (b, bspan), (c, cspan), (d, dspan)] => {
                                vm_instructions.push((
                                    Instruction::StackPush4 {
                                        source_a: reg_alloc.instruction_registers[a],
                                        source_b: reg_alloc.instruction_registers[b],
                                        source_c: reg_alloc.instruction_registers[c],
                                        source_d: reg_alloc.instruction_registers[d],
                                    },
                                    aspan.combine(bspan).combine(cspan).combine(dspan),
                                ));
                            }
                            _ => unreachable!(),
                        }
                    }
                }
                ir::InstructionKind::NoOp => {}
                ir::InstructionKind::Copy(source) => {
                    let dest_reg = reg_alloc.instruction_registers[inst_id];
                    let source_reg = reg_alloc.instruction_registers[source];
                    if dest_reg != source_reg {
                        vm_instructions.push((
                            Instruction::Copy {
                                dest: dest_reg,
                                source: source_reg,
                            },
                            inst.span,
                        ));
                    }
                }
                ir::InstructionKind::Constant(ref c) => {
                    let dest = reg_alloc.instruction_registers[inst_id];
                    let vm_inst = match *c {
                        Constant::Undefined => Instruction::Undefined { dest },
                        Constant::Boolean(value) => Instruction::Boolean { dest, value },
                        _ => Instruction::LoadConstant {
                            dest: reg_alloc.instruction_registers[inst_id],
                            constant: get_const_index(c)?,
                        },
                    };
                    vm_instructions.push((vm_inst, inst.span));
                }
                ir::InstructionKind::Closure { func, bind_this } => {
                    vm_instructions.push((
                        Instruction::Closure {
                            dest: reg_alloc.instruction_registers[inst_id],
                            proto: prototype_indexes[func],
                            bind_this,
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::OpenVariable(_) => {
                    // `OpenVariable` is an ephemeral instruction used only to allocate a heap
                    // index.
                }
                ir::InstructionKind::GetVariable(var) => {
                    vm_instructions.push((
                        Instruction::GetHeap {
                            dest: reg_alloc.instruction_registers[inst_id],
                            heap: heap_alloc.heap_indexes[var],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::SetVariable(dest, source) => {
                    vm_instructions.push((
                        Instruction::SetHeap {
                            heap: heap_alloc.heap_indexes[dest],
                            source: reg_alloc.instruction_registers[source],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::CloseVariable(var) => {
                    vm_instructions.push((
                        Instruction::ResetHeap {
                            heap: heap_alloc.heap_indexes[var],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::GetMagic(ref magic_var) => {
                    let magic_idx = magic_index(magic_var)
                        .ok_or(ProtoGenError::NoSuchMagic)?
                        .try_into()
                        .map_err(|_| ProtoGenError::MagicIndexOutOfRange)?;
                    vm_instructions.push((
                        Instruction::GetMagic {
                            dest: reg_alloc.instruction_registers[inst_id],
                            magic: magic_idx,
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::SetMagic(ref magic_var, source) => {
                    let magic_idx = magic_index(magic_var)
                        .ok_or(ProtoGenError::NoSuchMagic)?
                        .try_into()
                        .map_err(|_| ProtoGenError::MagicIndexOutOfRange)?;
                    vm_instructions.push((
                        Instruction::SetMagic {
                            magic: magic_idx,
                            source: reg_alloc.instruction_registers[source],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::Globals => {
                    vm_instructions.push((
                        Instruction::Globals {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::This => {
                    vm_instructions.push((
                        Instruction::This {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::Other => {
                    vm_instructions.push((
                        Instruction::Other {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::CurrentClosure => {
                    vm_instructions.push((
                        Instruction::CurrentClosure {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::OpenThisScope(_) => {
                    vm_instructions.push((Instruction::PushThis {}, inst.span));
                }
                ir::InstructionKind::SetThis(_, this) => {
                    vm_instructions.push((
                        Instruction::SetThis {
                            source: reg_alloc.instruction_registers[this],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::CloseThisScope(_) => {
                    vm_instructions.push((Instruction::PopThis {}, inst.span));
                }
                ir::InstructionKind::NewObject => {
                    vm_instructions.push((
                        Instruction::NewObject {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::NewArray => {
                    vm_instructions.push((
                        Instruction::NewArray {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::FixedArgument(index) => {
                    vm_instructions.push((
                        Instruction::ArgGet {
                            dest: reg_alloc.instruction_registers[inst_id],
                            index: index
                                .try_into()
                                .map_err(|_| ProtoGenError::StackIndexOutOfRange)?,
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::ArgumentCount => {
                    vm_instructions.push((
                        Instruction::ArgCount {
                            dest: reg_alloc.instruction_registers[inst_id],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::Argument(index) => {
                    vm_instructions.push((
                        Instruction::ArgGetAt {
                            dest: reg_alloc.instruction_registers[inst_id],
                            index: reg_alloc.instruction_registers[index],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::GetField { target, key } => {
                    vm_instructions.push((
                        Instruction::GetField {
                            dest: reg_alloc.instruction_registers[inst_id],
                            target: reg_alloc.instruction_registers[target],
                            key: reg_alloc.instruction_registers[key],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::SetField { target, key, value } => {
                    vm_instructions.push((
                        Instruction::SetField {
                            target: reg_alloc.instruction_registers[target],
                            key: reg_alloc.instruction_registers[key],
                            value: reg_alloc.instruction_registers[value],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::GetFieldConst { target, ref key } => {
                    vm_instructions.push((
                        Instruction::GetFieldConst {
                            dest: reg_alloc.instruction_registers[inst_id],
                            target: reg_alloc.instruction_registers[target],
                            key: get_const_index(key)?,
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::SetFieldConst {
                    target,
                    ref key,
                    value,
                } => {
                    vm_instructions.push((
                        Instruction::SetFieldConst {
                            target: reg_alloc.instruction_registers[target],
                            key: get_const_index(key)?,
                            value: reg_alloc.instruction_registers[value],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::GetIndex { target, index } => {
                    vm_instructions.push((
                        Instruction::GetIndex {
                            dest: reg_alloc.instruction_registers[inst_id],
                            target: reg_alloc.instruction_registers[target],
                            index: reg_alloc.instruction_registers[index],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::SetIndex {
                    target,
                    index,
                    value,
                } => {
                    vm_instructions.push((
                        Instruction::SetIndex {
                            target: reg_alloc.instruction_registers[target],
                            index: reg_alloc.instruction_registers[index],
                            value: reg_alloc.instruction_registers[value],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::GetIndexConst { target, ref index } => {
                    vm_instructions.push((
                        Instruction::GetIndexConst {
                            dest: reg_alloc.instruction_registers[inst_id],
                            target: reg_alloc.instruction_registers[target],
                            index: get_const_index(index)?,
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::SetIndexConst {
                    target,
                    ref index,
                    value,
                } => {
                    vm_instructions.push((
                        Instruction::SetIndexConst {
                            target: reg_alloc.instruction_registers[target],
                            index: get_const_index(index)?,
                            value: reg_alloc.instruction_registers[value],
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::Phi(shadow) => {
                    let shadow_reg = reg_alloc.shadow_registers[shadow];
                    let dest_reg = reg_alloc.instruction_registers[inst_id];
                    if shadow_reg != dest_reg {
                        vm_instructions.push((
                            Instruction::Copy {
                                dest: dest_reg,
                                source: shadow_reg,
                            },
                            inst.span,
                        ));
                    }
                }
                ir::InstructionKind::Upsilon(shadow, source) => {
                    if shadow_liveness.is_live_upsilon(shadow, block_id, inst_index) {
                        let shadow_reg = reg_alloc.shadow_registers[shadow];
                        let source_reg = reg_alloc.instruction_registers[source];
                        if shadow_reg != source_reg {
                            vm_instructions.push((
                                Instruction::Copy {
                                    dest: shadow_reg,
                                    source: source_reg,
                                },
                                inst.span,
                            ));
                        }
                    }
                }
                ir::InstructionKind::UnOp { op, source } => {
                    let output_reg = reg_alloc.instruction_registers[inst_id];
                    match op {
                        ir::UnOp::IsUndefined => {
                            vm_instructions.push((
                                Instruction::IsUndefined {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::IsDefined => {
                            vm_instructions.push((
                                Instruction::IsDefined {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::Test => {
                            vm_instructions.push((
                                Instruction::Test {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::Not => {
                            vm_instructions.push((
                                Instruction::Not {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::Negate => {
                            vm_instructions.push((
                                Instruction::Negate {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::BitNegate => {
                            vm_instructions.push((
                                Instruction::BitNegate {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::Increment => {
                            vm_instructions.push((
                                Instruction::Increment {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                        ir::UnOp::Decrement => {
                            vm_instructions.push((
                                Instruction::Decrement {
                                    dest: output_reg,
                                    arg: reg_alloc.instruction_registers[source],
                                },
                                inst.span,
                            ));
                        }
                    }
                }
                ir::InstructionKind::BinOp { left, op, right } => {
                    let dest = reg_alloc.instruction_registers[inst_id];
                    let left = reg_alloc.instruction_registers[left];
                    let right = reg_alloc.instruction_registers[right];
                    match op {
                        ir::BinOp::Add => {
                            vm_instructions
                                .push((Instruction::Add { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Sub => {
                            vm_instructions
                                .push((Instruction::Subtract { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Mult => {
                            vm_instructions
                                .push((Instruction::Multiply { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Div => {
                            vm_instructions
                                .push((Instruction::Divide { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Rem => {
                            vm_instructions
                                .push((Instruction::Remainder { dest, left, right }, inst.span));
                        }
                        ir::BinOp::IDiv => {
                            vm_instructions
                                .push((Instruction::IntDivide { dest, left, right }, inst.span));
                        }
                        ir::BinOp::LessThan => {
                            vm_instructions
                                .push((Instruction::IsLess { dest, left, right }, inst.span));
                        }
                        ir::BinOp::LessEqual => {
                            vm_instructions
                                .push((Instruction::IsLessEqual { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Equal => {
                            vm_instructions
                                .push((Instruction::IsEqual { dest, left, right }, inst.span));
                        }
                        ir::BinOp::NotEqual => {
                            vm_instructions
                                .push((Instruction::IsNotEqual { dest, left, right }, inst.span));
                        }
                        ir::BinOp::GreaterThan => {
                            vm_instructions.push((
                                Instruction::IsLess {
                                    dest,
                                    left: right,
                                    right: left,
                                },
                                inst.span,
                            ));
                        }
                        ir::BinOp::GreaterEqual => {
                            vm_instructions.push((
                                Instruction::IsLessEqual {
                                    dest,
                                    left: right,
                                    right: left,
                                },
                                inst.span,
                            ));
                        }
                        ir::BinOp::And => {
                            vm_instructions
                                .push((Instruction::And { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Or => {
                            vm_instructions
                                .push((Instruction::Or { dest, left, right }, inst.span));
                        }
                        ir::BinOp::Xor => {
                            vm_instructions
                                .push((Instruction::Xor { dest, left, right }, inst.span));
                        }
                        ir::BinOp::BitAnd => {
                            vm_instructions
                                .push((Instruction::BitAnd { dest, left, right }, inst.span));
                        }
                        ir::BinOp::BitOr => {
                            vm_instructions
                                .push((Instruction::BitOr { dest, left, right }, inst.span));
                        }
                        ir::BinOp::BitXor => {
                            vm_instructions
                                .push((Instruction::BitXor { dest, left, right }, inst.span));
                        }
                        ir::BinOp::BitShiftLeft => {
                            vm_instructions
                                .push((Instruction::BitShiftLeft { dest, left, right }, inst.span));
                        }
                        ir::BinOp::BitShiftRight => {
                            vm_instructions.push((
                                Instruction::BitShiftRight { dest, left, right },
                                inst.span,
                            ));
                        }
                        ir::BinOp::NullCoalesce => {
                            vm_instructions
                                .push((Instruction::NullCoalesce { dest, left, right }, inst.span));
                        }
                    }
                }
                ir::InstructionKind::OpenCallScope(_) => {
                    vm_instructions.push((Instruction::PushStackFrame {}, inst.span));
                }
                ir::InstructionKind::Call {
                    func,
                    stack_base,
                    this,
                    ..
                } => {
                    if stack_base != 0 {
                        vm_instructions.push((
                            Instruction::SplitStackFrame {
                                base: stack_base
                                    .try_into()
                                    .map_err(|_| ProtoGenError::StackIndexOutOfRange)?,
                            },
                            inst.span,
                        ));
                    }
                    vm_instructions.push((
                        Instruction::Call {
                            func: reg_alloc.instruction_registers[func],
                            this: this.map(|r| reg_alloc.instruction_registers[r]),
                        },
                        inst.span,
                    ));
                    if stack_base != 0 {
                        vm_instructions.push((Instruction::JoinStackFrame {}, inst.span));
                    }
                }
                ir::InstructionKind::GetStack(_, index) => {
                    vm_instructions.push((
                        Instruction::StackGet {
                            dest: reg_alloc.instruction_registers[inst_id],
                            index: index
                                .try_into()
                                .map_err(|_| ProtoGenError::StackIndexOutOfRange)?,
                        },
                        inst.span,
                    ));
                }
                ir::InstructionKind::CloseCallScope(_) => {
                    vm_instructions.push((Instruction::PopStackFrame {}, inst.span));
                }
            }
        }

        match block.exit.kind {
            ir::ExitKind::Exit => {
                vm_instructions.push((Instruction::PushStackFrame {}, block.exit.span));
                vm_instructions.push((Instruction::Return {}, block.exit.span));
            }
            ir::ExitKind::Return { stack_base, .. } => {
                vm_instructions.push((
                    Instruction::SplitStackFrame {
                        base: stack_base
                            .try_into()
                            .map_err(|_| ProtoGenError::StackIndexOutOfRange)?,
                    },
                    block.exit.span,
                ));

                vm_instructions.push((Instruction::Return {}, block.exit.span));
            }
            ir::ExitKind::Throw(value) => {
                vm_instructions.push((
                    Instruction::Throw {
                        source: reg_alloc.instruction_registers[value],
                    },
                    block.exit.span,
                ));
            }
            ir::ExitKind::Jump(block_id) => {
                // If we are the next block in output order, we don't need to add a jump
                if block_order_indexes[&block_id] != order_index + 1 {
                    block_vm_jumps.push((vm_instructions.len(), block_id));
                    vm_instructions
                        .push((Instruction::Jump { target: InstIdx(0) }, block.exit.span));
                }
            }
            ir::ExitKind::Branch {
                cond,
                if_true,
                if_false,
            } => {
                // Generate the minimal number of jump tests.
                //
                // If our `if_true` block is next, then jump to the `if_false` block when the
                // reversed condition is true.
                //
                // If the `if_false` block is next, then jump to the `if_true` block when the normal
                // condition is true.
                //
                // Otherwise, jump to the `if_true` block when the condition is true, then jump to
                // the `if_false` block unconditionally afterwards.
                let (cond, jump_test_branch, jump_after_branch) =
                    if block_order_indexes[&if_true] == order_index + 1 {
                        (cond.reverse(), if_false, None)
                    } else if block_order_indexes[&if_false] == order_index + 1 {
                        (cond, if_true, None)
                    } else {
                        (cond, if_true, Some(if_false))
                    };

                block_vm_jumps.push((vm_instructions.len(), jump_test_branch));
                vm_instructions.push((
                    match cond {
                        ir::BranchCondition::IsDefined(a) => Instruction::JumpIfUndefined {
                            target: InstIdx(0),
                            arg: reg_alloc.instruction_registers[a],
                            is_undefined: false,
                        },
                        ir::BranchCondition::IsUndefined(a) => Instruction::JumpIfUndefined {
                            target: InstIdx(0),
                            arg: reg_alloc.instruction_registers[a],
                            is_undefined: true,
                        },
                        ir::BranchCondition::IsTrue(a) => Instruction::JumpIf {
                            target: InstIdx(0),
                            arg: reg_alloc.instruction_registers[a],
                            is_true: true,
                        },
                        ir::BranchCondition::IsFalse(a) => Instruction::JumpIf {
                            target: InstIdx(0),
                            arg: reg_alloc.instruction_registers[a],
                            is_true: false,
                        },
                        ir::BranchCondition::Equal(a, b) => Instruction::JumpIfEqual {
                            target: InstIdx(0),
                            left: reg_alloc.instruction_registers[a],
                            right: reg_alloc.instruction_registers[b],
                        },
                        ir::BranchCondition::NotEqual(a, b) => Instruction::JumpIfNotEqual {
                            target: InstIdx(0),
                            left: reg_alloc.instruction_registers[a],
                            right: reg_alloc.instruction_registers[b],
                        },
                        ir::BranchCondition::LessThan(a, b) => Instruction::JumpIfLess {
                            target: InstIdx(0),
                            left: reg_alloc.instruction_registers[a],
                            right: reg_alloc.instruction_registers[b],
                        },
                        ir::BranchCondition::LessEqual(a, b) => Instruction::JumpIfLessEqual {
                            target: InstIdx(0),
                            left: reg_alloc.instruction_registers[a],
                            right: reg_alloc.instruction_registers[b],
                        },
                        ir::BranchCondition::GreaterThan(a, b) => Instruction::JumpIfLess {
                            target: InstIdx(0),
                            left: reg_alloc.instruction_registers[b],
                            right: reg_alloc.instruction_registers[a],
                        },
                        ir::BranchCondition::GreaterEqual(a, b) => Instruction::JumpIfLessEqual {
                            target: InstIdx(0),
                            left: reg_alloc.instruction_registers[b],
                            right: reg_alloc.instruction_registers[a],
                        },
                    },
                    block.exit.span,
                ));

                if let Some(jump_after) = jump_after_branch {
                    block_vm_jumps.push((vm_instructions.len(), jump_after));
                    vm_instructions
                        .push((Instruction::Jump { target: InstIdx(0) }, block.exit.span));
                }
            }
        }
    }

    for (index, block_id) in block_vm_jumps {
        let jump_offset = block_vm_starts[block_id]
            .try_into()
            .expect("instruction length overflow");
        match &mut vm_instructions[index].0 {
            Instruction::Jump { target } => {
                *target = jump_offset;
            }
            Instruction::JumpIf { target, .. } => {
                *target = jump_offset;
            }
            Instruction::JumpIfUndefined { target, .. } => {
                *target = jump_offset;
            }
            Instruction::JumpIfEqual { target, .. } => {
                *target = jump_offset;
            }
            Instruction::JumpIfNotEqual { target, .. } => {
                *target = jump_offset;
            }
            Instruction::JumpIfLess { target, .. } => {
                *target = jump_offset;
            }
            Instruction::JumpIfLessEqual { target, .. } => {
                *target = jump_offset;
            }
            _ => panic!("instruction not a jump"),
        }
    }

    let bytecode = vm::ByteCode::encode(vm_instructions.into_iter())?;

    Ok(Prototype {
        reference: ir.reference.clone(),
        bytecode,
        constants: constants.into_boxed_slice(),
        prototypes: prototypes.into_boxed_slice(),
        heap_vars: heap_alloc.heap_var_descriptors.into_boxed_slice(),
    })
}
