use std::{cell::RefCell, convert::Infallible, f64};

use fabricator_vm as vm;
use rand::{Rng as _, SeedableRng, rngs::SmallRng, seq::SliceRandom as _};

use crate::util::{MagicExt as _, resolve_array_range};

pub fn cos<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.cos())
}

pub fn sin<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.sin())
}

pub fn abs<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.abs())
}

pub fn sqrt<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.sqrt())
}

pub fn sqr<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg * arg)
}

pub fn power<'gc>(_ctx: vm::Context<'gc>, (arg, exp): (f64, f64)) -> Result<f64, Infallible> {
    Ok(arg.powf(exp))
}

pub fn round<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.round_ties_even())
}

pub fn floor<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.floor())
}

pub fn ceil<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(arg.ceil())
}

pub fn sign<'gc>(_ctx: vm::Context<'gc>, arg: f64) -> Result<f64, Infallible> {
    Ok(if arg > 0.0 {
        1.0
    } else if arg < 0.0 {
        -1.0
    } else {
        0.0
    })
}

pub fn min<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let mut min: f64 = exec.stack().from_index(ctx, 0)?;
    for i in 1..exec.stack().len() {
        min = min.min(exec.stack().from_index::<f64>(ctx, i)?);
    }
    exec.stack().replace(ctx, min);
    Ok(())
}

pub fn max<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), vm::RuntimeError> {
    let mut max: f64 = exec.stack().from_index(ctx, 0)?;
    for i in 1..exec.stack().len() {
        max = max.max(exec.stack().from_index::<f64>(ctx, i)?);
    }
    exec.stack().replace(ctx, max);
    Ok(())
}

pub fn clamp<'gc>(
    _ctx: vm::Context<'gc>,
    (val, min, max): (f64, f64, f64),
) -> Result<f64, vm::RuntimeError> {
    if min > max {
        return Err(vm::RuntimeError::msg(format!(
            "{} > {} in `clamp`",
            min, max
        )));
    }
    Ok(val.max(min).min(max))
}

pub struct Rng {
    rng: RefCell<SmallRng>,
}

impl Default for Rng {
    fn default() -> Self {
        Self {
            rng: RefCell::new(SmallRng::from_os_rng()),
        }
    }
}

pub type RngSingleton = gc_arena::Static<Rng>;

pub fn randomize<'gc>(ctx: vm::Context<'gc>, (): ()) -> Result<u32, Infallible> {
    // NOTE: GMS2 only generates a u32 and FoM relies on this.
    let seed: u32 = rand::rng().random();
    *ctx.singleton::<RngSingleton>().rng.borrow_mut() = SmallRng::seed_from_u64(seed as u64);
    Ok(seed)
}

pub fn random_set_seed<'gc>(ctx: vm::Context<'gc>, seed: u32) -> Result<(), Infallible> {
    *ctx.singleton::<RngSingleton>().rng.borrow_mut() = SmallRng::seed_from_u64(seed as u64);
    Ok(())
}

pub fn random<'gc>(ctx: vm::Context<'gc>, upper: f64) -> Result<f64, vm::RuntimeError> {
    if upper < 0.0 {
        return Err(vm::RuntimeError::msg(format!(
            "`random` upper range {upper} cannot be <= 0.0"
        )));
    }
    let mut rng = ctx.singleton::<RngSingleton>().rng.borrow_mut();
    Ok(rng.random_range(0.0..=upper))
}

pub fn irandom<'gc>(ctx: vm::Context<'gc>, upper: i64) -> Result<i64, vm::RuntimeError> {
    if upper < 0 {
        return Err(vm::RuntimeError::msg(format!(
            "`irandom` upper range {upper} cannot be <= 0"
        )));
    }
    let mut rng = ctx.singleton::<RngSingleton>().rng.borrow_mut();
    Ok(rng.random_range(0..=upper))
}

pub fn random_range<'gc>(
    ctx: vm::Context<'gc>,
    (lower, upper): (f64, f64),
) -> Result<f64, vm::RuntimeError> {
    if upper < lower {
        return Err(vm::RuntimeError::msg(format!(
            "`random_range`: invalid range [{lower}, {upper}]"
        )));
    }
    let mut rng = ctx.singleton::<RngSingleton>().rng.borrow_mut();
    Ok(rng.random_range(lower..=upper))
}

