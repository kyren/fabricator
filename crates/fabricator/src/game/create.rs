use std::{
    collections::{HashMap, hash_map},
    f64,
    fs::{self, File},
    io::{self, Read as _},
    rc::Rc,
    time::Instant,
};

use anyhow::{Context as _, Error, anyhow, bail, ensure};
use fabricator_compiler as compiler;
use fabricator_math::{Box2, Vec2};
use fabricator_stdlib::StdlibContext as _;
use fabricator_util::typed_id_map::{IdMap, SecondaryMap};
use fabricator_vm as vm;
use gc_arena::Gc;
use image::GenericImageView as _;
use rayon::iter::{IntoParallelIterator as _, IntoParallelRefIterator, ParallelIterator as _};
use sha2::{Digest as _, Sha256};

use crate::{
    api::{
        asset::assets_api,
        collision::collision_api,
        drawing::{
            ShaderUserData, SpriteUserData, TexturePageUserData, TileSetUserData, drawing_api,
        },
        font::{FontUserData, font_api},
        instance::instance_api,
        layer::layers_api,
        magic::MagicExt as _,
        object::{ObjectUserData, object_api},
        os::os_api,
        platform::platform_api,
        room::{RoomUserData, room_api},
        sound::{SoundUserData, sound_api},
        stub::stub_api,
        tile::tiles_api,
    },
    ffi::load_extension_file,
    game::maxrects::MaxRects,
    project::{CollisionKind, LayerType, ObjectEvent, Project, ScriptMode},
    state::{
        AnimationFrame, Configuration, InstanceTemplate, InstanceTemplateId, Object, ObjectId,
        Room, RoomId, RoomLayer, Scripts, Sprite, SpriteCollision, SpriteCollisionKind, SpriteId,
        State, Texture, TextureId, TexturePage, TexturePageId,
        configuration::{
            Font, FontId, RoomLayerType, RoomTileLayer, Shader, ShaderId, Sound, SoundId, TileSet,
            TileSetId,
        },
    },
};

