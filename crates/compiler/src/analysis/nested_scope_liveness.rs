use std::hash::Hash;

use fabricator_util::typed_id_map::{self, SecondaryMap};
use rustc_hash::FxHashSet;
use thiserror::Error;

use crate::{
    analysis::scope_liveness::{ScopeBlockLiveness, ScopeLiveness, ScopeLivenessError},
    graph::{dfs::try_depth_first_search_with, dominators::Dominators},
    ir,
};

#[derive(Debug, Copy, Clone, Error)]
pub enum NestedScopeVerificationErrorKind<I> {
    #[error("is not opened exactly once")]
    BadOpen,
    #[error("has close at {0} is not dominated by its open")]
    CloseNotDominated(ir::InstLocation),
    #[error("has close instruction at {0} does not close an open variable")]
    DeadClose(ir::InstLocation),
    #[error("has incoming edges for block {0} that are not all open or all closed")]
    IndeterminateState(ir::BlockId),
    #[error("has use at {0} is not dominated by its open or may occur after a close")]
    UseNotInRange(ir::InstLocation),
    #[error("is not strictly nested within scope {other_scope}")]
    ScopeNotNested { other_scope: I },
    #[error("use is within an inner scope {inner_scope} at location {instruction}")]
    UseOverlapsInner {
        inner_scope: I,
        instruction: ir::InstLocation,
    },
}

#[derive(Debug, Copy, Clone, Error)]
#[error("scope {scope} {kind}")]
pub struct NestedScopeVerificationError<I> {
    pub scope: I,
    pub kind: NestedScopeVerificationErrorKind<I>,
}

#[derive(Debug)]
pub struct NestedScopeLiveness<I>
where
    I: typed_id_map::Id,
{
    scope_meta: SecondaryMap<I, ScopeMeta<I>>,
    live_scopes_for_block: SecondaryMap<ir::BlockId, FxHashSet<I>>,
    nesting: usize,
}

