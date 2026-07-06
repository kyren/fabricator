use fabricator_util::{bit_containers::BitSlice as _, typed_id_map::SecondaryMap};
use fabricator_vm::instructions::HeapIdx;
use rustc_hash::FxHashMap;

use crate::{
    analysis::variable_liveness::VariableLiveness,
    code_gen::{ProtoGenError, prototype::HeapVarDescriptor},
    graph::dfs::topological_order,
    ir,
};

#[derive(Debug)]
pub struct HeapAllocation<S> {
    pub heap_var_descriptors: Vec<HeapVarDescriptor<S>>,
    pub heap_indexes: SecondaryMap<ir::VarId, HeapIdx>,
}

impl<S: Clone> HeapAllocation<S> {
    /// Determine descriptors for all variables.
    ///
    /// For all owned variables, will try to combine variables with independent lifetimes into the
    /// same index.
    pub fn allocate(
        ir: &ir::Function<S>,
        variable_liveness: &VariableLiveness,
        parent_heap_indexes: &SecondaryMap<ir::VarId, HeapIdx>,
    ) -> Result<Self, ProtoGenError> {
        let mut heap_vars = Vec::new();
        let mut heap_indexes: SecondaryMap<ir::VarId, HeapIdx> = SecondaryMap::new();

        // First, assign all of the non-owned variables because those require no analysis.
        for (var_id, var) in ir.variables.iter() {
            let desc = match var {
                ir::Variable::Heap => continue,
                ir::Variable::Static(init) => HeapVarDescriptor::Static(init.clone()),
                &ir::Variable::Upper(parent_var_id) => HeapVarDescriptor::UpValue(
                    *parent_heap_indexes
                        .get(parent_var_id)
                        .expect("upvalue not present in parent"),
                ),
            };

            let index: HeapIdx = heap_vars
                .len()
                .try_into()
                .map_err(|_| ProtoGenError::HeapVarOverflow)?;
            heap_indexes.insert(var_id, index);
            heap_vars.push(desc);
        }

        // Like SSA instructions, owned variables can be assigned in a single pass. Because we know
        // that a variable cannot become live again after its range ends, we can do a single pass
        // over the CFG in topological order and assign indexes as we go.

        let mut assigned_indexes = SecondaryMap::<ir::VarId, HeapIdx>::new();

        let block_order =
            topological_order(ir.start_block, |id| ir.blocks[id].exit.kind.successors());

        let mut available_indexes = Vec::new();
        for &block_id in &block_order {
            let block = &ir.blocks[block_id];

            // The set of heap indexes that are used at the start of this block.
            //
            // All upvalue and static indexes are always added to this set unconditionally.
            let mut live_in_indexes = [0u8; (u16::MAX as usize + 1) / 8];
            for i in 0..heap_vars.len() {
                live_in_indexes.set_bit(i, true);
            }

            let mut var_life_starts = FxHashMap::default();
            let mut var_life_ends = FxHashMap::default();
            for (var_id, range) in variable_liveness.live_for_block(block_id) {
                if let Some(start) = range.start {
                    assert!(var_life_starts.insert(start, var_id).is_none());
                } else {
                    live_in_indexes.set_bit(assigned_indexes[var_id].0 as usize, true);
                }

                if let Some(end) = range.end {
                    var_life_ends
                        .entry(end)
                        .or_insert_with(Vec::new)
                        .push(var_id);
                }
            }

            available_indexes.clear();
            available_indexes.extend((0u16..=u16::MAX).rev().flat_map(|index| {
                if live_in_indexes.get_bit(index as usize) {
                    None
                } else {
                    Some(HeapIdx(index))
                }
            }));

            for inst_index in 0..=block.instructions.len() {
                if let Some(&var_life_start) = var_life_starts.get(&inst_index) {
                    let idx = available_indexes
                        .pop()
                        .ok_or(ProtoGenError::HeapVarOverflow)?;
                    assert!(assigned_indexes.insert(var_life_start, idx).is_none());
                }

                for &var_life_end in var_life_ends.get(&inst_index).into_iter().flatten() {
                    available_indexes.push(assigned_indexes[var_life_end]);
                }
            }
        }

        if !assigned_indexes.is_empty() {
            let mut max_idx = None;
            for (var_id, heap_idx) in assigned_indexes.into_iter() {
                assert!(heap_idx.0 as usize >= heap_vars.len());
                assert!(heap_indexes.insert(var_id, heap_idx).is_none());
                max_idx = max_idx.max(Some(heap_idx));
            }

            let owned_start = heap_vars.len();
            for idx in heap_vars.len() as u16..=max_idx.unwrap().0 {
                heap_vars.push(HeapVarDescriptor::Owned(HeapIdx(idx - owned_start as u16)));
            }
        }

        Ok(Self {
            heap_var_descriptors: heap_vars,
            heap_indexes,
        })
    }
}
