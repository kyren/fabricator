use fabricator_math::{Box2, Vec2};
use rand::RngExt;

/// A 4D k-d tree of 2D bound boxes, allowing for efficient bounds queries.
///
/// Construction of the k-d tree is O(n * log(n)) and has cost similar to a simple quicksort.
///
/// The cost of querying the tree is difficult to analyze exactly, but it is generally much better
/// than linear and can average as low as O(log(n)) for queries that return a single result.
///
/// Rebuilding and querying the k-d tree will re-use internal buffers and, unless the internal
/// buffers are grown, requires no allocation.
#[derive(Debug, Clone)]
pub struct BoundBoxTree<N, T> {
    nodes: Vec<Node<N, T>>,
    root: NodeIndex,
}

impl<N, T> Default for BoundBoxTree<N, T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            root: INVALID_NODE,
        }
    }
}

impl<N, T> BoundBoxTree<N, T> {
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = INVALID_NODE;
    }

    pub fn is_empty(&self) -> bool {
        self.root == INVALID_NODE
    }
}

impl<N: Ord, T> BoundBoxTree<N, T> {
    pub fn build(i: impl IntoIterator<Item = (Box2<N>, T)>) -> Self {
        let mut this = Self::default();
        this.extend(i);
        this
    }

    /// Add all of the given entries to this tree.
    ///
    /// Existing values are not removed, and this triggers a full rebuild of the inner k-d tree.
    pub fn extend(&mut self, i: impl IntoIterator<Item = (Box2<N>, T)>) {
        self.nodes.extend(i.into_iter().map(|(bounds, value)| Node {
            bounds,
            value,
            left: INVALID_NODE,
            right: INVALID_NODE,
        }));

        self.root = build(&mut self.nodes, N::lt);
    }
}

impl<N: num::Float, T> BoundBoxTree<N, T> {
    pub fn fbuild(i: impl IntoIterator<Item = (Box2<N>, T)>) -> Self {
        let mut this = Self::default();
        this.fextend(i);
        this
    }

    /// Extend this tree with the given entries, using floating point coordinates.
    ///
    /// Any entries with bounds that contain a `NaN` are filtered and can never be returned from
    /// a query.
    pub fn fextend(&mut self, i: impl IntoIterator<Item = (Box2<N>, T)>) {
        self.nodes
            .extend(i.into_iter().filter_map(|(bounds, value)| {
                if !bounds.min.into_iter().any(N::is_nan) && !bounds.max.into_iter().any(N::is_nan)
                {
                    Some(Node {
                        bounds,
                        value,
                        left: INVALID_NODE,
                        right: INVALID_NODE,
                    })
                } else {
                    None
                }
            }));

        self.root = build(&mut self.nodes, |a, b| {
            debug_assert!(!a.is_nan() && !b.is_nan());
            a < b
        });
    }
}

/// A reusable buffer for performing queries on a `BoundBoxTree`.
#[derive(Default)]
pub struct BoundBoxQuery {
    stack: Vec<(NodeIndex, Depth)>,
}

impl BoundBoxQuery {
    pub fn intersects<'a, N: PartialOrd + Copy, T>(
        &'a mut self,
        tree: &'a BoundBoxTree<N, T>,
        bounds: Box2<N>,
    ) -> impl Iterator<Item = &'a T> {
        struct OverlapsQuery<N>(Box2<N>);

        impl<N: PartialOrd + Copy> Query<N> for OverlapsQuery<N> {
            fn limit_by_xmin(&self, xmin: &N) -> bool {
                self.0.max[0] > *xmin
            }

            fn limit_by_ymin(&self, ymin: &N) -> bool {
                self.0.max[1] > *ymin
            }

            fn limit_by_xmax(&self, xmax: &N) -> bool {
                self.0.min[0] < *xmax
            }

            fn limit_by_ymax(&self, ymax: &N) -> bool {
                self.0.min[1] < *ymax
            }

            fn test_bounds(&self, rect: &Box2<N>) -> bool {
                self.0.intersects(*rect)
            }
        }

        self.query(tree, OverlapsQuery(bounds))
    }

    pub fn contains<'a, N: PartialOrd + Copy, T>(
        &'a mut self,
        tree: &'a BoundBoxTree<N, T>,
        point: Vec2<N>,
    ) -> impl Iterator<Item = &'a T> {
        struct ContainsQuery<N>(Vec2<N>);

        impl<N: PartialOrd + Copy> Query<N> for ContainsQuery<N> {
            fn limit_by_xmin(&self, xmin: &N) -> bool {
                self.0[0] >= *xmin
            }

            fn limit_by_ymin(&self, ymin: &N) -> bool {
                self.0[1] >= *ymin
            }

            fn limit_by_xmax(&self, xmax: &N) -> bool {
                self.0[0] < *xmax
            }

            fn limit_by_ymax(&self, ymax: &N) -> bool {
                self.0[1] < *ymax
            }

            fn test_bounds(&self, rect: &Box2<N>) -> bool {
                rect.contains(self.0)
            }
        }

        self.query(tree, ContainsQuery(point))
    }

    fn query<'a, N, T>(
        &'a mut self,
        tree: &'a BoundBoxTree<N, T>,
        query: impl Query<N>,
    ) -> impl Iterator<Item = &'a T> {
        QueryIter::new(&tree.nodes, &mut self.stack, query, tree.root)
    }
}