#[derive(Debug)]
struct ScopeMeta<I> {
    liveness: ScopeLiveness,
    inner_scopes: Vec<I>,
    nesting_level: usize,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ScopeOperation<I> {
    Open(I),
    Use(I),
    Close(I),
}

impl<I> NestedScopeLiveness<I>
where
    I: typed_id_map::Id + Eq + Hash + Copy,
{
    /// Compute scope liveness ranges and nesting level for every scope in the given IR.
    ///
    /// A scope is live after it is opened and dead when it is closed.
    ///
    /// This also verifies all scopes within the IR and their use, namely that:
    ///   1) Every scope has exactly one open instruction.
    ///   2) Every instruction in the CFG has a statically defined opened or closed state for every
    ///      scope, there are no regions which depend on the path through the CFG.
    ///   3) Every use of a scope is in its definitely-open region.
    ///   4) Scopes may be nested, but they must be strictly so.
    ///   5) Every use of a scope is not within an inner nested scope.
    ///   6) Every scope close instruction closes an open scope.
    fn compute_with<S>(
        ir: &ir::Function<S>,
        scope_inst_op: impl Fn(&ir::Instruction<S>) -> Option<ScopeOperation<I>>,
        scope_exit_op: impl Fn(&ir::Exit) -> Option<ScopeOperation<I>>,
    ) -> Result<Self, NestedScopeVerificationError<I>> {
        let dominators =
            Dominators::compute(ir.start_block, |b| ir.blocks[b].exit.kind.successors());

        let mut scopes: SecondaryMap<I, ()> = SecondaryMap::new();
        let mut scope_open: SecondaryMap<I, ir::InstLocation> = SecondaryMap::new();
        let mut scope_uses: SecondaryMap<I, Vec<ir::InstLocation>> = SecondaryMap::new();
        let mut scope_closes: SecondaryMap<I, Vec<ir::InstLocation>> = SecondaryMap::new();

        // In-order list of every scope operation for each basic block.
        let mut scope_block_ops: SecondaryMap<ir::BlockId, Vec<(usize, ScopeOperation<I>)>> =
            SecondaryMap::new();

        for block_id in dominators.topological_order() {
            let mut insert_scope_op = |inst_loc, scope_op| {
                match scope_op {
                    ScopeOperation::Open(scope) => {
                        scopes.insert(scope, ());
                        if scope_open.insert(scope, inst_loc).is_some() {
                            return Err(NestedScopeVerificationError {
                                kind: NestedScopeVerificationErrorKind::BadOpen,
                                scope,
                            });
                        }
                    }
                    ScopeOperation::Use(scope) => {
                        scopes.insert(scope, ());
                        scope_uses.get_or_insert_default(scope).push(inst_loc);
                    }
                    ScopeOperation::Close(scope) => {
                        scopes.insert(scope, ());
                        scope_closes.get_or_insert_default(scope).push(inst_loc);
                    }
                }

                scope_block_ops
                    .get_or_insert_default(inst_loc.block_id)
                    .push((inst_loc.index, scope_op));
                Ok(())
            };

            let block = &ir.blocks[block_id];
            for (inst_index, &inst_id) in block.instructions.iter().enumerate() {
                if let Some(op) = scope_inst_op(&ir.instructions[inst_id]) {
                    insert_scope_op(ir::InstLocation::new(block_id, inst_index), op)?;
                }
            }
            if let Some(op) = scope_exit_op(&block.exit) {
                insert_scope_op(
                    ir::InstLocation::new(block_id, block.instructions.len()),
                    op,
                )?;
            }
        }

        let mut this = NestedScopeLiveness {
            scope_meta: SecondaryMap::new(),
            live_scopes_for_block: SecondaryMap::new(),
            nesting: 0,
        };

        for scope in scopes.ids() {
            let &scope_open = scope_open.get(scope).ok_or(NestedScopeVerificationError {
                kind: NestedScopeVerificationErrorKind::BadOpen,
                scope,
            })?;

            let scope_liveness = ScopeLiveness::compute(
                ir,
                &dominators,
                scope_open,
                scope_closes.get(scope).into_iter().flatten().copied(),
            )
            .map_err(|e| NestedScopeVerificationError {
                kind: match e {
                    ScopeLivenessError::CloseNotDominated(inst_loc) => {
                        NestedScopeVerificationErrorKind::CloseNotDominated(inst_loc)
                    }
                    ScopeLivenessError::IndeterminateState(block_id) => {
                        NestedScopeVerificationErrorKind::IndeterminateState(block_id)
                    }
                    ScopeLivenessError::DeadClose(inst_loc) => {
                        NestedScopeVerificationErrorKind::DeadClose(inst_loc)
                    }
                },
                scope,
            })?;

            for &inst_loc in scope_uses.get(scope).into_iter().flatten() {
                let live_range = scope_liveness.for_block(inst_loc.block_id).ok_or(
                    NestedScopeVerificationError {
                        kind: NestedScopeVerificationErrorKind::UseNotInRange(inst_loc),
                        scope,
                    },
                )?;

                if live_range
                    .start
                    .is_some_and(|start| inst_loc.index <= start)
                {
                    return Err(NestedScopeVerificationError {
                        kind: NestedScopeVerificationErrorKind::UseNotInRange(inst_loc),
                        scope,
                    });
                }
                if live_range.end.is_some_and(|end| inst_loc.index >= end) {
                    return Err(NestedScopeVerificationError {
                        kind: NestedScopeVerificationErrorKind::UseNotInRange(inst_loc),
                        scope,
                    });
                }
            }

            for (block_id, _) in scope_liveness.live_blocks() {
                this.live_scopes_for_block
                    .get_or_insert_default(block_id)
                    .insert(scope);
            }
            this.scope_meta.insert(
                scope,
                ScopeMeta {
                    liveness: scope_liveness,
                    inner_scopes: Vec::new(),
                    nesting_level: 0,
                },
            );
        }

        // Check that scopes are strictly nested and assign a "nesting level" to each scope.
        //
        // We do a DFS on the graph and keep track of the current top-level scope. If we encounter a
        // close that is not the current top scope, we know that scopes are not strictly nested.
        //
        // As we do this, make sure that every scope use is of the current top-level scope. If it is
        // not, we know we have an improperly nested use.

        let mut scope_stack = Vec::<I>::new();
        try_depth_first_search_with(
            &mut scope_stack,
            ir.start_block,
            |scope_stack, block_id| {
                for &(inst_index, scope_op) in scope_block_ops.get(block_id).into_iter().flatten() {
                    match scope_op {
                        ScopeOperation::Use(scope) => {
                            let top_scope = scope_stack.last().copied().unwrap();
                            if scope != top_scope {
                                return Err(NestedScopeVerificationError {
                                    kind: NestedScopeVerificationErrorKind::UseOverlapsInner {
                                        inner_scope: top_scope,
                                        instruction: ir::InstLocation::new(block_id, inst_index),
                                    },
                                    scope,
                                });
                            }
                        }
                        ScopeOperation::Open(_) | ScopeOperation::Close(_) => {}
                    }

                    match scope_op {
                        ScopeOperation::Open(scope) => {
                            this.scope_meta.get_mut(scope).unwrap().nesting_level =
                                scope_stack.len();
                            if let Some(&upper) = scope_stack.last() {
                                this.scope_meta
                                    .get_mut(upper)
                                    .unwrap()
                                    .inner_scopes
                                    .push(scope);
                            }
                            scope_stack.push(scope);
                        }
                        ScopeOperation::Close(scope) => {
                            let top_scope = scope_stack.pop().unwrap();
                            if scope != top_scope {
                                return Err(NestedScopeVerificationError {
                                    kind: NestedScopeVerificationErrorKind::ScopeNotNested {
                                        other_scope: scope,
                                    },
                                    scope: top_scope,
                                });
                            }
                        }
                        ScopeOperation::Use(_) => {}
                    }
                }

                Ok(ir.blocks[block_id].exit.kind.successors())
            },
            |scope_stack, block_id| {
                for &(_, scope_op) in scope_block_ops.get(block_id).into_iter().flatten().rev() {
                    match scope_op {
                        ScopeOperation::Close(scope) => {
                            scope_stack.push(scope);
                        }
                        ScopeOperation::Open(scope) => {
                            assert!(scope_stack.pop() == Some(scope));
                        }
                        ScopeOperation::Use(_) => {}
                    }
                }

                Ok(())
            },
        )?;

        this.nesting = this
            .scope_meta
            .values()
            .map(|m| m.nesting_level + 1)
            .max()
            .unwrap_or(0);

        Ok(this)
    }

    pub fn scopes(&self) -> impl Iterator<Item = I> {
        self.scope_meta.ids()
    }

    /// Returns all owned scopes that are live anywhere within the given block.
    pub fn live_for_block(
        &self,
        block_id: ir::BlockId,
    ) -> impl Iterator<Item = (I, ScopeBlockLiveness)> + '_ {
        self.live_scopes_for_block
            .get(block_id)
            .into_iter()
            .flatten()
            .map(move |&scope| {
                (
                    scope,
                    self.scope_meta[scope].liveness.for_block(block_id).unwrap(),
                )
            })
    }

