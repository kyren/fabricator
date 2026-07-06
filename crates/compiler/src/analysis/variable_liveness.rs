use fabricator_util::typed_id_map::SecondaryMap;
use rustc_hash::FxHashSet;
use thiserror::Error;

use crate::{
    analysis::scope_liveness::{ScopeBlockLiveness, ScopeLiveness, ScopeLivenessError},
    graph::dominators::Dominators,
    ir,
};

#[derive(Debug, Copy, Clone, Error)]
pub enum VariableVerificationErrorKind {
    #[error("is non-owned and has an open or close instruction at {0}")]
    OpenCloseUnOwned(ir::InstLocation),
    #[error("is not opened exactly once")]
    BadOpen,
    #[error("has close at {0} is not dominated by its open")]
    CloseNotDominated(ir::InstLocation),
    #[error("has close instruction at {0} which does not close an open variable")]
    DeadClose(ir::InstLocation),
    #[error("has incoming edges for block {0} that are not all open or all closed")]
    IndeterminateState(ir::BlockId),
    #[error("has a use at {0} which is not dominated by its open or may occur after a close")]
    UseNotInRange(ir::InstLocation),
}

#[derive(Debug, Copy, Clone, Error)]
#[error("variable {var_id} {kind}")]
pub struct VariableVerificationError {
    pub var_id: ir::VarId,
    pub kind: VariableVerificationErrorKind,
}

#[derive(Debug)]
pub struct VariableLiveness {
    live_ranges: SecondaryMap<ir::VarId, ScopeLiveness>,
    live_variables_for_block: SecondaryMap<ir::BlockId, FxHashSet<ir::VarId>>,
}

impl VariableLiveness {
    /// Compute owned variable liveness ranges for the scoe of every reachable block in the given
    /// IR. Additionally verifies that all uses are within the live range and only owned variables
    /// are opened or closed.
    pub fn compute<S>(ir: &ir::Function<S>) -> Result<Self, VariableVerificationError> {
        let dominators =
            Dominators::compute(ir.start_block, |b| ir.blocks[b].exit.kind.successors());

        let mut variables: SecondaryMap<ir::VarId, ()> = SecondaryMap::new();
        let mut variable_open: SecondaryMap<ir::VarId, ir::InstLocation> = SecondaryMap::new();
        let mut variable_uses: SecondaryMap<ir::VarId, Vec<ir::InstLocation>> = SecondaryMap::new();
        let mut variable_closes: SecondaryMap<ir::VarId, Vec<ir::InstLocation>> =
            SecondaryMap::new();

        for block_id in dominators.topological_order() {
            let block = &ir.blocks[block_id];
            for (inst_index, &inst_id) in block.instructions.iter().enumerate() {
                let inst_loc = ir::InstLocation::new(block_id, inst_index);

                match ir.instructions[inst_id].kind {
                    ir::InstructionKind::OpenVariable(var_id) => {
                        variables.insert(var_id, ());
                        if variable_open.insert(var_id, inst_loc).is_some() {
                            return Err(VariableVerificationError {
                                kind: VariableVerificationErrorKind::BadOpen,
                                var_id,
                            });
                        }
                    }
                    ir::InstructionKind::GetVariable(var_id)
                    | ir::InstructionKind::SetVariable(var_id, _) => {
                        variables.insert(var_id, ());
                        variable_uses.get_or_insert_default(var_id).push(inst_loc);
                    }
                    ir::InstructionKind::Closure { func, .. } => {
                        for var in ir.functions[func].variables.values() {
                            // Creating a closure uses every upper variable that the closure closes
                            // over.
                            if let &ir::Variable::Upper(var_id) = var {
                                variables.insert(var_id, ());
                                variable_uses.get_or_insert_default(var_id).push(inst_loc);
                            }
                        }
                    }
                    ir::InstructionKind::CloseVariable(var_id) => {
                        variables.insert(var_id, ());
                        variable_closes.get_or_insert_default(var_id).push(inst_loc);
                    }
                    _ => {}
                }
            }
        }

        let mut this = VariableLiveness {
            live_ranges: SecondaryMap::new(),
            live_variables_for_block: SecondaryMap::new(),
        };

        for var_id in variables.ids() {
            match ir.variables[var_id] {
                ir::Variable::Heap => {}
                ir::Variable::Static(_) | ir::Variable::Upper(_) => {
                    if let Some(&inst_loc) = variable_open
                        .get(var_id)
                        .or_else(|| variable_closes.get(var_id)?.first())
                    {
                        return Err(VariableVerificationError {
                            kind: VariableVerificationErrorKind::OpenCloseUnOwned(inst_loc),
                            var_id,
                        });
                    }

                    // We do not need to compute liveness ranges for non-owned variables, they are
                    // alive for the entire function.
                    continue;
                }
            }

            let &variable_open = variable_open.get(var_id).ok_or(VariableVerificationError {
                kind: VariableVerificationErrorKind::BadOpen,
                var_id,
            })?;

            let variable_liveness = ScopeLiveness::compute(
                ir,
                &dominators,
                variable_open,
                variable_closes.get(var_id).into_iter().flatten().copied(),
            )
            .map_err(|e| VariableVerificationError {
                kind: match e {
                    ScopeLivenessError::CloseNotDominated(inst_loc) => {
                        VariableVerificationErrorKind::CloseNotDominated(inst_loc)
                    }
                    ScopeLivenessError::IndeterminateState(block_id) => {
                        VariableVerificationErrorKind::IndeterminateState(block_id)
                    }
                    ScopeLivenessError::DeadClose(inst_loc) => {
                        VariableVerificationErrorKind::DeadClose(inst_loc)
                    }
                },
                var_id,
            })?;

            for &inst_loc in variable_uses.get(var_id).into_iter().flatten() {
                let live_range = variable_liveness.for_block(inst_loc.block_id).ok_or({
                    VariableVerificationError {
                        kind: VariableVerificationErrorKind::UseNotInRange(inst_loc),
                        var_id,
                    }
                })?;

                if live_range
                    .start
                    .is_some_and(|start| inst_loc.index <= start)
                {
                    return Err(VariableVerificationError {
                        kind: VariableVerificationErrorKind::UseNotInRange(inst_loc),
                        var_id,
                    });
                }
                if live_range.end.is_some_and(|end| inst_loc.index >= end) {
                    return Err(VariableVerificationError {
                        kind: VariableVerificationErrorKind::UseNotInRange(inst_loc),
                        var_id,
                    });
                }
            }

            for (block_id, _) in variable_liveness.live_blocks() {
                this.live_variables_for_block
                    .get_or_insert_default(block_id)
                    .insert(var_id);
            }
            this.live_ranges.insert(var_id, variable_liveness);
        }

        Ok(this)
    }