pub fn create_state(
    interpreter: &mut vm::Interpreter,
    project: &Project,
    config_name: &str,
) -> Result<State, Error> {
    // TODO: Hard coded tick rate, normally configured by 'options/main/options_main.yy'.
    const TICK_RATE: f64 = 60.0;

    let mut sprites = IdMap::<SpriteId, Rc<Sprite>>::new();
    let mut sprite_dict = HashMap::<String, SpriteId>::new();

    let mut textures = IdMap::<TextureId, Rc<Texture>>::new();
    let mut textures_for_hash = HashMap::new();

    let sprite_frame_meta = read_sprite_frame_meta(project)?;

    for (sprite_name, sprite) in &project.sprites {
        let size = Vec2::new(sprite.width, sprite.height);
        let sprite_frame_meta = &sprite_frame_meta[sprite_name.as_str()];

        let mut frame_dict = HashMap::new();
        for frame in sprite.frames.values() {
            let frame_meta = &sprite_frame_meta[frame.name.as_str()];
            ensure!(
                Vec2::new(frame_meta.image_width, frame_meta.image_height) == size,
                "frame size does not match sprite size for frame {:?}",
                frame.name
            );

            // Deduplicate frame textures, if we have already created a texture for an existing
            // frame with the same content hash, re-use its texture.
            match textures_for_hash.entry(frame_meta.unique_hash) {
                hash_map::Entry::Occupied(occupied) => {
                    frame_dict.insert(frame.name.clone(), *occupied.get());
                }
                hash_map::Entry::Vacant(vacant) => {
                    let texture_id = textures.insert(Rc::new(Texture {
                        texture_group: sprite.texture_group.clone(),
                        image_path: frame.image_path.clone(),
                        size,
                        cropped_size: Vec2::new(
                            frame_meta.cropped_image_width,
                            frame_meta.cropped_image_height,
                        ),
                        cropped_offset: Vec2::new(
                            frame_meta.cropped_image_xoffset,
                            frame_meta.cropped_image_yoffset,
                        ),
                    }));

                    vacant.insert(texture_id);
                    frame_dict.insert(frame.name.clone(), texture_id);
                }
            }
        }

        let mut frames = Vec::new();
        let mut start_time = 0.0;
        for animation_frame in &sprite.animation_frames {
            frames.push(AnimationFrame {
                texture: frame_dict
                    .get(&animation_frame.frame)
                    .copied()
                    .with_context(|| anyhow!("invalid frame named {:?}", animation_frame.frame))?,
                frame_start: start_time,
            });
            start_time += animation_frame.length;
        }

        let sprite_size = Vec2::new(sprite.width, sprite.height);
        let sprite_origin = Vec2::new(sprite.origin_x, sprite.origin_y);

        let collision_bounds = Box2::<f64>::new(
            Vec2::new(sprite.bbox_left, sprite.bbox_top).cast(),
            Vec2::new(sprite.bbox_right, sprite.bbox_bottom).cast(),
        )
        .translate(-sprite_origin.cast::<f64>());

        let (collision_kind, collision_rotates) = match sprite.collision_kind {
            CollisionKind::Rectangle => (SpriteCollisionKind::Rect, false),
            CollisionKind::RectangleWithRotation => (SpriteCollisionKind::Rect, true),
            CollisionKind::Ellipse => (SpriteCollisionKind::Ellipse, false),
            CollisionKind::Diamond => (SpriteCollisionKind::Diamond, false),
        };

        let sprite_id = sprites.insert_with_id(|id| {
            let userdata = interpreter
                .enter(|ctx| ctx.stash(SpriteUserData::new(ctx, id, ctx.intern(sprite_name))));
            Rc::new(Sprite {
                name: sprite_name.clone(),
                playback_speed: sprite.playback_speed,
                playback_length: sprite.playback_length,
                size: sprite_size,
                origin: sprite_origin,
                collision: SpriteCollision {
                    kind: collision_kind,
                    bounds: collision_bounds,
                },
                collision_rotates,
                frames,
                userdata,
            })
        });
        sprite_dict.insert(sprite_name.clone(), sprite_id);
    }

    let mut fonts = IdMap::<FontId, Rc<Font>>::new();
    for font in project.fonts.values() {
        fonts.insert_with_id(|id| {
            let userdata = interpreter
                .enter(|ctx| ctx.stash(FontUserData::new(ctx, id, ctx.intern(&font.name))));
            Rc::new(Font {
                name: font.name.clone(),
                userdata,
            })
        });
    }

    let mut sounds = IdMap::<SoundId, Rc<Sound>>::new();
    for sound in project.sounds.values() {
        sounds.insert_with_id(|id| {
            let userdata = interpreter
                .enter(|ctx| ctx.stash(SoundUserData::new(ctx, id, ctx.intern(&sound.name))));
            Rc::new(Sound {
                name: sound.name.clone(),
                duration: sound.duration,
                userdata,
            })
        });
    }

    let mut shaders = IdMap::<ShaderId, Rc<Shader>>::new();
    for shader in project.shaders.values() {
        shaders.insert_with_id(|id| {
            let userdata = interpreter
                .enter(|ctx| ctx.stash(ShaderUserData::new(ctx, id, ctx.intern(&shader.name))));
            Rc::new(Shader {
                name: shader.name.clone(),
                userdata,
            })
        });
    }

    let mut tile_sets = IdMap::<TileSetId, Rc<TileSet>>::new();
    let mut tile_set_dict = HashMap::<String, TileSetId>::new();

    for (tile_set_name, tile_set) in &project.tile_sets {
        let tile_set_id = tile_sets.insert_with_id(|id| {
            let userdata = interpreter
                .enter(|ctx| ctx.stash(TileSetUserData::new(ctx, id, ctx.intern(&tile_set.name))));
            Rc::new(TileSet {
                name: tile_set.name.clone(),
                tile_count: tile_set.tile_count,
                userdata,
            })
        });

        if tile_set_dict
            .insert(tile_set_name.clone(), tile_set_id)
            .is_some()
        {
            bail!("duplicate tile set named {tile_set_name:?}");
        };
    }

    let mut objects = IdMap::<ObjectId, Object>::new();
    let mut object_dict = HashMap::<String, ObjectId>::new();

    for (object_name, object) in &project.objects {
        let sprite = object
            .sprite
            .as_ref()
            .map(|sprite_name| {
                sprite_dict
                    .get(sprite_name)
                    .copied()
                    .with_context(|| anyhow!("missing sprite named {:?}", sprite_name))
            })
            .transpose()?;

        let object_id = objects.insert_with_id(|id| {
            let userdata = interpreter
                .enter(|ctx| ctx.stash(ObjectUserData::new(ctx, id, ctx.intern(&object_name))));
            Object {
                name: object_name.clone(),
                parent: None,
                sprite,
                persistent: object.persistent,
                userdata,
                tags: object.tags.clone(),
            }
        });
        if object_dict.insert(object_name.clone(), object_id).is_some() {
            bail!("duplicate object named {object_name:?}");
        };
    }

    for (object_name, object) in &project.objects {
        let object_id = object_dict[object_name];
        if let Some(parent_name) = &object.parent_object {
            let &parent_object_id = object_dict
                .get(parent_name)
                .with_context(|| anyhow!("no such parent object named {:?}", parent_name))?;
            objects[object_id].parent = Some(parent_object_id);
        }
    }

    let objects = objects.map_value(Rc::new);

    let mut rooms = IdMap::<RoomId, Rc<Room>>::new();
    let mut room_dict = HashMap::<String, RoomId>::new();

    let mut instance_templates = IdMap::<InstanceTemplateId, InstanceTemplate>::new();

    for room in project.rooms.values() {
        let mut layers = HashMap::new();

        for (layer_name, layer) in &room.layers {
            let layer_type = match &layer.layer_type {
                LayerType::Instances(instances) => {
                    let mut template_ids = Vec::new();

                    for instance in instances {
                        let template_id = instance_templates.insert(InstanceTemplate {
                            object: *object_dict.get(&instance.object).with_context(|| {
                                format!("missing object named {:?}", instance.object)
                            })?,
                            position: Vec2::new(instance.x, instance.y),
                        });
                        template_ids.push(template_id);
                    }

                    RoomLayerType::Instances(template_ids)
                }
                LayerType::Assets => RoomLayerType::Assets,
                LayerType::Tile(tile_layer) => {
                    let tile_set = match &tile_layer.tile_set {
                        Some(tile_set_name) => {
                            Some(*tile_set_dict.get(tile_set_name).with_context(|| {
                                format!("missing tile set named {tile_set_name:?}")
                            })?)
                        }
                        None => None,
                    };
                    let room_tile_layer = RoomTileLayer {
                        position: Vec2::new(tile_layer.x, tile_layer.y),
                        tile_set: tile_set,
                        grid_dimensions: Vec2::new(tile_layer.grid_width, tile_layer.grid_height),
                        grid: tile_layer.tile_grid.clone(),
                    };
                    RoomLayerType::Tile(room_tile_layer)
                }
                LayerType::Background => RoomLayerType::Background,
            };

            layers.insert(
                layer_name.clone(),
                RoomLayer {
                    name: layer.name.clone(),
                    depth: layer.depth,
                    visible: layer.visible,
                    layer_type,
                },
            );
        }

        let room_id = rooms.insert_with_id(|id| {
            let room_ud = interpreter
                .enter(|ctx| ctx.stash(RoomUserData::new(ctx, id, ctx.intern(&room.name))));
            Rc::new(Room {
                name: room.name.clone(),
                size: Vec2::new(room.width, room.height),
                layers,
                userdata: room_ud,
                tags: room.tags.clone(),
            })
        });

        room_dict.insert(room.name.clone(), room_id);
    }

    let first_room = project.room_order.first().context("no first room")?;
    let first_room = *room_dict
        .get(first_room)
        .with_context(|| "no such room `{first_room:?}`")?;
    let last_room = project.room_order.last().context("no last room")?;
    let last_room = *room_dict
        .get(last_room)
        .with_context(|| "no such room `{last_room:?}`")?;

    let texture_placements = textures
        .iter()
        .map(|(texture_id, texture)| TexturePlacement {
            texture_id,
            size: texture.cropped_size,
            group_name: &texture.texture_group,
        })
        .collect::<Vec<_>>();
    let texture_page_list = compute_texture_pages(project, texture_placements)?;

    let mut texture_pages = IdMap::<TexturePageId, Rc<TexturePage>>::new();

    for page_data in texture_page_list {
        texture_pages.insert_with_id(|id| {
            let userdata = interpreter.enter(|ctx| ctx.stash(TexturePageUserData::new(ctx, id)));
            Rc::new(TexturePage {
                size: page_data.size,
                border: page_data.border,
                group_name: page_data.group_name,
                group_number: page_data.group_number,
                textures: page_data.textures,
                userdata,
            })
        });
    }

    let mut texture_page_for_texture = SecondaryMap::new();
    for (texture_page_id, texture_page) in texture_pages.iter() {
        for texture_id in texture_page.textures.ids() {
            texture_page_for_texture.insert(texture_id, texture_page_id);
        }
    }

    let config = Configuration {
        data_path: project.base_path.join("datafiles"),
        tick_rate: TICK_RATE,
        sprites,
        textures,
        texture_pages,
        texture_page_for_texture,
        fonts,
        sounds,
        shaders,
        tile_sets,
        tile_set_dict,
        objects,
        object_dict,
        instance_templates,
        rooms,
        room_dict,
        first_room,
        last_room,
    };

    let scripts = load_scripts(project, &config, config_name, interpreter)?;

    Ok(State {
        start_instant: Instant::now(),
        config,
        scripts,
        current_room: None,
        next_room: Some(first_room),
        layers: Default::default(),
        named_layers: Default::default(),
        tile_maps: Default::default(),
        instances: Default::default(),
        instance_for_template: Default::default(),
        instances_for_object: Default::default(),
        instances_for_layer: Default::default(),
        instance_bound_tree: Default::default(),
    })
}

