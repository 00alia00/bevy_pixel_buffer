//! Adds a way to easily attach a compute shader to update the pixel buffer
//! every frame.
//!
//! This allows for fast buffer updates with functions that are
//! relatively expensive to perform, as it is done on the GPU.
use std::{borrow::Cow, marker::PhantomData};

use bevy::{
    asset::Asset,
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::*,
        renderer::RenderDevice,
        texture::{FallbackImage, GpuImage},
        Extract, Render, RenderApp, RenderSet,
    },
    utils::{HashMap, HashSet},
};

use crate::pixel_buffer::PixelBuffer;

#[allow(unused)] // doc link
use crate::pixel_buffer::Fill;

/// Implemented by a type that represents a compute shader instance.
///
/// # Example
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy::reflect::{TypePath};
/// # use bevy_pixel_buffer::compute_shader::ComputeShader;
/// # use bevy::render::render_resource::{ShaderRef, AsBindGroup};
///
/// #[derive(Asset, AsBindGroup, TypePath, Clone, Debug, Default)]
/// #[type_path = "example::my_shader"] // Make sure this is unique
/// struct MyShader {}
///
/// impl ComputeShader for MyShader {
///     fn shader() -> ShaderRef {
///         "my_shader.wgsl".into() // loaded from the bevy assets directory
///     }
///
///     fn entry_point() -> std::borrow::Cow<'static, str> {
///         "update".into()
///     }
///
///     fn workgroups(texture_size: UVec2) -> UVec2 {
///         texture_size / 8
///     }
/// }
/// ```
///
/// # About the number of workgrups
/// The reason behind [ComputeShader::workgroups] has a size parameter is because it can
/// change by a [Fill] configuration or because it is changed in some user defined bevy system.
///
/// The number of workgroups combined with the workgroup size (defined in the shader) need to match
/// together to process all the of the texture (the pixel buffer). For example, it we want **one invocation**
/// of the shader per pixel, and we have a `512 * 512` buffer, and the workgroup size is `8` by `8`;
/// we need `512 / 8 = 64` workgroups in each dimension to process the entire buffer.
///
/// Notice that with a [Fill] configuration that updates the size automatically, this can be a problem if the size
/// is not a multiple of a desired number, in our example, `8`. In this example, we would have to use
/// [Fill::with_scaling_multiple] to ensure that the size is a multiple of our workgroup size.
///
/// # About the bindings in the shader
/// The bind group 0 is set up with the texture in binding 0. The bind group 1 is the user bind group. The user bind
/// groups is provided by the implementation of the [AsBindGroup] trait, probably derivind it.
pub trait ComputeShader:
    Asset + AsBindGroup + Send + Sync + Clone + Asset + Default + Sized + 'static
{
    /// Shader code to load. Returning [ShaderRef::Default] would result in a panic.
    fn shader() -> ShaderRef;
    /// Entry point of the shader.
    fn entry_point() -> Cow<'static, str>;
    /// Number of workgroups.
    fn workgroups(texture_size: UVec2) -> UVec2;
}

/// Plugin added to register a shader
///
/// # Panics (when added)
/// - If the [ComputeShader::shader] returns a [ShaderRef::Default], as there is no
/// default compute shader.
///
/// - If the bevy render graph cannot be extended with a new node for some reason.
pub struct ComputeShaderPlugin<S: ComputeShader>(PhantomData<S>);

impl<S: ComputeShader> Default for ComputeShaderPlugin<S> {
    fn default() -> Self {
        Self(Default::default())
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct UserCs;

impl<S: ComputeShader> Plugin for ComputeShaderPlugin<S> {
    fn build(&self, app: &mut App) {
        app.init_asset::<S>();

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<ExtractedShaders<S>>()
                .init_resource::<PreparedShaders<S>>()
                .init_resource::<PreparedImages<S>>()
                .add_systems(ExtractSchedule, cs_extract::<S>)
                .add_systems(
                    Render,
                    (prepare_images::<S>, prepare_shaders::<S>).in_set(RenderSet::Prepare),
                )
                .add_systems(Render, cs_queue_bind_group::<S>.in_set(RenderSet::Queue));
            let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
            render_graph.add_node(UserCs, ComputeShaderNode::<S>::default());
            render_graph.add_node_edge(UserCs, bevy::render::graph::CameraDriverLabel);
        } else {
            warn!("Can't build ComputeShaderPlugin: RenderApp sub app not found.")
        }
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<ComputeShaderPipeline<S>>();
        }
    }
}

