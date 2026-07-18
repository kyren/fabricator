use fabricator_util::typed_id_map::SecondaryMap;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{analysis::shadow_liveness::ShadowLiveness, graph::dfs::depth_first_search, ir};

pub type UpsilonReachabilityMap = SecondaryMap<ir::ShadowVar, UpsilonReach>;

/// Analyze `ShadowLiveness` to determine, for each live region of the shadow variable, which
/// `Upsilon` instructions may have written to it.
///
/// Any `Upsilon` that could have affected the value of the shadow variable *since the last
/// execution of the `Phi` instruction* is considered to "reach" that region.
pub fn compute_upsilon_reachability<S>(
    ir: &ir::Function<S>,
    shadow_liveness: &ShadowLiveness,
) -> UpsilonReachabilityMap {
    let mut reach_map = UpsilonReachabilityMap::default();

    for shadow_var in shadow_liveness.live_shadow_vars() {
        let mut live_blocks = FxHashSet::default();
        let mut live_upsilons = Vec::new();
        let mut phi_block = None;

        for (block_id, liveness) in shadow_liveness.live_ranges(shadow_var) {
            live_blocks.insert(block_id);

            if let Some(incoming) = liveness.incoming_range {
                if let Some(upsilon) = incoming.start {
                    live_upsilons.push(ir::InstLocation::new(block_id, upsilon));
                }
                phi_block = Some(block_id);
            }

            if let Some(outgoing) = liveness.outgoing_range {
                if let Some(upsilon) = outgoing.start {
                    live_upsilons.push(ir::InstLocation::new(block_id, upsilon));
                }
            }
        }

        let phi_block = phi_block.unwrap();

        let mut outgoing_reach: FxHashMap<ir::BlockId, Vec<ir::InstLocation>> =
            FxHashMap::from_iter(live_blocks.iter().map(|&block_id| (block_id, Vec::new())));

        for &upsilon_loc in &live_upsilons {
            depth_first_search(
                upsilon_loc.block_id,
                |block_id| {
                    outgoing_reach.get_mut(&block_id).unwrap().push(upsilon_loc);

                    // We only need to traverse down parts of the CFG that are live for this shadow
                    // variable.
                    //
                    // We always stop *before* the phi block, because we don't need to mark the
                    // incoming range reachability (since it's always the full set of live upsilons
                    // by definition).
                    //
                    // In the case where there is also an overlapping outgoing range in the phi
                    // block that contains an `Upsilon` instruction, we start iterating on that
                    // block so it won't be skipped.
                    ir.blocks[block_id]
                        .exit
                        .kind
                        .successors()
                        .filter(|&block_id| {
                            live_blocks.contains(&block_id) && block_id != phi_block
                        })
                },
                |_| {},
            );
        }

        reach_map.insert(
            shadow_var,
            UpsilonReach {
                live_upsilons,
                outgoing_reach,
            },
        );
    }

    reach_map
}

#[derive(Debug)]
pub struct UpsilonReach {
    /// Every live `Upsilon` instruction.
    ///
    /// The reach for the single incoming range for the shadow variable is always every live
    /// `Upsilon` instruction, by definition.
    pub live_upsilons: Vec<ir::InstLocation>,

    /// If there is an outgoing range for the shadow variable in the block key, the `HashMap` will
    /// contain every `Upsilon` instruction that may have written to the variable in this region
    /// since the last execution of the `Phi` instruction.
    pub outgoing_reach: FxHashMap<ir::BlockId, Vec<ir::InstLocation>>,
}

#[cfg(test)]
mod tests {
    use fabricator_vm::{FunctionRef, Span};

    use crate::constant::Constant;

    use super::*;

    #[test]
    fn test_upsilon_reach() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut shadow_vars = ir::ShadowVarSet::new();

        let shadow_var = shadow_vars.insert(());

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        let one = instructions.insert(ir::Instruction {
            kind: ir::InstructionKind::Constant(Constant::Integer(1)),
            span: Span::null(),
        });
        block_a.instructions.push(one);
        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::Upsilon(shadow_var, one),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::Phi(shadow_var),
                span: Span::null(),
            }));

        let two = instructions.insert(ir::Instruction {
            kind: ir::InstructionKind::Constant(Constant::Integer(2)),
            span: Span::null(),
        });
        block_b.instructions.push(two);
        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::Upsilon(shadow_var, two),
                span: Span::null(),
            }));

        block_b.exit.kind = ir::ExitKind::Jump(block_b_id);

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars,
            this_scopes: Default::default(),
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        let shadow_liveness = ShadowLiveness::compute(&ir).unwrap();
        let upsilon_reach = compute_upsilon_reachability(&ir, &shadow_liveness);

        assert!(
            shadow_liveness
                .live_range_in_block(block_b_id, shadow_var)
                .unwrap()
                .incoming_range
                .is_some()
        );
        assert_eq!(
            upsilon_reach[shadow_var].outgoing_reach[&block_a_id],
            [ir::InstLocation::new(block_a_id, 1)]
        );
        assert_eq!(
            upsilon_reach[shadow_var].outgoing_reach[&block_b_id],
            [ir::InstLocation::new(block_b_id, 2)]
        );
    }

    #[test]
    fn test_upsilon_reach_overlap() {
        let mut instructions = ir::InstructionMap::<&'static str>::new();
        let mut blocks = ir::BlockMap::new();
        let mut shadow_vars = ir::ShadowVarSet::new();

        let shadow_var = shadow_vars.insert(());

        let block_a_id = blocks.insert(ir::Block::default());
        let block_b_id = blocks.insert(ir::Block::default());

        let block_a = &mut blocks[block_a_id];

        let one = instructions.insert(ir::Instruction {
            kind: ir::InstructionKind::Constant(Constant::Integer(1)),
            span: Span::null(),
        });
        block_a.instructions.push(one);
        block_a
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::Upsilon(shadow_var, one),
                span: Span::null(),
            }));

        block_a.exit.kind = ir::ExitKind::Jump(block_b_id);

        let block_b = &mut blocks[block_b_id];

        block_b
            .instructions
            .push(instructions.insert(ir::Instruction {
                kind: ir::InstructionKind::Phi(shadow_var),
                span: Span::null(),
            }));

        block_b.exit.kind = ir::ExitKind::Jump(block_b_id);

        let ir = ir::Function {
            reference: FunctionRef::Chunk,
            instructions,
            blocks,
            variables: Default::default(),
            shadow_vars,
            this_scopes: Default::default(),
            call_scopes: Default::default(),
            functions: Default::default(),
            start_block: block_a_id,
        };

        let shadow_liveness = ShadowLiveness::compute(&ir).unwrap();
        let upsilon_reach = compute_upsilon_reachability(&ir, &shadow_liveness);

        assert!(
            shadow_liveness
                .live_range_in_block(block_b_id, shadow_var)
                .unwrap()
                .incoming_range
                .is_some()
        );
        assert_eq!(
            upsilon_reach[shadow_var].outgoing_reach[&block_a_id],
            [ir::InstLocation::new(block_a_id, 1)]
        );
        assert!(
            shadow_liveness
                .live_range_in_block(block_b_id, shadow_var)
                .unwrap()
                .outgoing_range
                .unwrap()
                .start
                .is_none()
        );
        assert_eq!(upsilon_reach[shadow_var].outgoing_reach[&block_b_id], []);
    }
}