pub fn irandom_range<'gc>(
    ctx: vm::Context<'gc>,
    (lower, upper): (i64, i64),
) -> Result<i64, vm::RuntimeError> {
    if upper < lower {
        return Err(vm::RuntimeError::msg(format!(
            "`irandom_range`: invalid range [{lower}, {upper}]"
        )));
    }
    let mut rng = ctx.singleton::<RngSingleton>().rng.borrow_mut();
    Ok(rng.random_range(lower..=upper))
}

pub fn choose<'gc>(
    ctx: vm::Context<'gc>,
    mut exec: vm::Execution<'gc, '_>,
) -> Result<(), Infallible> {
    let mut stack = exec.stack();
    let mut rng = ctx.singleton::<RngSingleton>().rng.borrow_mut();
    let i = rng.random_range(0..stack.len());
    let v = stack[i];
    stack.replace(ctx, v);
    Ok(())
}

pub fn array_shuffle<'gc>(
    ctx: vm::Context<'gc>,
    (src, src_index, length): (vm::Array<'gc>, Option<isize>, Option<isize>),
) -> Result<vm::Array<'gc>, vm::RuntimeError> {
    let (src_range, is_reverse) = resolve_array_range(src.len(), src_index, length)?;

    let mut vals: Vec<_> = if is_reverse {
        src_range.rev().map(|i| src.get(i)).collect()
    } else {
        src_range.map(|i| src.get(i)).collect()
    };

    let mut rng = ctx.singleton::<RngSingleton>().rng.borrow_mut();
    vals.shuffle(&mut rng);

    Ok(vm::Array::from_iter(&ctx, vals))
}

pub fn point_in_rectangle<'gc>(
    _ctx: vm::Context<'gc>,
    (px, py, xmin, ymin, xmax, ymax): (f64, f64, f64, f64, f64, f64),
) -> Result<bool, Infallible> {
    Ok(px >= xmin && px <= xmax && py >= ymin && py <= ymax)
}

/// Moves a given value from start to stop by a certain percent.
pub fn lerp<'gc>(
    _ctx: vm::Context<'gc>,
    (start, stop, percent): (f64, f64, f64),
) -> Result<f64, Infallible> {
    Ok(start + (stop - start) * percent)
}

/// Extracts the fractional part of a floating point number.
pub fn frac<'gc>(_ctx: vm::Context<'gc>, input: f64) -> Result<f64, Infallible> {
    Ok(input.fract())
}

/// Returns the smallest difference between two angles, which will be between (-180.0..180.0).
/// The result of this function, when added to `src` will give `dst`.
pub fn angle_difference<'gc>(
    _ctx: vm::Context<'gc>,
    (dst, src): (f64, f64),
) -> Result<f64, Infallible> {
    let diff = (dst - src) % 360.0;
    let output = if diff > 180.0 {
        diff - 360.0
    } else if diff < -180.0 {
        diff + 360.0
    } else {
        diff
    };

    Ok(output)
}

/// Computers [`f64::atan2`] for the difference between two points, and then converts
/// the result into degrees.
///
/// This gives the direction from point1 to point2.
///
/// Note: this assumes y is down.
pub fn point_direction<'gc>(
    _ctx: vm::Context<'gc>,
    (x1, y1, x2, y2): (f64, f64, f64, f64),
) -> Result<f64, Infallible> {
    let dx = x2 - x1;
    let dy = y1 - y2;

    let mut angle_deg = dy.atan2(dx).to_degrees();

    // we're wrapping since the bottom angles are given as negatives in atan2
    if angle_deg < 0.0 {
        angle_deg += 360.0;
    }

    Ok(angle_deg)
}

/// Returns the length from tail to tip.
pub fn point_distance<'gc>(
    _ctx: vm::Context<'gc>,
    (tail_x, tail_y, tip_x, tip_y): (f64, f64, f64, f64),
) -> Result<f64, Infallible> {
    let x = tip_x - tail_x;
    let y = tip_y - tail_y;

    let dot = (x * x) + (y * y);
    Ok(f64::sqrt(dot))
}