#[derive(Resource)]
struct ComputeShaderPipeline<S: ComputeShader> {
    pipeline_id: CachedComputePipelineId,
    texture_bind_group_layout: BindGroupLayout,
    user_bind_group_layout: BindGroupLayout,
    marker: PhantomData<S>,
}

impl<S: ComputeShader> FromWorld for ComputeShaderPipeline<S> {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let asset_server = world.resource::<AssetServer>();

        let shader = match S::shader() {
            ShaderRef::Default => panic!("Default compute shader does not exist."),
            ShaderRef::Handle(h) => h,
            ShaderRef::Path(p) => asset_server.load(p),
        };
        let entry_point = S::entry_point();

        let texture_bind_group_layout = device.create_bind_group_layout(
            None,
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::StorageTexture {
                    access: StorageTextureAccess::ReadWrite,
                    format: TextureFormat::Rgba8Unorm,
                    view_dimension: TextureViewDimension::D2,
                },
                count: None,
            }],
        );

        let user_bind_group_layout = S::bind_group_layout(device);

        let layout = vec![
            texture_bind_group_layout.clone(),
            user_bind_group_layout.clone(),
        ];

        let pipeline_cache = world.resource_mut::<PipelineCache>();
        let pipeline_id = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: None,
            layout,
            shader,
            shader_defs: vec![],
            entry_point,
            push_constant_ranges: vec![],
            zero_initialize_workgroup_memory: true,
        });

        ComputeShaderPipeline {
            pipeline_id,
            texture_bind_group_layout,
            user_bind_group_layout,
            marker: Default::default(),
        }
    }
}

#[derive(Resource)]
struct InvalidatedImages<S: ComputeShader> {
    invalid: HashSet<AssetId<Image>>,
    marker: PhantomData<S>,
}

impl<S: ComputeShader> Default for InvalidatedImages<S> {
    fn default() -> Self {
        Self {
            invalid: Default::default(),
            marker: Default::default(),
        }
    }
}

#[derive(Resource)]
struct ExtractedShaders<S: ComputeShader> {
    extracted: Vec<(AssetId<S>, S)>,
    removed: Vec<AssetId<S>>,
}

impl<S: ComputeShader> Default for ExtractedShaders<S> {
    fn default() -> Self {
        Self {
            extracted: Default::default(),
            removed: Default::default(),
        }
    }
}

#[allow(clippy::type_complexity)]
fn cs_extract<S: ComputeShader>(
    mut commands: Commands,
    mut previous_len: Local<usize>,
    buffers: Extract<Query<(Entity, &Sprite, &Handle<S>), With<PixelBuffer>>>,
    mut shader_events: Extract<EventReader<AssetEvent<S>>>,
    shader_assets: Extract<Res<Assets<S>>>,
    mut image_events: Extract<EventReader<AssetEvent<Image>>>,
) {
    let mut buffer_images = HashSet::with_capacity(*previous_len);

    // Extract the entities to apply shaders
    let mut values = Vec::with_capacity(*previous_len);
    for (entity, image_handle, shader_handle) in buffers.iter() {
        values.push((
            entity,
            (image_handle.clone_weak(), shader_handle.clone_weak()),
        ));
        buffer_images.insert(image_handle.id());
    }
    *previous_len = values.len();
    commands.insert_or_spawn_batch(values);

    // Update the shader cache
    let mut changed = HashSet::default();
    let mut removed = Vec::new();
    for event in shader_events.read() {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => {
                changed.insert(*id);
            }
            AssetEvent::Removed { id } | AssetEvent::Unused { id } => {
                changed.remove(id);
                removed.push(*id);
            }
        }
    }

    let mut extracted = Vec::new();
    for id in changed.drain() {
        if let Some(asset) = shader_assets.get(id) {
            extracted.push((id, asset.clone()));
        }
    }

    commands.insert_resource(ExtractedShaders { extracted, removed });

    // Update image bind group cache
    let mut invalid = HashSet::default();
    for event in image_events.read() {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::LoadedWithDependencies { id }
                if buffer_images.contains(id) =>
            {
                invalid.insert(*id);
            }
            _ => {}
        }
    }

    commands.insert_resource(InvalidatedImages {
        invalid,
        marker: PhantomData::<S>,
    });
}