    /// Returns all owned variables that are live anywhere within the given block.
    pub fn live_for_block(
        &self,
        block_id: ir::BlockId,
    ) -> impl Iterator<Item = (ir::VarId, ScopeBlockLiveness)> + '_ {
        self.live_variables_for_block
            .get(block_id)
            .into_iter()
            .flatten()
            .map(move |&var_id| {
                (
                    var_id,
                    self.live_ranges[var_id].for_block(block_id).unwrap(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use fabricator_vm::{FunctionRef, Span};

    use crate::constant::Constant;

    use super::*;

    #[test]
    fn test_variable_liveness_loop_closes() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut variables = ir::VariableMap::new();

        let var = variables.insert(ir::Variable::Heap);

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenVariable(var),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseVariable(var),
                span: Span::null(),
            }));

        block_b.exit.kind = ir::ExitKind::Jump(block_b_id);

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            num_parameters: 0,
            instructions,
            blocks,
            variables,
            shadow_vars: Default::default(),
            this_scopes: Default::default(),
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        assert!(matches!(
            VariableLiveness::compute(&ir),
            Err(VariableVerificationError {
                kind: VariableVerificationErrorKind::IndeterminateState(..),
                ..
            })
        ));
    }

    #[test]
    fn test_variable_liveness_loop_reopens() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut variables = ir::VariableMap::new();

        let var = variables.insert(ir::Variable::Heap);

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        let true_ = instructions.insert(ir::Instruction {
            kind: ir::InstructionKind::Constant(Constant::Boolean(true)),
            span: Span::null(),
        });
        block_a.instructions.push(true_);
        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenVariable(var),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Branch {
            cond: ir::BranchCondition::IsTrue(true_),
            if_false: block_b_id,
            if_true: block_a_id,
        };

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseVariable(var),
                span: Span::null(),
            }));

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            num_parameters: 0,
            instructions,
            blocks,
            variables,
            shadow_vars: Default::default(),
            this_scopes: Default::default(),
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        assert!(matches!(
            VariableLiveness::compute(&ir),
            Err(VariableVerificationError {
                kind: VariableVerificationErrorKind::IndeterminateState(..),
                ..
            })
        ));
    }
}
