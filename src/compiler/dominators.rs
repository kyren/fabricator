use crate::util::index_containers::{IndexMap, IndexSet};

pub trait Node: Copy + Eq {
    /// Must be a unique, low-valued array index, algorithms may use memory proportional to the
    /// maximum length returned here.
    fn index(&self) -> usize;
}

impl Node for usize {
    fn index(&self) -> usize {
        *self
    }
}

/// Calculate dominators and dominance frontiers for every node in a directed graph.
#[derive(Debug)]
pub struct Dominators<N> {
    // We reference all nodes internally by DFS post-order index, and we keep dictionaries to go
    // from post-order index <-> node.
    postorder: Vec<N>,
    postorder_indexes: IndexMap<usize>,

    dominators: Vec<usize>,
    dominance_ranges: Vec<(usize, usize)>,
    dominance_frontiers: Vec<IndexSet>,
}

impl<N: Node> Dominators<N> {
    /// Calculate the dominator tree and dominance frontiers for every reachable node a directed
    /// graph, starting with node `start`.
    ///
    /// The `edges` function should return all nodes which are reachable from the given node
    /// parameter in the directed graph.
    ///
    /// The `start` node is always considered to dominate every other node.
    pub fn compute<I>(start: N, edges: impl Fn(N) -> I) -> Self
    where
        I: IntoIterator<Item = N>,
    {
        let postorder = {
            let mut postorder = Vec::new();

            let mut stack = Vec::new();
            let mut visited = IndexSet::new();
            stack.push(start);
            visited.insert(start.index());

            while let Some(node) = stack.last().copied() {
                let mut leaf = true;
                for en in edges(node) {
                    if !visited.contains(en.index()) {
                        visited.insert(en.index());
                        leaf = false;
                        stack.push(en);
                    }
                }

                if leaf {
                    postorder.push(node);
                    stack.pop();
                }
            }

            postorder
        };

        let postorder_indexes = {
            let mut postorder_indexes = IndexMap::new();
            for i in 0..postorder.len() {
                postorder_indexes.insert(postorder[i].index(), i);
            }
            postorder_indexes
        };

        let predecessors = {
            let mut predecessors = Vec::new();
            for _ in 0..postorder.len() {
                predecessors.push(IndexSet::new());
            }

            for &node in &postorder {
                for en in edges(node) {
                    predecessors
                        .get_mut(postorder_indexes[en.index()])
                        .unwrap()
                        .insert(postorder_indexes[node.index()]);
                }
            }
            predecessors
        };

        // Dominance algorithm is sourced from:
        //
        // A Simple, Fast Dominance Algorithm, Cooper et al.
        // https://www.clear.rice.edu/comp512/Lectures/Papers/TR06-33870-Dom.pdf
        let dominators = {
            let intersect =
                |dominators: &IndexMap<usize>, mut finger1: usize, mut finger2: usize| -> usize {
                    while finger1 != finger2 {
                        while finger1 < finger2 {
                            finger1 = dominators[finger1];
                        }
                        while finger2 < finger1 {
                            finger2 = dominators[finger2];
                        }
                    }
                    finger1
                };

            let mut dominators = IndexMap::new();
            dominators.insert(
                postorder_indexes[start.index()],
                postorder_indexes[start.index()],
            );

            let mut changed = true;
            while changed {
                changed = false;

                for node in postorder.iter().rev().copied() {
                    if node == start {
                        continue;
                    }

                    let ni = postorder_indexes[node.index()];
                    let mut new_idom = None;
                    for p in predecessors.get(ni).unwrap().iter() {
                        if dominators.contains(p) {
                            new_idom = Some(match new_idom {
                                Some(idom) => intersect(&dominators, p, idom),
                                None => p,
                            });
                        }
                    }

                    // We should be iterating in an order where every node will have at least one
                    // predecessor with a computed dominator.
                    let new_idom = new_idom
                        .expect("all nodes should have a predecessor with a computed dominator");
                    if dominators.get(ni).copied() != Some(new_idom) {
                        dominators.insert(ni, new_idom);
                        changed = true;
                    }
                }
            }

            // Every dominator should be computed at this point
            (0..postorder.len())
                .map(|i| dominators[i])
                .collect::<Vec<_>>()
        };

        // Find ranges that represent all nodes (in post-order index) that a given node dominates.
        //
        // Makes "does A dominate B?" queries O(1).
        let dominance_ranges = {
            // Start with every node dominating itself.
            let mut dominance_ranges = (0..postorder.len()).map(|i| (i, i)).collect::<Vec<_>>();

            // We walk the nodes here in DFS post-order. Since a node's immediate dominator MUST
            // come after it in DFS post-order, we just need to unify a every node's range with
            // its immediate dominator in this loop and all dominance ranges will be unified in
            // one pass.
            for i in 0..postorder.len() {
                assert!(i <= dominators[i]);
                if i != dominators[i] {
                    let (start, end) = dominance_ranges[i];
                    let (idom_start, idom_end) = &mut dominance_ranges[dominators[i]];
                    *idom_start = (*idom_start).min(start);
                    *idom_end = (*idom_end).max(end);
                }
            }

            dominance_ranges
        };

        // Dominance frontier algorithm is also sourced from:
        //
        // A Simple, Fast Dominance Algorithm, Cooper et al.
        // https://www.clear.rice.edu/comp512/Lectures/Papers/TR06-33870-Dom.pdf
        let dominance_frontiers = {
            let mut dominance_frontiers = vec![IndexSet::new(); postorder.len()];

            for i in 0..postorder.len() {
                if predecessors[i].len() >= 2 {
                    for p in predecessors[i].iter() {
                        let mut runner = p;
                        while runner != dominators[i] {
                            dominance_frontiers[runner].insert(i);
                            runner = dominators[runner];
                        }
                    }
                }
            }

            dominance_frontiers
        };

        Dominators {
            postorder,
            postorder_indexes,
            dominators,
            dominance_ranges,
            dominance_frontiers,
        }
    }