struct PreparedImage<S> {
    texture_bind_group: BindGroup,
    marker: PhantomData<S>,
    size: UVec2,
}

#[derive(Resource, Default, Deref, DerefMut)]
struct PreparedImages<S>(HashMap<AssetId<Image>, PreparedImage<S>>);

fn prepare_images<S: ComputeShader>(
    mut previous_len: Local<usize>,
    buffers: Query<&Sprite, With<Handle<S>>>,
    render_device: Res<RenderDevice>,
    pipeline: Res<ComputeShaderPipeline<S>>,
    images: Res<RenderAssets<GpuImage>>,
    invalid_images: Res<InvalidatedImages<S>>,
    mut prepared_images: ResMut<PreparedImages<S>>,
) {
    // remove invalid prepared images
    prepared_images.retain(|id, _| !invalid_images.invalid.contains(id));
    let mut buffer_images = HashSet::with_capacity(*previous_len);
    // iterate over all the buffers
    for image_handle in buffers.iter() {
        let image_handle_id = image_handle.id();

        buffer_images.insert(image_handle_id);

        // if the image is not prepared, do it
        if !prepared_images.contains_key(&image_handle_id) {
            if let Some(view) = images.get(image_handle_id) {
                let texture_bind_group = render_device.create_bind_group(
                    None,
                    &pipeline.texture_bind_group_layout,
                    &[BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&view.texture_view),
                    }],
                );

                prepared_images.insert(
                    image_handle_id,
                    PreparedImage {
                        texture_bind_group,
                        size: view.size,
                        marker: PhantomData::<S>,
                    },
                );
            }
        }
    }
    *previous_len = buffer_images.len();

    // remove untracked images
    if prepared_images.len() != buffer_images.len() {
        prepared_images.retain(|h, _| buffer_images.contains(h));
    }
}

struct PreparedShader<S> {
    user_bind_group: BindGroup,
    marker: PhantomData<S>,
}

#[derive(Resource, Default, Deref, DerefMut)]
struct PreparedShaders<S: Asset + Default>(HashMap<AssetId<S>, PreparedShader<S>>);

struct PrepareNextFrameShaders<S: ComputeShader> {
    assets: Vec<(AssetId<S>, S)>,
}

impl<S: ComputeShader> Default for PrepareNextFrameShaders<S> {
    fn default() -> Self {
        Self {
            assets: Default::default(),
        }
    }
}

fn prepare_shaders<S: ComputeShader>(
    mut prepare_next_frame: Local<PrepareNextFrameShaders<S>>,
    mut extracted_assets: ResMut<ExtractedShaders<S>>,
    mut render_materials: ResMut<PreparedShaders<S>>,
    render_device: Res<RenderDevice>,
    images: Res<RenderAssets<GpuImage>>,
    fallback_image: Res<FallbackImage>,
    pipeline: Res<ComputeShaderPipeline<S>>,
) {
    let mut queued_assets = std::mem::take(&mut prepare_next_frame.assets);
    for (handle_id, shader) in queued_assets.drain(..) {
        match prepare_shader(&shader, &render_device, &images, &fallback_image, &pipeline) {
            Ok(prepared_asset) => {
                render_materials.insert(handle_id, prepared_asset);
            }
            Err(AsBindGroupError::InvalidSamplerType(_, _, _)) => {
                error!("Invalid sampler type encountered while preparing shader.");
            }
            Err(AsBindGroupError::RetryNextUpdate) => {
                prepare_next_frame.assets.push((handle_id, shader));
            }
        }
    }

    for removed in std::mem::take(&mut extracted_assets.removed) {
        render_materials.remove(&removed);
    }

    for (handle_id, shader) in std::mem::take(&mut extracted_assets.extracted) {
        match prepare_shader(&shader, &render_device, &images, &fallback_image, &pipeline) {
            Ok(prepared_asset) => {
                render_materials.insert(handle_id, prepared_asset);
            }
            Err(AsBindGroupError::InvalidSamplerType(_, _, _)) => {
                error!("Invalid sampler type encountered while preparing shader.");
            }
            Err(AsBindGroupError::RetryNextUpdate) => {
                prepare_next_frame.assets.push((handle_id, shader));
            }
        }
    }
}