struct FrameMeta {
    pub unique_hash: u128,

    pub image_width: u32,
    pub image_height: u32,

    pub cropped_image_xoffset: u32,
    pub cropped_image_yoffset: u32,
    pub cropped_image_width: u32,
    pub cropped_image_height: u32,
}

type SpriteFrameMeta<'a> = HashMap<&'a str, FrameMeta>;

fn read_sprite_frame_meta(
    project: &Project,
) -> Result<HashMap<&'_ str, SpriteFrameMeta<'_>>, Error> {
    project
        .sprites
        .par_iter()
        .map(|(sprite_name, sprite)| {
            let texture_group = project
                .texture_groups
                .get(&sprite.texture_group)
                .with_context(|| anyhow!("invalid texture group {:?}", sprite.texture_group))?;

            let mut sprite_meta = SpriteFrameMeta::new();
            for (frame_name, frame) in &sprite.frames {
                let image_buf = fs::read(&frame.image_path)?;

                // NOTE: We are deduplicating frames by the hash of their *file contents*, which is
                // possibly too pessimistic.
                let unique_hash =
                    u128::from_le_bytes(Sha256::digest(&image_buf)[0..16].try_into().unwrap());

                let reader = io::Cursor::new(&image_buf);
                let image = if let Ok(fmt) = image::ImageFormat::from_path(&frame.image_path) {
                    image::ImageReader::with_format(reader, fmt)
                } else {
                    image::ImageReader::new(reader).with_guessed_format()?
                }
                .decode()?;

                let image_width = image.width();
                let image_height = image.height();

                let mut crop_left = 0;
                let mut crop_top = 0;
                let mut crop_right = image_width;
                let mut crop_bottom = image_height;

                if texture_group.auto_crop {
                    let is_not_translucent = |x: u32, y: u32| image.get_pixel(x, y).0[3] != 0;

                    while crop_left < image_width {
                        if (0..image_height).any(|y| is_not_translucent(crop_left, y)) {
                            break;
                        }

                        crop_left += 1;
                    }

                    while crop_top < image_height {
                        if (crop_left..image_width).any(|x| is_not_translucent(x, crop_top)) {
                            break;
                        }

                        crop_top += 1;
                    }

                    while crop_right > crop_left {
                        if (crop_top..image_height).any(|y| is_not_translucent(crop_right - 1, y)) {
                            break;
                        }

                        crop_right -= 1;
                    }

                    while crop_bottom > crop_top {
                        if (crop_left..crop_right).any(|x| is_not_translucent(x, crop_bottom - 1)) {
                            break;
                        }

                        crop_bottom -= 1;
                    }
                }

                sprite_meta.insert(
                    frame_name.as_str(),
                    FrameMeta {
                        unique_hash,
                        image_width,
                        image_height,
                        cropped_image_xoffset: crop_left,
                        cropped_image_yoffset: crop_top,
                        cropped_image_width: crop_right - crop_left,
                        cropped_image_height: crop_bottom - crop_top,
                    },
                );
            }

            Ok((sprite_name.as_str(), sprite_meta))
        })
        .collect()
}