type NodeIndex = usize;
type Depth = usize;

const INVALID_NODE: NodeIndex = NodeIndex::MAX;

#[derive(Debug, Clone)]
struct Node<N, T> {
    bounds: Box2<N>,
    value: T,
    left: NodeIndex,
    right: NodeIndex,
}

fn build<N, T>(nodes: &mut Vec<Node<N, T>>, less_than: impl Fn(&N, &N) -> bool) -> NodeIndex {
    fn build_sub<N, T>(
        rng: &mut impl rand::Rng,
        nodes: &mut Vec<Node<N, T>>,
        depth: Depth,
        min: NodeIndex,
        max: NodeIndex,
        less_than: &impl Fn(&N, &N) -> bool,
    ) -> NodeIndex {
        if min >= max {
            return INVALID_NODE;
        }

        let mid = min + (max - min) / 2;
        // Find the median element and partition the array based on the appropriate ordering for
        // this dimension. Each bound box is represented as the 4D point [xmin, ymin, xmax, ymax].
        quickselect(rng, &mut nodes[min..max], mid - min, |a, b| {
            match depth % 4 {
                0 => less_than(&a.bounds.min[0], &b.bounds.min[0]),
                1 => less_than(&a.bounds.min[1], &b.bounds.min[1]),
                2 => less_than(&a.bounds.max[0], &b.bounds.max[0]),
                3 => less_than(&a.bounds.max[1], &b.bounds.max[1]),
                _ => unreachable!(),
            }
        });
        let depth = depth.checked_add(1).unwrap();

        nodes[mid].left = build_sub(rng, nodes, depth, min, mid, less_than);
        nodes[mid].right = build_sub(rng, nodes, depth, mid + 1, max, less_than);

        mid
    }

    build_sub(&mut rand::rng(), nodes, 0, 0, nodes.len(), &less_than)
}

trait Query<N> {
    fn limit_by_xmin(&self, xmin: &N) -> bool;
    fn limit_by_ymin(&self, ymin: &N) -> bool;
    fn limit_by_xmax(&self, xmax: &N) -> bool;
    fn limit_by_ymax(&self, ymax: &N) -> bool;

    fn test_bounds(&self, rect: &Box2<N>) -> bool;
}

struct QueryIter<'a, N, T, Q> {
    nodes: &'a [Node<N, T>],
    stack: &'a mut Vec<(NodeIndex, Depth)>,
    query: Q,
}

impl<'a, N, T, Q> QueryIter<'a, N, T, Q> {
    fn new(
        nodes: &'a [Node<N, T>],
        stack: &'a mut Vec<(NodeIndex, Depth)>,
        query: Q,
        root: NodeIndex,
    ) -> Self {
        stack.clear();
        if root != INVALID_NODE {
            stack.push((root, 0));
        }

        QueryIter {
            nodes,
            stack,
            query,
        }
    }
}

impl<'a, N, T, Q> Iterator for QueryIter<'a, N, T, Q>
where
    Q: Query<N>,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((node_index, depth)) = self.stack.pop() {
            let node = &self.nodes[node_index];
            let lower_depth = depth + 1;

            // For each dimension, we can potentially limit *one* of the left or right k-d nodes
            // based on the bound that this dimension represents.
            //
            // If this dimension is a minimum (xmin or ymin), then we only have to go down the right
            // node when we know that the query may intersect with a min bound greater than this
            // node's value (since the right node xmin / ymin must be greater than this one's).
            //
            // Conversely, if this dimension is a maximum (xmax or ymax), then we only have to
            // go down the left node when we know that the query may intersect with a max bound
            // less than this node's value (since the left node xmax / ymax must be less than this
            // one's).

            match depth % 4 {
                0 => {
                    if node.left != INVALID_NODE {
                        self.stack.push((node.left, lower_depth));
                    }
                    if node.right != INVALID_NODE {
                        if self.query.limit_by_xmin(&node.bounds.min[0]) {
                            self.stack.push((node.right, lower_depth));
                        }
                    }
                }
                1 => {
                    if node.left != INVALID_NODE {
                        self.stack.push((node.left, lower_depth));
                    }
                    if node.right != INVALID_NODE {
                        if self.query.limit_by_ymin(&node.bounds.min[1]) {
                            self.stack.push((node.right, lower_depth));
                        }
                    }
                }
                2 => {
                    if node.left != INVALID_NODE {
                        if self.query.limit_by_xmax(&node.bounds.max[0]) {
                            self.stack.push((node.left, lower_depth));
                        }
                    }
                    if node.right != INVALID_NODE {
                        self.stack.push((node.right, lower_depth));
                    }
                }
                3 => {
                    if node.left != INVALID_NODE {
                        if self.query.limit_by_ymax(&node.bounds.max[1]) {
                            self.stack.push((node.left, lower_depth));
                        }
                    }
                    if node.right != INVALID_NODE {
                        self.stack.push((node.right, lower_depth));
                    }
                }
                _ => unreachable!(),
            }

            if self.query.test_bounds(&node.bounds) {
                return Some(&node.value);
            }
        }

        None
    }
}