    /// Return the immediate dominator ("idom") of the given node.
    ///
    /// Returns `None` if the given node `n` was not reachable when `Dominators` was constructed and
    /// thus has no dominance information.
    pub fn idom(&self, n: N) -> Option<N> {
        Some(self.postorder[self.dominators[self.postorder_indexes.get(n.index()).copied()?]])
    }

    /// Queries whether node `a` dominates node `b`.
    ///
    /// Post-order index ranges are calculated during `Dominators` construction that make this query
    /// O(1).
    ///
    /// Returns `None` if the either of the given nodes `a` or `b` was not reachable when
    /// `Dominators` was constructed and thus no dominance can be determined.
    pub fn dominates(&self, a: N, b: N) -> Option<bool> {
        let (a_start, a_end) =
            self.dominance_ranges[self.postorder_indexes.get(a.index()).copied()?];
        let (b_start, b_end) =
            self.dominance_ranges[self.postorder_indexes.get(b.index()).copied()?];
        Some(a_start <= b_start && a_end >= b_end)
    }

    /// Return the (precalculated) dominance frontier of the given node.
    ///
    /// Returns `None` if the given node `n` was not reachable when `Dominators` was constructed and
    /// thus has no dominance information.
    pub fn dominance_frontier(&self, n: N) -> Option<impl Iterator<Item = N> + '_> {
        Some(
            self.dominance_frontiers[self.postorder_indexes.get(n.index()).copied()?]
                .iter()
                .map(|n| self.postorder[n]),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::util::index_containers::IndexMap;

    use super::*;

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct TestNode(usize, &'static str);

    #[derive(Default)]
    struct TestGraph {
        next_node: usize,
        edges: IndexMap<Vec<TestNode>>,
    }

    impl Node for TestNode {
        fn index(&self) -> usize {
            self.0
        }
    }

    impl TestGraph {
        pub fn create_node(&mut self, name: &'static str) -> TestNode {
            let i = self.next_node;
            self.next_node += 1;
            let n = TestNode(i, name);

            self.edges.insert(n.index(), Vec::new());
            n
        }

        pub fn add_edge(&mut self, from: TestNode, to: TestNode) {
            self.edges.get_mut(from.index()).unwrap().push(to);
        }

        fn edges_from(&self, node: TestNode) -> impl Iterator<Item = TestNode> + '_ {
            self.edges.get(node.index()).unwrap().iter().copied()
        }
    }

    #[test]
    fn test_dominator_tree() {
        let mut graph = TestGraph::default();

        // [A]-->[B]--+
        //       /    |
        //      /     |
        //     V      V
        //   [C]<----[D]
        //   / \      |
        //  V   V     |
        // [E]  [F]<--+
        //  ^
        //  |
        // [G] # Not reachable

        let a = graph.create_node("A");
        let b = graph.create_node("B");
        let c = graph.create_node("C");
        let d = graph.create_node("D");
        let e = graph.create_node("E");
        let f = graph.create_node("F");

        // `G` should not be reachable and should have no dominance information
        let g = graph.create_node("G");

        graph.add_edge(a, b);
        graph.add_edge(b, c);
        graph.add_edge(b, d);
        graph.add_edge(c, e);
        graph.add_edge(c, f);
        graph.add_edge(d, c);
        graph.add_edge(d, f);

        graph.add_edge(g, e);

        let tree = Dominators::compute(a, |n| graph.edges_from(n));

        assert_eq!(tree.idom(a).unwrap(), a);
        assert_eq!(tree.idom(b).unwrap(), a);
        assert_eq!(tree.idom(c).unwrap(), b);
        assert_eq!(tree.idom(d).unwrap(), b);
        assert_eq!(tree.idom(e).unwrap(), c);
        assert_eq!(tree.idom(f).unwrap(), b);
        assert!(tree.idom(g).is_none());

        let dominating_pairs = [
            (a, a),
            (a, b),
            (a, c),
            (a, d),
            (a, e),
            (a, f),
            (b, b),
            (b, c),
            (b, d),
            (b, e),
            (b, f),
            (c, c),
            (c, e),
            (d, d),
            (e, e),
            (f, f),
        ];

        for na in [a, b, c, d, e, f] {
            for nb in [a, b, c, d, e, f] {
                assert!(tree.dominates(na, nb).unwrap() == dominating_pairs.contains(&(na, nb)));
            }

            assert!(tree.dominates(na, g).is_none());
        }

        let dominance_frontiers = [
            (a, vec![]),
            (b, vec![]),
            (c, vec![f]),
            (d, vec![c, f]),
            (e, vec![]),
            (f, vec![]),
        ];

        for (n, domf) in dominance_frontiers {
            let observed_domf = tree.dominance_frontier(n).unwrap().collect::<Vec<_>>();
            for f in &observed_domf {
                assert!(domf.contains(f));
            }
            for f in &domf {
                assert!(observed_domf.contains(f));
            }
        }

        assert!(tree.dominance_frontier(g).is_none());
    }
}
