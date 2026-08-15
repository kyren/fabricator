use fabricator_collision::{
    bound_box_tree::BoundBoxQuery, gjk, support_ext::SupportMapExt, support_maps,
};
use fabricator_math::Vec2;
use fabricator_vm as vm;

use crate::{
    api::{magic::MagicExt as _, object},
    state::{EventState, State},
};

pub fn collision_api<'gc>(ctx: vm::Context<'gc>) -> vm::MagicSet<'gc> {
    let mut magic = vm::MagicSet::new();

    let collision_line = vm::Callback::from_fn(ctx, |ctx, mut exec| {
        let (x1, y1, x2, y2, _obj, _prec, notme): (f64, f64, f64, f64, vm::UserData, bool, bool) =
            exec.stack().consume(ctx)?;

        let line_collision = support_maps::Line([Vec2::new(x1, y1), Vec2::new(x2, y2)]);
        let bounds = line_collision.bound_box();

        let self_instance = EventState::ctx_with(ctx, |event| event.instance_id).ok();

        State::ctx_with(ctx, |state| {
            let mut query = BoundBoxQuery::default();
            for &instance_id in query.intersects(&state.instance_bound_tree, bounds) {
                let Some(instance) = state.instances.get(instance_id) else {
                    continue;
                };

                if notme {
                    if self_instance == Some(instance_id) {
                        continue;
                    }
                }

                if let Some(collision) = state.instance_collision(instance_id) {
                    let intersection = collision.intersect(line_collision);

                    let res = gjk::gjk(
                        gjk::Settings {
                            tolerance: 0.1,
                            max_iterations: 12,
                            max_distance: 0.0,
                            find_closest_point: false,
                        },
                        intersection,
                        &mut Default::default(),
                    );

                    if matches!(res, gjk::Result::Touching) {
                        exec.stack().replace(ctx, ctx.fetch(&instance.this));
                        return Ok(());
                    }
                }
            }

            exec.stack().replace(ctx, object::no_one(ctx));
            Ok(())
        })?
    });
    magic
        .add_constant(ctx, ctx.intern_static("collision_line"), collision_line)
        .unwrap();

    magic
}