/// Returns the horizontal component of a vector with given length and direction.
/// `direction` is given in degrees.
pub fn lengthdir_x<'gc>(
    _ctx: vm::Context<'gc>,
    (length, direction): (f64, f64),
) -> Result<f64, Infallible> {
    let angle_rad = direction.to_radians();
    Ok(length * angle_rad.cos())
}

/// Returns the vertical component of a vector with given length and direction.
/// `direction` is given in degrees.
///
/// Note: y is down.
pub fn lengthdir_y<'gc>(
    _ctx: vm::Context<'gc>,
    (length, direction): (f64, f64),
) -> Result<f64, Infallible> {
    let angle_rad = direction.to_radians();
    Ok(-length * angle_rad.sin())
}

/// The equivalent to running [`degtorad`] and then [`arctan2`] on the resulting output. In fact,
/// this is what it does internally.
pub fn darctan2<'gc>(ctx: vm::Context<'gc>, (y, x): (f64, f64)) -> Result<f64, Infallible> {
    let Ok(angle) = arctan2(ctx, (y, x));
    degtorad(ctx, angle)
}

/// Computes the arc tangent. See [`f64::atan2`] for more information.
///
/// Returns results in radians. See [`darctan2`] for one which returns in degrees.
pub fn arctan2<'gc>(_ctx: vm::Context<'gc>, (y, x): (f64, f64)) -> Result<f64, Infallible> {
    Ok(y.atan2(x))
}

/// Converts degrees to radians.
pub fn degtorad<'gc>(_ctx: vm::Context<'gc>, input: f64) -> Result<f64, Infallible> {
    Ok(input.to_radians())
}

/// Converts degrees to radians.
pub fn radtodeg<'gc>(_ctx: vm::Context<'gc>, input: f64) -> Result<f64, Infallible> {
    Ok(input.to_degrees())
}

pub fn math_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_constant(ctx, "NaN", f64::NAN);
    lib.insert_constant(ctx, "infinity", f64::INFINITY);
    lib.insert_constant(ctx, "pi", f64::consts::PI);
    lib.insert_callback(ctx, "cos", cos);
    lib.insert_callback(ctx, "sin", sin);
    lib.insert_callback(ctx, "abs", abs);
    lib.insert_callback(ctx, "sqrt", sqrt);
    lib.insert_callback(ctx, "sqr", sqr);
    lib.insert_callback(ctx, "power", power);
    lib.insert_callback(ctx, "round", round);
    lib.insert_callback(ctx, "floor", floor);
    lib.insert_callback(ctx, "ceil", ceil);
    lib.insert_callback(ctx, "sign", sign);
    lib.insert_exec_callback(ctx, "min", min);
    lib.insert_exec_callback(ctx, "max", max);
    lib.insert_callback(ctx, "clamp", clamp);
    lib.insert_callback(ctx, "randomize", randomize);
    lib.insert_callback(ctx, "random_set_seed", random_set_seed);
    lib.insert_callback(ctx, "random", random);
    lib.insert_callback(ctx, "irandom", irandom);
    lib.insert_callback(ctx, "random_range", random_range);
    lib.insert_callback(ctx, "irandom_range", irandom_range);
    lib.insert_exec_callback(ctx, "choose", choose);
    lib.insert_callback(ctx, "array_shuffle", array_shuffle);
    lib.insert_callback(ctx, "point_in_rectangle", point_in_rectangle);
    lib.insert_callback(ctx, "lerp", lerp);
    lib.insert_callback(ctx, "frac", frac);
    lib.insert_callback(ctx, "angle_difference", angle_difference);
    lib.insert_callback(ctx, "point_direction", point_direction);
    lib.insert_callback(ctx, "point_distance", point_distance);
    lib.insert_callback(ctx, "lengthdir_x", lengthdir_x);
    lib.insert_callback(ctx, "lengthdir_y", lengthdir_y);
    lib.insert_callback(ctx, "darctan2", darctan2);
    lib.insert_callback(ctx, "arctan2", arctan2);
    lib.insert_callback(ctx, "degtorad", degtorad);
    lib.insert_callback(ctx, "radtodeg", radtodeg);
}
