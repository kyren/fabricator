use std::collections::hash_map;

use fabricator_util::{index_containers::IndexSet, typed_id_map::SecondaryMap};
use fabricator_vm::Span;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    analysis::vec_change_set::VecChangeSet,
    constant::Constant,
    graph::{dfs::depth_first_search_with, dominators::Dominators},
    ir,
};

/// Convert uses of IR variables into SSA, possibly by inserting Phi and Upsilon instructions.
///
/// This will convert uses of variables in reachable blocks, blocks that are never executed will not
/// be modified. Also, any variables that are upvalues from the parent function or upvalues in any
/// child functions will not be changed into SSA form to maintain cross-function sharing.
///
/// All uses of variables without an assignment are converted into `Undefined`. This includes
/// `GetVariable` instructions used before a `SetVariable` to the same variable, and also any
/// required `Upsilon` instructions without a previous assignment.
///
/// The resulting phi placement will be *minimal* but not *pruned*.
pub fn convert_to_ssa<S>(ir: &mut ir::Function<S>) {
    // We don't do SSA conversion of any shared variables: static variables, upvalues, and any owned
    // variables shared to a lower function.
    let mut skip_vars = FxHashSet::default();
    for (var_id, var) in ir.variables.iter() {
        if !var.is_heap() {
            skip_vars.insert(var_id);
        }
    }
    for child_func in ir.functions.values() {
        for var in child_func.variables.values() {
            if let &ir::Variable::Upper(var_id) = var {
                skip_vars.insert(var_id);
            }
        }
    }

    let dominators = Dominators::compute(ir.start_block, |b| ir.blocks[b].exit.kind.successors());

    let mut assigning_blocks: SecondaryMap<ir::VarId, FxHashSet<ir::BlockId>> = SecondaryMap::new();

    // We don't do any SSA conversion of unreachable blocks.
    for block_id in dominators.topological_order() {
        let block = &ir.blocks[block_id];
        for &inst_id in &block.instructions {
            if let ir::InstructionKind::SetVariable(var_id, _) = ir.instructions[inst_id].kind {
                if !skip_vars.contains(&var_id) {
                    let blocks = assigning_blocks.get_or_insert_default(var_id);
                    blocks.insert(block_id);
                }
            }
        }
    }

    // Add phi functions by using dominance frontiers of blocks that write to variables (including
    // inserted phi functions).
    //
    // This algorithm is from Cytron et al. (1991)
    // https://bears.ece.ucsb.edu/class/ece253/papers/cytron91.pdf

    let mut phi_functions: SecondaryMap<ir::BlockId, FxHashMap<ir::VarId, ir::ShadowVar>> =
        SecondaryMap::new();
    let mut shadow_map: SecondaryMap<ir::ShadowVar, ir::VarId> = SecondaryMap::new();

    let mut work_queue = Vec::new();
    let mut work_added = IndexSet::new();
    for (var_id, assigning_blocks) in assigning_blocks.iter() {
        assert!(work_queue.is_empty());
        work_added.clear();

        for &block_id in assigning_blocks {
            work_queue.push(block_id);
            work_added.insert(block_id.index() as usize);
        }

        while let Some(assigning_block_id) = work_queue.pop() {
            for frontier_block_id in dominators.dominance_frontier(assigning_block_id).unwrap() {
                if let hash_map::Entry::Vacant(vacant) = phi_functions
                    .get_or_insert_default(frontier_block_id)
                    .entry(var_id)
                {
                    let shadow_var = ir.shadow_vars.insert(());
                    vacant.insert(shadow_var);
                    shadow_map.insert(shadow_var, var_id);
                    if work_added.insert(frontier_block_id.index() as usize) {
                        work_queue.push(frontier_block_id);
                    }
                }
            }
        }
    }

    // Actually insert `Phi` instructions for every variable at the beginning of every block that we
    // have determined needs them.
    let mut inst_change_set = VecChangeSet::new();
    for block_id in dominators.topological_order() {
        let block = &mut ir.blocks[block_id];
        if let Some(phi_functions) = phi_functions.get(block_id) {
            for &shadow_var in phi_functions.values() {
                inst_change_set.insert(
                    0,
                    ir.instructions.insert(ir::Instruction {
                        kind: ir::InstructionKind::Phi(shadow_var),
                        span: Span::null(),
                    }),
                );
            }
            inst_change_set.apply(&mut block.instructions);
        }
    }

    // Rename all variables, converting into SSA form.
    //
    // This algorithm is also from Cytron et al. (1991)
    // https://bears.ece.ucsb.edu/class/ece253/papers/cytron91.pdf

    let current_vars: SecondaryMap<ir::VarId, Vec<ir::InstId>> =
        SecondaryMap::from_iter(assigning_blocks.ids().map(|var_id| (var_id, Vec::new())));
    let var_stack_bottom: SecondaryMap<ir::BlockId, FxHashMap<ir::VarId, usize>> =
        SecondaryMap::new();

    // We turn the recursive algorithm from Cytron et al. into an explicit DFS of the dominator
    // tree.
    let start_block = ir.start_block;
    depth_first_search_with(
        &mut (current_vars, var_stack_bottom),
        start_block,
        |(current_vars, var_stack_bottom), block_id| {
            let var_stack_bottom = var_stack_bottom.get_or_insert_default(block_id);
            for (var_id, stack) in current_vars.iter() {
                var_stack_bottom.insert(var_id, stack.len());
            }

            let block = &mut ir.blocks[block_id];
            for &inst_id in &block.instructions {
                let inst = &mut ir.instructions[inst_id];
                match inst.kind {
                    ir::InstructionKind::GetVariable(var_id) => {
                        if let Some(top) = current_vars.get(var_id).and_then(|s| s.last().copied())
                        {
                            inst.kind = ir::InstructionKind::Copy(top);
                        } else if !skip_vars.contains(&var_id) {
                            // If the current variable has had no assignments, then we replace it
                            // with `Undefined`.
                            inst.kind = ir::InstructionKind::Constant(Constant::Undefined);
                        }
                    }
                    ir::InstructionKind::SetVariable(var_id, source) => {
                        if let Some(stack) = current_vars.get_mut(var_id) {
                            inst.kind = ir::InstructionKind::NoOp;
                            stack.push(source);
                        } else {
                            assert!(skip_vars.contains(&var_id));
                        }
                    }
                    ir::InstructionKind::Phi(shadow_var) => {
                        // If there is a `Phi` function we did not insert, we ignore it.
                        if let Some(&var_id) = shadow_map.get(shadow_var) {
                            current_vars.get_mut(var_id).unwrap().push(inst_id);
                        }
                    }
                    ir::InstructionKind::OpenVariable(var_id) => {
                        if !skip_vars.contains(&var_id) {
                            inst.kind = ir::InstructionKind::NoOp;
                        }
                    }
                    ir::InstructionKind::CloseVariable(var_id) => {
                        if !skip_vars.contains(&var_id) {
                            inst.kind = ir::InstructionKind::NoOp;
                        }
                    }
                    _ => {}
                }
            }

            // Loop through every successor block, for every phi function that was inserted into
            // that block, we must insert a matching upsilon.
            for succ in block.exit.kind.successors() {
                if let Some(phi_functions) = phi_functions.get(succ) {
                    for (&var_id, &shadow_var) in phi_functions {
                        let var_inst;
                        if let Some(top) = current_vars.get(var_id).and_then(|s| s.last().copied())
                        {
                            var_inst = top;
                        } else {
                            // If we don't have a value to add to an `Upsilon`, then we have to
                            // make one to keep the IR well-formed. This was use of a value that was
                            // undefined on this code path, so we set the value to undefined.
                            var_inst = ir.instructions.insert(ir::Instruction {
                                kind: ir::InstructionKind::Constant(Constant::Undefined),
                                span: Span::null(),
                            });
                            block.instructions.push(var_inst);
                        }

                        block
                            .instructions
                            .push(ir.instructions.insert(ir::Instruction {
                                kind: ir::InstructionKind::Upsilon(shadow_var, var_inst),
                                span: ir.instructions[var_inst].span,
                            }));
                    }
                }
            }

            dominators.dominance_children(block_id).unwrap()
        },
        |(current_vars, var_stack_bottom), block_id| {
            for (&var_id, &stack_bottom) in &var_stack_bottom[block_id] {
                current_vars.get_mut(var_id).unwrap().truncate(stack_bottom);
            }
        },
    );
}
