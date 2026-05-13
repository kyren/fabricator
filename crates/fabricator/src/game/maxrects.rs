use std::cmp::{max, min};

use fabricator_math::{Box2, Vec2};

pub struct MaxRects<N, I> {
    size: Vec2<N>,
    to_place: Vec<PackedRect<N, I>>,
}

pub struct PackedRect<N, I> {
    pub item: I,
    pub size: Vec2<N>,
    pub placement: Option<Vec2<N>>,
}

impl<N, I> MaxRects<N, I>
where
    N: Copy + num::Num + Ord,
{
    pub fn new(size: Vec2<N>) -> Self {
        MaxRects {
            size,
            to_place: Vec::new(),
        }
    }

    pub fn add(&mut self, item: I, size: Vec2<N>) {
        self.to_place.push(PackedRect {
            item,
            size,
            placement: None,
        });
    }

    pub fn pack(self) -> Vec<PackedRect<N, I>> {
        let MaxRects { size, mut to_place } = self;
        let mut working = (0..to_place.len()).collect::<Vec<_>>();
        let mut free_rects = vec![Box2::with_size(Vec2::zero(), size)];
        let mut split_free_rects = Vec::new();

        loop {
            let mut best_placement = None;
            let mut working_idx = 0;

            while working_idx < working.len() {
                let size = to_place[working[working_idx]].size;
                let mut placement = None;

                for (free_idx, &free) in free_rects.iter().enumerate() {
                    let free_size = free.size();
                    if free_size[0] >= size[0] && free_size[1] >= size[1] {
                        let leftover_horiz = free_size[0] - size[0];
                        let leftover_vert = free_size[1] - size[1];

                        let fit_score = FitScore {
                            short_leftover: min(leftover_horiz, leftover_vert),
                            long_leftover: max(leftover_horiz, leftover_vert),
                        };

                        if placement.is_none_or(|(_, score)| fit_score < score) {
                            placement = Some((free_idx, fit_score));
                        }
                    }
                }

                if let Some((free_idx, fit_score)) = placement {
                    if best_placement
                        .map(|(_, _, score)| fit_score < score)
                        .unwrap_or(true)
                    {
                        best_placement = Some((working_idx, free_idx, fit_score));
                    }
                    working_idx += 1;
                } else {
                    working.swap_remove(working_idx);
                }
            }

            if let Some((working_idx, free_idx, _)) = best_placement {
                let min = free_rects[free_idx].min;
                let place_idx = working.swap_remove(working_idx);
                let place_rect = Box2::with_size(min, to_place[place_idx].size);

                let mut free_idx = 0;
                while free_idx < free_rects.len() {
                    if free_rects[free_idx].intersects(place_rect) {
                        let intersect_free = free_rects.swap_remove(free_idx);
                        if intersect_free.min[0] < place_rect.min[0] {
                            let mut left = intersect_free;
                            left.max[0] = place_rect.min[0];
                            split_free_rects.push(left);
                        }
                        if intersect_free.max[0] > place_rect.max[0] {
                            let mut right = intersect_free;
                            right.min[0] = place_rect.max[0];
                            split_free_rects.push(right);
                        }
                        if intersect_free.min[1] < place_rect.min[1] {
                            let mut top = intersect_free;
                            top.max[1] = place_rect.min[1];
                            split_free_rects.push(top);
                        }
                        if intersect_free.max[1] > place_rect.max[1] {
                            let mut bottom = intersect_free;
                            bottom.min[1] = place_rect.max[1];
                            split_free_rects.push(bottom);
                        }
                    } else {
                        free_idx += 1;
                    }
                }
                free_rects.extend(split_free_rects.drain(..));

                remove_redundant_rects(&mut free_rects);

                to_place[place_idx].placement = Some(min);
            } else {
                break;
            }
        }

        to_place
    }
}

fn remove_redundant_rects<N: Copy + PartialOrd + num::Num>(rects: &mut Vec<Box2<N>>) {
    let mut free_idx = 0;
    while free_idx < rects.len() {
        let free = rects[free_idx];
        if free.is_empty() || rects[free_idx + 1..].iter().any(|r| r.contains_box(free)) {
            rects.swap_remove(free_idx);
            continue;
        }

        for j in (free_idx + 1..rects.len()).rev() {
            if free.contains_box(rects[j]) {
                rects.swap_remove(j);
            }
        }

        free_idx += 1;
    }
}

// A score for how well a placed rect fits into a free rect. Ordered, with lower values being a
// better score than higher values. Sorted first by the smaller leftover space of either the width
// or height, then by the larger.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct FitScore<N> {
    short_leftover: N,
    long_leftover: N,
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, ops::Add};

    use rand::{RngExt, SeedableRng, rngs::SmallRng};

    use super::*;

    fn test_placements_distinct<N, T>(placements: &[PackedRect<N, T>])
    where
        N: PartialOrd + Add<Output = N> + Copy,
    {
        // Assert that no placed rect overlaps any other placed rect
        for i in 0..placements.len() {
            for j in i + 1..placements.len() {
                let a = Box2::with_size(placements[i].placement.unwrap(), placements[i].size);
                let b = Box2::with_size(placements[j].placement.unwrap(), placements[j].size);
                assert!(!a.intersects(b));
            }
        }
    }

    #[test]
    fn remove_redundant() {
        let mut rects = Vec::new();
        for i in 0..10 {
            rects.push(Box2::with_size(Vec2::new(i * 10, 0), Vec2::new(10, 10)));
            rects.push(Box2::with_size(Vec2::new(i * 10 + 1, 1), Vec2::new(9, 9)));
        }

        assert_eq!(rects.len(), 20);
        remove_redundant_rects(&mut rects);
        assert_eq!(rects.len(), 10);

        for rect in &rects {
            assert_eq!(rect.size(), Vec2::new(10, 10));
        }
    }

    #[test]
    fn pack_all() {
        let mut packer = MaxRects::new(Vec2::new(40, 40));
        for i in 0..16 {
            packer.add(i, Vec2::new(10, 10));
        }
        test_placements_distinct(&packer.pack());
    }

    #[test]
    fn pack_all_randomized() {
        let mut rng = SmallRng::seed_from_u64(42);
        let mut packer = MaxRects::new(Vec2::new(100, 100));
        for i in 0..100 {
            packer.add(
                i,
                Vec2::new(10 - rng.random_range(0..3), 10 - rng.random_range(0..3)),
            );
        }
        test_placements_distinct(&packer.pack());
    }

    #[test]
    fn pack_all_except_one() {
        let mut packer = MaxRects::new(Vec2::new(40, 40));
        packer.add(0, Vec2::new(10, 10));
        packer.add(1, Vec2::new(50, 50));
        packer.add(2, Vec2::new(10, 10));
        let packed: HashMap<i32, bool> = packer
            .pack()
            .into_iter()
            .map(|p| (p.item, p.placement.is_some()))
            .collect();

        assert!(packed[&0]);
        assert!(!packed[&1]);
        assert!(packed[&2]);
    }
}