    /// Return all scopes which lie *immediately* inside the given scope.
    pub fn inner_scopes(&self, scope: I) -> impl Iterator<Item = I> {
        self.scope_meta
            .get(scope)
            .map(|m| m.inner_scopes.iter().copied())
            .into_iter()
            .flatten()
    }

    pub fn has_inner_scope(&self, scope: I) -> bool {
        self.scope_meta
            .get(scope)
            .map(|m| !m.inner_scopes.is_empty())
            .unwrap_or(false)
    }

    /// Return the scope with the deepest nesting level which encloses the given instruction.
    ///
    /// If the given instruction is itself a scope open or close, then this will return the *outer*
    /// scope for that instruction.
    pub fn deepest_for(&self, inst_loc: ir::InstLocation) -> Option<I> {
        let mut deepest = None;
        for (scope, liveness) in self.live_for_block(inst_loc.block_id) {
            let within_bounds = liveness.start.is_none_or(|start| inst_loc.index > start)
                && liveness.end.is_none_or(|end| inst_loc.index < end);

            if within_bounds
                && deepest.is_none_or(|prev_scope| {
                    self.scope_meta[scope].nesting_level > self.scope_meta[prev_scope].nesting_level
                })
            {
                deepest = Some(scope);
            }
        }
        deepest
    }

    /// Returns how deeply scopes are nested. If no scope is nested within another scope, this will
    /// be 1. If no scopes were found, this will be 0.
    pub fn nesting(&self) -> usize {
        self.nesting
    }

    /// Returns the nesting level of the given scope. Top-level scopes are 0, every inner scope is 1
    /// larger than its outer scope.
    pub fn nesting_level(&self, scope: I) -> Option<usize> {
        Some(self.scope_meta.get(scope)?.nesting_level)
    }
}

pub type ThisScopeVerificationError = NestedScopeVerificationError<ir::ThisScope>;
pub type ThisScopeLiveness = NestedScopeLiveness<ir::ThisScope>;

impl ThisScopeLiveness {
    pub fn compute<S>(
        ir: &ir::Function<S>,
    ) -> Result<NestedScopeLiveness<ir::ThisScope>, ThisScopeVerificationError> {
        NestedScopeLiveness::compute_with(
            ir,
            |inst| match inst.kind {
                ir::InstructionKind::OpenThisScope(scope) => Some(ScopeOperation::Open(scope)),
                ir::InstructionKind::SetThis(scope, _) => Some(ScopeOperation::Use(scope)),
                ir::InstructionKind::CloseThisScope(scope) => Some(ScopeOperation::Close(scope)),
                _ => None,
            },
            |_| None,
        )
    }
}

pub type CallScopeVerificationError = NestedScopeVerificationError<ir::CallScope>;
pub type CallScopeLiveness = NestedScopeLiveness<ir::CallScope>;