fn load_scripts(
    project: &Project,
    config: &Configuration,
    config_name: &str,
    interpreter: &mut vm::Interpreter,
) -> Result<Scripts, Error> {
    let scripts = interpreter.enter(|ctx| -> Result<_, Error> {
        let mut object_events =
            HashMap::<ObjectId, HashMap<ObjectEvent, vm::StashedClosure>>::new();
        let mut magic = vm::MagicSet::new();

        magic.merge_unique(&ctx.stdlib())?;

        magic.merge_unique(&os_api(ctx))?;
        magic.merge_unique(&platform_api(ctx))?;
        magic.merge_unique(&collision_api(ctx))?;
        magic.merge_unique(&stub_api(ctx))?;
        magic.merge_unique(&object_api(ctx, &config)?)?;
        magic.merge_unique(&instance_api(ctx))?;
        magic.merge_unique(&room_api(ctx, &config)?)?;
        magic.merge_unique(&drawing_api(ctx, &config)?)?;
        magic.merge_unique(&font_api(ctx, &config)?)?;
        magic.merge_unique(&sound_api(ctx, &config)?)?;
        magic.merge_unique(&assets_api(ctx, &config)?)?;
        magic.merge_unique(&tiles_api(ctx))?;
        magic.merge_unique(&layers_api(ctx))?;

        for extension in project.extensions.values() {
            for file in &extension.files {
                if let Some(callbacks) = load_extension_file(ctx, file)? {
                    for (name, callback) in callbacks {
                        magic.add_constant(&ctx, name, callback)?;
                    }
                }
            }
        }

        let magic = Gc::new(&ctx, magic);

        log::info!("compiling all global scripts...");
        let mut script_compiler = compiler::Compiler::new(
            ctx,
            config_name,
            compiler::ImportItems::with_magic(&ctx, magic),
        );

        let mut scripts = project.scripts.values().collect::<Vec<_>>();

        // Compile scripts in a deterministic order (lexicographically sorted by name).
        scripts.sort_by_key(|s| &s.name);

        let mut code_buf = String::new();
        for script in scripts {
            code_buf.clear();
            File::open(&script.path)?.read_to_string(&mut code_buf)?;
            script_compiler.add_chunk(
                match script.mode {
                    ScriptMode::Compat => compiler::CompileSettings::compat(),
                    ScriptMode::Modern => compiler::CompileSettings::strict(),
                },
                script.path.to_string_lossy().into_owned(),
                &code_buf,
            )?;
        }

        let script_output = script_compiler.compile()?;
        log::info!("finished compiling all global scripts!");

        log::info!("compiling all object scripts...");
        for (object_name, proj_object) in &project.objects {
            for (&event, script) in &proj_object.event_scripts {
                code_buf.clear();
                File::open(&script.path)?.read_to_string(&mut code_buf)?;
                let name = script.path.to_string_lossy();
                let proto_output = compiler::Compiler::compile_chunk(
                    ctx,
                    config_name,
                    script_output.exported_imports,
                    match script.mode {
                        ScriptMode::Compat => compiler::CompileSettings::compat(),
                        ScriptMode::Modern => compiler::CompileSettings::strict(),
                    }
                    .export_top_level_functions(false),
                    name.into_owned(),
                    &code_buf,
                )?;
                let proto = proto_output.chunk_prototype;
                object_events
                    .entry(config.object_dict[object_name])
                    .or_default()
                    .insert(
                        event,
                        ctx.stash(vm::Closure::new(&ctx, proto, vm::Value::Undefined).unwrap()),
                    );
            }
        }
        log::info!("finished compiling all object scripts!");

        Ok(Scripts {
            scripts: script_output
                .chunks
                .into_iter()
                .map(|proto| {
                    ctx.stash(vm::Closure::new(&ctx, proto, vm::Value::Undefined).unwrap())
                })
                .collect(),
            object_events,
        })
    })?;

    Ok(scripts)
}

