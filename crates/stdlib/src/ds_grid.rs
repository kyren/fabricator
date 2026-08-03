use std::{convert::Infallible, sync::atomic};

use fabricator_vm as vm;
use gc_arena::{Collect, Gc, Lock, Mutation, Rootable, barrier};
use thiserror::Error;

use crate::util::MagicExt as _;

#[derive(Debug, Copy, Clone, Error)]
#[error("index [{x}, {y}] out of range of grid size {width}x{height}")]
pub struct OutOfGridRangeError {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct DsGrid<'gc> {
    array: Box<[Lock<vm::Value<'gc>>]>,
    width: usize,
    height: usize,
    numeric_id: i64,
}

impl<'gc> DsGrid<'gc> {
    pub fn new(width: usize, height: usize) -> Self {
        static NUMERIC_ID: atomic::AtomicI64 = atomic::AtomicI64::new(0);
        let numeric_id = NUMERIC_ID.fetch_add(1, atomic::Ordering::Relaxed);

        Self {
            array: vec![Lock::new(vm::Value::Undefined); width.checked_mul(height).unwrap()]
                .into_boxed_slice(),
            numeric_id,
            width,
            height,
        }
    }

    pub fn into_userdata(self, ctx: vm::Context<'gc>) -> vm::UserData<'gc> {
        struct DsGridMethods;

        impl<'gc> vm::UserDataMethods<'gc> for DsGridMethods {
            fn get_index(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                indexes: &[vm::Value<'gc>],
            ) -> Result<vm::Value<'gc>, vm::RuntimeError> {
                if indexes.len() != 2 {
                    return Err(vm::RuntimeError::msg("expected 2 indexes for ds_grid"));
                }

                let x: usize = vm::FromValue::from_value(ctx, indexes[0])?;
                let y: usize = vm::FromValue::from_value(ctx, indexes[1])?;

                Ok(DsGrid::downcast(ud).unwrap().get(x, y)?)
            }

            fn set_index(
                &self,
                ud: vm::UserData<'gc>,
                ctx: vm::Context<'gc>,
                indexes: &[vm::Value<'gc>],
                value: vm::Value<'gc>,
            ) -> Result<(), vm::RuntimeError> {
                if indexes.len() != 2 {
                    return Err(vm::RuntimeError::msg("expected 2 indexes for ds_grid"));
                }

                let x: usize = vm::FromValue::from_value(ctx, indexes[0])?;
                let y: usize = vm::FromValue::from_value(ctx, indexes[1])?;

                DsGrid::set(DsGrid::downcast_write(&ctx, ud).unwrap(), x, y, value)?;
                Ok(())
            }

            fn coerce_integer(&self, ud: vm::UserData<'gc>, _ctx: vm::Context<'gc>) -> Option<i64> {
                Some(DsGrid::downcast(ud).unwrap().numeric_id)
            }
        }

        #[derive(Collect)]
        #[collect(no_drop)]
        struct DsGridMethodsSingleton<'gc>(Gc<'gc, dyn vm::UserDataMethods<'gc>>);

        impl<'gc> vm::Singleton<'gc> for DsGridMethodsSingleton<'gc> {
            fn create(ctx: vm::Context<'gc>) -> Self {
                let methods = ctx.alloc_static(DsGridMethods);
                DsGridMethodsSingleton(gc_arena::unsize!(methods => dyn vm::UserDataMethods<'gc>))
            }
        }

        let methods = ctx.singleton::<Rootable![DsGridMethodsSingleton<'_>]>().0;
        let ud = vm::UserData::new::<Rootable![DsGrid<'_>]>(&ctx, self);
        ud.set_methods(&ctx, Some(methods));
        ud
    }

    #[inline]
    pub fn downcast(ud: vm::UserData<'gc>) -> Result<&'gc DsGrid<'gc>, vm::BadUserDataType> {
        ud.downcast::<Rootable![DsGrid<'_>]>()
    }

    #[inline]
    pub fn downcast_write(
        mc: &Mutation<'gc>,
        ud: vm::UserData<'gc>,
    ) -> Result<&'gc barrier::Write<DsGrid<'gc>>, vm::BadUserDataType> {
        ud.downcast_write::<Rootable![DsGrid<'_>]>(mc)
    }

    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    pub fn get(&self, x: usize, y: usize) -> Result<vm::Value<'gc>, OutOfGridRangeError> {
        if x < self.width && y < self.height {
            Ok(self.array[y * self.width + x].get())
        } else {
            Err(OutOfGridRangeError {
                x,
                y,
                width: self.width,
                height: self.height,
            })
        }
    }

    pub fn set(
        this: &barrier::Write<Self>,
        x: usize,
        y: usize,
        value: vm::Value<'gc>,
    ) -> Result<(), OutOfGridRangeError> {
        if x < this.width && y < this.height {
            let array = barrier::field!(this, DsGrid, array).as_deref();
            array[y * this.width + x].unlock().set(value);
            Ok(())
        } else {
            Err(OutOfGridRangeError {
                x,
                y,
                width: this.width,
                height: this.height,
            })
        }
    }
}

pub fn ds_grid_create<'gc>(
    ctx: vm::Context<'gc>,
    (width, height): (usize, usize),
) -> Result<vm::UserData<'gc>, Infallible> {
    Ok(DsGrid::new(width, height).into_userdata(ctx))
}

pub fn ds_grid_set_region<'gc>(
    ctx: vm::Context<'gc>,
    (grid, xmin, ymin, xmax, ymax, value): (
        vm::UserData<'gc>,
        usize,
        usize,
        usize,
        usize,
        vm::Value<'gc>,
    ),
) -> Result<(), vm::RuntimeError> {
    let grid = DsGrid::downcast_write(&ctx, grid)?;

    if xmin > xmax || ymin > ymax {
        return Err(vm::RuntimeError::msg(format!(
            "grid region [{xmin}, {ymin}, {xmax}, {ymax}] is invalid"
        )));
    }

    if xmin >= grid.width || xmax >= grid.width || ymin >= grid.height || ymax >= grid.height {
        return Err(vm::RuntimeError::msg(format!(
            "grid region [{xmin}, {ymin}, {xmax}, {ymax}] is out of range of size {}x{}",
            grid.width, grid.height
        )));
    }

    for x in xmin..=xmax {
        for y in ymin..=ymax {
            DsGrid::set(grid, x, y, value)?;
        }
    }
    Ok(())
}

pub fn ds_grid_clear<'gc>(
    ctx: vm::Context<'gc>,
    (grid, value): (vm::UserData<'gc>, vm::Value<'gc>),
) -> Result<(), vm::BadUserDataType> {
    let grid = DsGrid::downcast_write(&ctx, grid)?;
    let array = barrier::field!(grid, DsGrid, array).as_deref();
    for i in 0..array.len() {
        array[i].unlock().set(value);
    }
    Ok(())
}

pub fn ds_grid_width<'gc>(
    _ctx: vm::Context<'gc>,
    grid: vm::UserData<'gc>,
) -> Result<isize, vm::BadUserDataType> {
    Ok(DsGrid::downcast(grid)?.width as isize)
}

pub fn ds_grid_height<'gc>(
    _ctx: vm::Context<'gc>,
    grid: vm::UserData<'gc>,
) -> Result<isize, vm::BadUserDataType> {
    Ok(DsGrid::downcast(grid)?.height as isize)
}

/// Only clears the grid, as all objects in fabricator are automatically GCed.
pub fn ds_grid_destroy<'gc>(
    ctx: vm::Context<'gc>,
    grid: vm::UserData<'gc>,
) -> Result<(), vm::BadUserDataType> {
    ds_grid_clear(ctx, (grid, vm::Value::Undefined))
}

pub fn ds_grid_lib<'gc>(ctx: vm::Context<'gc>, lib: &mut vm::MagicSet<'gc>) {
    lib.insert_callback(ctx, "ds_grid_create", ds_grid_create);
    lib.insert_callback(ctx, "ds_grid_set_region", ds_grid_set_region);
    lib.insert_callback(ctx, "ds_grid_clear", ds_grid_clear);
    lib.insert_callback(ctx, "ds_grid_width", ds_grid_width);
    lib.insert_callback(ctx, "ds_grid_height", ds_grid_height);
    lib.insert_callback(ctx, "ds_grid_destroy", ds_grid_destroy);
}