fn prepare_shader<S: ComputeShader>(
    shaader: &S,
    render_device: &RenderDevice,
    images: &RenderAssets<GpuImage>,
    fallback_image: &FallbackImage,
    pipeline: &ComputeShaderPipeline<S>,
) -> Result<PreparedShader<S>, AsBindGroupError> {
    let prepared = shaader.as_bind_group(
        &pipeline.user_bind_group_layout,
        render_device,
        images,
        // fallback_image,
    )?;
    Ok(PreparedShader {
        user_bind_group: prepared.bind_group,
        marker: PhantomData,
    })
}

#[derive(Resource)]
struct ComputeShaderQueue<S: ComputeShader>(Vec<ComputeShaderInfo>, PhantomData<S>);
struct ComputeShaderInfo {
    texture_bind_group: BindGroup,
    user_bind_group: BindGroup,
    workgroups: UVec2,
}

fn cs_queue_bind_group<S: ComputeShader>(
    mut commands: Commands,
    buffers: Query<(&Sprite, &Handle<S>)>,
    prepared_shaders: Res<PreparedShaders<S>>,
    prepared_images: Res<PreparedImages<S>>,
    mut previous_len: Local<usize>,
) {
    let mut shaders = Vec::with_capacity(*previous_len);
    for (image_handle, shader_handle) in buffers.iter() {
        if let (Some(prepared_image), Some(prepared_shader)) = (
            prepared_images.get(&image_handle.id()),
            prepared_shaders.get(&shader_handle.id()),
        ) {
            shaders.push(ComputeShaderInfo {
                texture_bind_group: prepared_image.texture_bind_group.clone(),
                user_bind_group: prepared_shader.user_bind_group.clone(),
                workgroups: S::workgroups(prepared_image.size),
            });
        }
    }
    *previous_len = shaders.len();
    commands.insert_resource(ComputeShaderQueue::<S>(shaders, Default::default()));
}

struct ComputeShaderNode<S: ComputeShader> {
    state: State,
    marker: PhantomData<S>,
}

enum State {
    Loading,
    Update,
}

impl<S: ComputeShader> Default for ComputeShaderNode<S> {
    fn default() -> Self {
        Self {
            state: State::Loading,
            marker: Default::default(),
        }
    }
}

impl<S: ComputeShader> render_graph::Node for ComputeShaderNode<S> {
    fn update(&mut self, world: &mut World) {
        let pipeline = world.resource::<ComputeShaderPipeline<S>>();
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            State::Loading => {
                if let CachedPipelineState::Ok(_) =
                    pipeline_cache.get_compute_pipeline_state(pipeline.pipeline_id)
                {
                    self.state = State::Update;
                }
            }
            State::Update => {}
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        if matches!(self.state, State::Loading) {
            return Ok(());
        }

        let mut pass = render_context
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor::default());

        let shader_queue = world.resource::<ComputeShaderQueue<S>>();

        for shader in shader_queue.0.iter() {
            // index 0 is texture
            pass.set_bind_group(0, &shader.texture_bind_group, &[]);
            // index 1 is user bind group
            pass.set_bind_group(1, &shader.user_bind_group, &[]);
            let pipeline = world.resource::<ComputeShaderPipeline<S>>();
            let pipeline_cache = world.resource::<PipelineCache>();

            if let Some(update_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline_id)
            {
                pass.set_pipeline(update_pipeline);
                pass.dispatch_workgroups(shader.workgroups.x, shader.workgroups.y, 1);
            } else {
                error!("Could not retrieve compute shader pipeline from pipeline cache even after checking the state is not Loading.")
            }
        }

        Ok(())
    }
}