struct TexturePlacement<'a> {
    texture_id: TextureId,
    size: Vec2<u32>,
    group_name: &'a str,
}

struct TexturePageData {
    pub size: Vec2<u32>,
    pub border: u32,
    pub group_name: String,
    pub group_number: usize,
    pub textures: SecondaryMap<TextureId, Vec2<u32>>,
}

fn compute_texture_pages<'a>(
    project: &Project,
    textures: impl IntoIterator<Item = TexturePlacement<'a>>,
) -> Result<Vec<TexturePageData>, Error> {
    // TODO: Hard coded texture page size normally configured by
    // 'options/<platform>/options_<platform>.yy'.
    const TEXTURE_PAGE_SIZE: Vec2<u32> = Vec2::new(2048, 2048);

    // Treat images as being sized in blocks of `GRANULARITY` width and height. The larger the
    // number, the coarser and faster the image placement.
    const GRANULARITY: u32 = 8;
    assert!(TEXTURE_PAGE_SIZE[0] % GRANULARITY == 0 && TEXTURE_PAGE_SIZE[1] % GRANULARITY == 0);

    let mut texture_groups = HashMap::<&str, Vec<(TextureId, Vec2<u32>)>>::new();
    for tp in textures {
        texture_groups
            .entry(tp.group_name)
            .or_default()
            .push((tp.texture_id, tp.size));
    }

    log::info!("packing textures...");
    let texture_page_list = texture_groups
        .into_par_iter()
        .map(|(group_name, group)| {
            let border = project.texture_groups[group_name].border as u32;

            let mut to_place = group.into_iter().collect::<SecondaryMap<_, _>>();
            let mut texture_pages = Vec::new();

            while !to_place.is_empty() {
                let mut packer = MaxRects::new(TEXTURE_PAGE_SIZE / GRANULARITY);

                for (texture_id, &size) in to_place.iter() {
                    let padded_size = size + Vec2::splat(border * 2);
                    if padded_size[0] > TEXTURE_PAGE_SIZE[0]
                        || padded_size[1] > TEXTURE_PAGE_SIZE[1]
                    {
                        bail!(
                            "texture size {:?} is greater than the texture page size",
                            padded_size
                        );
                    }
                    packer.add(texture_id, padded_size.map(|v| v.div_ceil(GRANULARITY)));
                }

                let mut texture_page = TexturePageData {
                    size: TEXTURE_PAGE_SIZE,
                    border,
                    group_name: group_name.to_owned(),
                    group_number: texture_pages.len(),
                    textures: SecondaryMap::new(),
                };

                let prev_place_len = to_place.len();
                for packed in packer.pack() {
                    if let Some(mut position) = packed.placement {
                        position = position * GRANULARITY + Vec2::splat(border);
                        texture_page.textures.insert(packed.item, position);
                        to_place.remove(packed.item);
                    }
                }

                assert!(
                    to_place.len() < prev_place_len,
                    "should always add at least a single texture per iteration"
                );

                texture_pages.push(texture_page);
            }

            log::info!(
                "finished packing textures for group {group_name} with {} texture pages",
                texture_pages.len()
            );

            Ok(texture_pages)
        })
        .collect::<Result<Vec<Vec<_>>, Error>>()?;

    let texture_pages = texture_page_list.into_iter().flatten().collect();

    log::info!("finished packing all textures!");

    Ok(texture_pages)
}