// Partially sort the given slice, making sure that the kth element is in the correct position.
//
// https://en.wikipedia.org/wiki/Quickselect
fn quickselect<T>(
    rng: &mut impl rand::Rng,
    list: &mut [T],
    k: usize,
    less_than: impl Fn(&T, &T) -> bool,
) {
    assert!(k < list.len());

    let mut left = 0;
    let mut right = list.len() - 1;

    let mut partition = |left, right, pivot| -> usize {
        list.swap(pivot, right);

        let mut store_index = left;
        for i in left..right {
            if less_than(&list[i], &list[right]) {
                list.swap(store_index, i);
                store_index += 1;
            }
        }

        list.swap(right, store_index);
        store_index
    };

    loop {
        // We pick a random pivot value here, which means that the quickselect algorithm has "almost
        // certain" linear time.
        let mut pivot = rng.random_range(left..=right);
        pivot = partition(left, right, pivot);

        if k == pivot {
            return;
        } else if k < pivot {
            right = pivot - 1;
        } else {
            left = pivot + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bound_box_tree() {
        let mut bb_tree = BoundBoxTree::default();

        bb_tree.fextend([
            (Box2::new([1.0, 1.0].into(), [2.0, 2.0].into()), 0),
            (Box2::new([1.0, 3.0].into(), [2.0, 4.0].into()), 1),
            (Box2::new([3.0, 1.0].into(), [4.0, 2.0].into()), 2),
            (Box2::new([3.0, 3.0].into(), [4.0, 4.0].into()), 3),
        ]);

        let mut bb_query = BoundBoxQuery::default();

        assert_eq!(
            bb_query
                .intersects(
                    &bb_tree,
                    Box2::new([1.25, 1.25].into(), [1.75, 1.75].into())
                )
                .collect::<Vec<_>>(),
            &[&0]
        );

        assert_eq!(
            bb_query
                .intersects(
                    &bb_tree,
                    Box2::new([1.25, 3.25].into(), [1.75, 3.75].into())
                )
                .collect::<Vec<_>>(),
            &[&1]
        );

        assert_eq!(
            bb_query
                .intersects(
                    &bb_tree,
                    Box2::new([3.25, 1.25].into(), [3.75, 1.75].into())
                )
                .collect::<Vec<_>>(),
            &[&2]
        );

        assert_eq!(
            bb_query
                .intersects(
                    &bb_tree,
                    Box2::new([3.25, 3.25].into(), [3.75, 3.75].into())
                )
                .collect::<Vec<_>>(),
            &[&3]
        );

        assert_eq!(
            bb_query
                .intersects(
                    &bb_tree,
                    Box2::new([0.25, 0.25].into(), [0.75, 0.75].into())
                )
                .count(),
            0
        );
        assert_eq!(
            bb_query
                .intersects(
                    &bb_tree,
                    Box2::new([4.25, 4.25].into(), [4.75, 4.75].into())
                )
                .count(),
            0
        );

        assert_eq!(
            bb_query
                .contains(&bb_tree, [1.5, 1.5].into())
                .collect::<Vec<_>>(),
            &[&0]
        );

        assert_eq!(
            bb_query
                .contains(&bb_tree, [1.5, 3.5].into())
                .collect::<Vec<_>>(),
            &[&1]
        );

        assert_eq!(
            bb_query
                .contains(&bb_tree, [3.5, 1.5].into())
                .collect::<Vec<_>>(),
            &[&2]
        );

        assert_eq!(
            bb_query
                .contains(&bb_tree, [3.5, 3.5].into())
                .collect::<Vec<_>>(),
            &[&3]
        );

        assert_eq!(bb_query.contains(&bb_tree, [0.5, 0.5].into()).count(), 0);
        assert_eq!(bb_query.contains(&bb_tree, [4.5, 4.5].into()).count(), 0);
    }
}
