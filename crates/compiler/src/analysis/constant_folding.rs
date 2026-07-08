use crate::{constant::Constant, graph::dfs::topological_order, ir};

pub fn fold_constants<S: Eq + Clone>(ir: &mut ir::Function<S>) {
    let reachable_blocks =
        topological_order(ir.start_block, |b| ir.blocks[b].exit.kind.successors());

    // Since every instruction is in SSA form and in well-formed IR every use must be dominated by a
    // definition, iterating in topological order should fold everything possible in one pass.
    for &block_id in &reachable_blocks {
        let block = &mut ir.blocks[block_id];

        for &inst_id in &block.instructions {
            let get_constant = |inst_id| {
                if let ir::InstructionKind::Constant(c) = &ir.instructions[inst_id].kind {
                    Some(c)
                } else {
                    None
                }
            };

            let mut new_inst = None;
            match ir.instructions[inst_id].kind.clone() {
                ir::InstructionKind::Copy(source) => {
                    new_inst =
                        get_constant(source).map(|c| ir::InstructionKind::Constant(c.clone()));
                }
                ir::InstructionKind::UnOp { op, source } => {
                    if let Some(c) = get_constant(source) {
                        new_inst = match op {
                            ir::UnOp::IsDefined => Some(Constant::Boolean(!c.is_undefined())),
                            ir::UnOp::IsUndefined => Some(Constant::Boolean(c.is_undefined())),
                            ir::UnOp::Test => Some(Constant::Boolean(!c.cast_bool())),
                            ir::UnOp::Not => Some(Constant::Boolean(!c.cast_bool())),
                            ir::UnOp::Negate => c.negate(),
                            ir::UnOp::BitNegate => c.bit_negate().map(Constant::Integer),
                            ir::UnOp::Increment => c.add(&Constant::Integer(1)),
                            ir::UnOp::Decrement => c.sub(&Constant::Integer(1)),
                        }
                        .map(ir::InstructionKind::Constant);
                    }
                }
                ir::InstructionKind::BinOp { left, op, right } => {
                    let left_const = get_constant(left);
                    let right_const = get_constant(right);
                    if let (Some(l), Some(r)) = (left_const, right_const) {
                        new_inst = match op {
                            ir::BinOp::Add => l.add(r),
                            ir::BinOp::Sub => l.sub(r),
                            ir::BinOp::Mult => l.mult(r),
                            ir::BinOp::Div => l.div(r),
                            ir::BinOp::Rem => l.rem(r),
                            ir::BinOp::IDiv => l.idiv(r).map(Constant::Integer),
                            ir::BinOp::Equal => Some(Constant::Boolean(l.equal(r))),
                            ir::BinOp::NotEqual => Some(Constant::Boolean(!l.equal(r))),
                            ir::BinOp::LessThan => l.less_than(r).map(Constant::Boolean),
                            ir::BinOp::LessEqual => l.less_equal(r).map(Constant::Boolean),
                            ir::BinOp::GreaterThan => r.less_than(l).map(Constant::Boolean),
                            ir::BinOp::GreaterEqual => r.less_equal(l).map(Constant::Boolean),
                            ir::BinOp::And => Some(Constant::Boolean(l.and(r))),
                            ir::BinOp::Or => Some(Constant::Boolean(l.or(r))),
                            ir::BinOp::Xor => Some(Constant::Boolean(l.xor(r))),
                            ir::BinOp::BitAnd => l.bit_and(r).map(Constant::Integer),
                            ir::BinOp::BitOr => l.bit_or(r).map(Constant::Integer),
                            ir::BinOp::BitXor => l.bit_xor(r).map(Constant::Integer),
                            ir::BinOp::BitShiftLeft => l.bit_shift_left(r).map(Constant::Integer),
                            ir::BinOp::BitShiftRight => l.bit_shift_right(r).map(Constant::Integer),
                            ir::BinOp::NullCoalesce => Some(l.null_coalesce(r).clone()),
                        }
                        .map(ir::InstructionKind::Constant);
                    } else if let Some((un_op, source)) = match op {
                        ir::BinOp::Add => {
                            if right_const.and_then(Constant::as_integer) == Some(1) {
                                Some((ir::UnOp::Increment, left))
                            } else if right_const.and_then(Constant::as_integer) == Some(-1) {
                                Some((ir::UnOp::Decrement, left))
                            } else {
                                None
                            }
                        }
                        ir::BinOp::Sub => {
                            if right_const.and_then(Constant::as_integer) == Some(1) {
                                Some((ir::UnOp::Decrement, left))
                            } else if right_const.and_then(Constant::as_integer) == Some(-1) {
                                Some((ir::UnOp::Increment, left))
                            } else {
                                None
                            }
                        }
                        ir::BinOp::Equal => match (left_const, right_const) {
                            (_, Some(Constant::Undefined)) => Some((ir::UnOp::IsUndefined, left)),
                            (Some(Constant::Undefined), _) => Some((ir::UnOp::IsUndefined, right)),
                            _ => None,
                        },
                        ir::BinOp::NotEqual => match (left_const, right_const) {
                            (_, Some(Constant::Undefined)) => Some((ir::UnOp::IsDefined, left)),
                            (Some(Constant::Undefined), _) => Some((ir::UnOp::IsDefined, right)),
                            _ => None,
                        },
                        _ => None,
                    } {
                        new_inst = Some(ir::InstructionKind::UnOp { op: un_op, source });
                    } else if op == ir::BinOp::NullCoalesce {
                        if left_const.is_some_and(Constant::is_undefined) {
                            new_inst = Some(ir::InstructionKind::Copy(right));
                        }
                    }
                }
                ir::InstructionKind::GetField { target, key } => {
                    if let Some(key) = get_constant(key) {
                        new_inst = Some(ir::InstructionKind::GetFieldConst {
                            target,
                            key: key.clone(),
                        })
                    }
                }
                ir::InstructionKind::SetField { target, key, value } => {
                    if let Some(key) = get_constant(key) {
                        new_inst = Some(ir::InstructionKind::SetFieldConst {
                            target,
                            key: key.clone(),
                            value,
                        })
                    }
                }
                ir::InstructionKind::GetIndex { target, index } => {
                    if let Some(index) = get_constant(index) {
                        new_inst = Some(ir::InstructionKind::GetIndexConst {
                            target,
                            index: index.clone(),
                        })
                    }
                }
                ir::InstructionKind::SetIndex {
                    target,
                    index,
                    value,
                } => {
                    if let Some(index) = get_constant(index) {
                        new_inst = Some(ir::InstructionKind::SetIndexConst {
                            target,
                            index: index.clone(),
                            value,
                        })
                    }
                }
                _ => {}
            }

            if let Some(new_inst) = new_inst {
                ir.instructions[inst_id].kind = new_inst;
            }
        }

        if let ir::ExitKind::Branch {
            cond,
            if_false,
            if_true,
        } = block.exit.kind
        {
            let get_constant = |inst_id| {
                if let ir::InstructionKind::Constant(c) = &ir.instructions[inst_id].kind {
                    Some(c)
                } else {
                    None
                }
            };

            let const_cond = match cond {
                ir::BranchCondition::IsDefined(a) => get_constant(a).map(|c| !c.is_undefined()),
                ir::BranchCondition::IsUndefined(a) => get_constant(a).map(|c| c.is_undefined()),
                ir::BranchCondition::IsTrue(a) => get_constant(a).map(|c| c.cast_bool()),
                ir::BranchCondition::IsFalse(a) => get_constant(a).map(|c| !c.cast_bool()),
                ir::BranchCondition::Equal(a, b) => get_constant(a)
                    .and_then(|a| Some((a, get_constant(b)?)))
                    .map(|(a, b)| a.equal(b)),
                ir::BranchCondition::NotEqual(a, b) => get_constant(a)
                    .and_then(|a| Some((a, get_constant(b)?)))
                    .map(|(a, b)| !a.equal(b)),
                ir::BranchCondition::LessThan(a, b) => get_constant(a)
                    .and_then(|a| Some((a, get_constant(b)?)))
                    .and_then(|(a, b)| a.less_than(b)),
                ir::BranchCondition::LessEqual(a, b) => get_constant(a)
                    .and_then(|a| Some((a, get_constant(b)?)))
                    .and_then(|(a, b)| a.less_equal(b)),
                ir::BranchCondition::GreaterThan(a, b) => get_constant(a)
                    .and_then(|a| Some((a, get_constant(b)?)))
                    .and_then(|(a, b)| b.less_than(a)),
                ir::BranchCondition::GreaterEqual(a, b) => get_constant(a)
                    .and_then(|a| Some((a, get_constant(b)?)))
                    .and_then(|(a, b)| b.less_equal(a)),
            };

            if let Some(cond_is_true) = const_cond {
                let target = if cond_is_true { if_true } else { if_false };
                block.exit.kind = ir::ExitKind::Jump(target);
            }
        }
    }
}