impl CallScopeLiveness {
    pub fn compute<S>(
        ir: &ir::Function<S>,
    ) -> Result<NestedScopeLiveness<ir::CallScope>, CallScopeVerificationError> {
        NestedScopeLiveness::compute_with(
            ir,
            |inst| match inst.kind {
                ir::InstructionKind::OpenCallScope(scope) => Some(ScopeOperation::Open(scope)),
                ir::InstructionKind::StackPush(scope, _) => Some(ScopeOperation::Use(scope)),
                ir::InstructionKind::Call { scope, .. } => Some(ScopeOperation::Use(scope)),
                ir::InstructionKind::StackGet(scope, _) => Some(ScopeOperation::Use(scope)),
                ir::InstructionKind::CloseCallScope(scope) => Some(ScopeOperation::Close(scope)),
                _ => None,
            },
            |exit| match exit.kind {
                ir::ExitKind::Return { call_scope, .. } => Some(ScopeOperation::Close(call_scope)),
                _ => None,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use fabricator_vm::{FunctionRef, Span};

    use crate::constant::Constant;

    use super::*;

    #[test]
    fn test_nested_scopes_loop_closes() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut this_scopes = ir::ThisScopeSet::new();

        let scope = this_scopes.insert(());

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(scope),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(scope),
                span: Span::null(),
            }));

        block_b.exit.kind = ir::ExitKind::Jump(block_b_id);

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars: Default::default(),
            this_scopes,
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        assert!(matches!(
            ThisScopeLiveness::compute(&ir),
            Err(NestedScopeVerificationError {
                kind: NestedScopeVerificationErrorKind::IndeterminateState(..),
                ..
            })
        ));
    }

    #[test]
    fn test_nested_scopes_loop_reopens() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut this_scopes = ir::ThisScopeSet::new();

        let scope = this_scopes.insert(());

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
                kind: ir::InstructionKind::OpenThisScope(scope),
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
                kind: ir::InstructionKind::CloseThisScope(scope),
                span: Span::null(),
            }));

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars: Default::default(),
            this_scopes,
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        assert!(matches!(
            ThisScopeLiveness::compute(&ir),
            Err(NestedScopeVerificationError {
                kind: NestedScopeVerificationErrorKind::IndeterminateState(..),
                ..
            })
        ));
    }

    #[test]
    fn test_nested_scopes_not_nested() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut this_scopes = ir::ThisScopeSet::new();

        let outer_scope = this_scopes.insert(());
        let inner_scope = this_scopes.insert(());

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(outer_scope),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(inner_scope),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(outer_scope),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(inner_scope),
                span: Span::null(),
            }));

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars: Default::default(),
            this_scopes,
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        assert!(matches!(
            ThisScopeLiveness::compute(&ir),
            Err(NestedScopeVerificationError {
                kind: NestedScopeVerificationErrorKind::ScopeNotNested { .. },
                ..
            })
        ));
    }

    #[test]
    fn test_nested_scopes_accesses_inner() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut this_scopes = ir::ThisScopeSet::new();

        let outer_scope = this_scopes.insert(());
        let inner_scope = this_scopes.insert(());

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(outer_scope),
                span: Span::null(),
            }));

        let this = instructions.insert(ir::Instruction {
            kind: ir::InstructionKind::NewObject,
            span: Span::null(),
        });
        block_a.instructions.push(this);

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(inner_scope),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::SetThis(outer_scope, this),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(inner_scope),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(outer_scope),
                span: Span::null(),
            }));

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars: Default::default(),
            this_scopes,
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        assert!(matches!(
            ThisScopeLiveness::compute(&ir),
            Err(NestedScopeVerificationError {
                kind:
                    NestedScopeVerificationErrorKind::UseOverlapsInner {
                    inner_scope: inner,
                    ..
                },
                ..
            }) if inner == inner_scope
        ));
    }

    #[test]
    fn test_nested_scopes_nesting_level() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut this_scopes = ir::ThisScopeSet::new();

        let outer_scope = this_scopes.insert(());
        let inner_scope = this_scopes.insert(());

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(outer_scope),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::OpenThisScope(inner_scope),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::NoOp,
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(inner_scope),
                span: Span::null(),
            }));

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::CloseThisScope(outer_scope),
                span: Span::null(),
            }));

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars: Default::default(),
            this_scopes,
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        let liveness = ThisScopeLiveness::compute(&ir).unwrap();

        assert!(liveness.nesting() == 2);
        assert!(liveness.deepest_for(ir::InstLocation::new(block_b_id, 1)) == Some(inner_scope));
        assert!(liveness.inner_scopes(outer_scope).eq([inner_scope]));
        assert!(liveness.nesting_level(outer_scope) == Some(0));
        assert!(liveness.nesting_level(inner_scope) == Some(1));
    }
}
