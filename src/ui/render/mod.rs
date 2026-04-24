use crate::{
    dll::win32::{GdiObject, GpuBitmapUpdate, GpuDrawCommand, GpuSurfaceOp, Win32Context},
    ui::Painter,
};
use std::{collections::HashMap, sync::Arc};
use wgpu::{
    Adapter, BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, BlendState,
    BufferUsages, Color, ColorTargetState, ColorWrites, CommandEncoder, CommandEncoderDescriptor,
    CompositeAlphaMode, CurrentSurfaceTexture, Device, DeviceDescriptor, Extent3d, Features,
    FilterMode, FragmentState, Instance, InstanceDescriptor, LoadOp, MemoryHints, MipmapFilterMode,
    MultisampleState, Operations, PipelineCompilationOptions, PipelineLayout,
    PipelineLayoutDescriptor, PowerPreference, PresentMode, PrimitiveState, PrimitiveTopology,
    Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, RequestAdapterOptions, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModule, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp,
    Surface, SurfaceConfiguration, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    Trace, VertexAttribute, VertexBufferLayout, VertexFormat, VertexState, VertexStepMode,
    util::DeviceExt,
};
use winit::window::Window;

const UI_SHADER: &str = r#"
@group(0) @binding(0)
var frame_tex: texture_2d<f32>;
@group(0) @binding(1)
var frame_sampler: sampler;
@group(0) @binding(2)
var mask_tex: texture_2d<f32>;
@group(0) @binding(3)
var mask_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );

    var out: VertexOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = floor(position.xy);
    let frame_size = vec2<f32>(textureDimensions(frame_tex));
    if (pixel.x < 0.0 || pixel.y < 0.0 || pixel.x >= frame_size.x || pixel.y >= frame_size.y) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let frame_uv = (pixel + vec2<f32>(0.5, 0.5)) / frame_size;
    let mask_size = vec2<f32>(textureDimensions(mask_tex));
    let mask_uv = (pixel + vec2<f32>(0.5, 0.5)) / mask_size;

    let color = textureSampleLevel(frame_tex, frame_sampler, frame_uv, 0.0);
    let mask = textureSampleLevel(mask_tex, mask_sampler, mask_uv, 0.0).r;
    return vec4<f32>(color.rgb, color.a * mask);
}
"#;

const GEOMETRY_SHADER: &str = r#"
struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(input.position, 0.0, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return input.color;
}
"#;

const TEXT_SHADER: &str = r#"
@group(0) @binding(0)
var alpha_tex: texture_2d<f32>;
@group(0) @binding(1)
var alpha_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(input.position, 0.0, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let alpha = textureSampleLevel(alpha_tex, alpha_sampler, input.uv, 0.0).r;
    return vec4<f32>(input.color.rgb, input.color.a * alpha);
}
"#;

const BLIT_SHADER: &str = r#"
@group(0) @binding(0)
var src_tex: texture_2d<f32>;
@group(0) @binding(1)
var src_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(input.position, 0.0, 1.0);
    out.uv = input.uv;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(src_tex, src_sampler, input.uv, 0.0);
}
"#;

const TEXT_ATLAS_SIZE: u32 = 1024;
const TEXT_ATLAS_PADDING: u32 = 1;

/// UI 전체가 공유하는 `wgpu` 장치와 공통 파이프라인 자원입니다.
pub(crate) struct UiGpuContext {
    /// `wgpu` 인스턴스입니다.
    pub(crate) instance: Instance,
    /// UI 렌더링에 사용할 어댑터입니다.
    pub(crate) adapter: Adapter,
    /// UI 렌더링용 디바이스입니다.
    pub(crate) device: Device,
    /// UI 렌더링용 제출 큐입니다.
    pub(crate) queue: Queue,
    bind_group_layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    shader: ShaderModule,
    geometry_shader: ShaderModule,
    text_bind_group_layout: BindGroupLayout,
    text_pipeline_layout: PipelineLayout,
    text_shader: ShaderModule,
    blit_bind_group_layout: BindGroupLayout,
    blit_pipeline_layout: PipelineLayout,
    blit_shader: ShaderModule,
    sampler: Sampler,
}

/// 창 하나의 렌더 소스를 나타냅니다.
pub(crate) enum WindowRenderContent {
    /// guest surface bitmap을 직접 업로드하는 경로입니다.
    GuestBitmap { hwnd: u32, surface_bitmap: u32 },
    /// 기존 CPU `Painter`를 유지하는 경로입니다.
    CpuPainter(Box<dyn Painter>),
}

/// 창 하나의 `wgpu` 프레젠테이션 상태를 보관합니다.
pub(crate) struct WindowRenderTarget {
    window: Arc<Window>,
    surface: Surface<'static>,
    config: SurfaceConfiguration,
    pipeline: RenderPipeline,
    fill_pipeline: RenderPipeline,
    text_pipeline: RenderPipeline,
    blit_pipeline: RenderPipeline,
    content_format: TextureFormat,
    frame_texture: Texture,
    frame_view: TextureView,
    frame_size: (u32, u32),
    needs_full_frame_upload: bool,
    mask_texture: Texture,
    mask_view: TextureView,
    mask_size: (u32, u32),
    text_atlas: TextAtlas,
    bind_group: BindGroup,
    last_mask: Option<MaskCacheKey>,
    /// CPU painter와 RGBA 변환 경로에서 재사용하는 스크래치 버퍼입니다.
    pub(crate) scratch_pixels: Vec<u32>,
    scratch_bytes: Vec<u8>,
    geometry_bytes: Vec<u8>,
    geometry_vertex_count: u32,
    /// 현재 창의 콘텐츠 종류입니다.
    pub(crate) content: WindowRenderContent,
}

#[derive(Clone, PartialEq, Eq)]
struct MaskCacheKey {
    width: u32,
    height: u32,
    rects: Vec<(i32, i32, i32, i32)>,
}

struct GuestBitmapSnapshot {
    width: u32,
    height: u32,
    pixels: Arc<std::sync::Mutex<Vec<u32>>>,
    rects: Vec<(i32, i32, i32, i32)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextAtlasKey {
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextAtlasSlot {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct TextAtlas {
    texture: Texture,
    bind_group: BindGroup,
    width: u32,
    height: u32,
    next_x: u32,
    next_y: u32,
    row_height: u32,
    cache: HashMap<TextAtlasKey, TextAtlasSlot>,
}

/// `RedrawRequested` 처리 결과입니다.
pub(crate) enum RenderOutcome {
    /// 정상적으로 프레임을 제출했습니다.
    Rendered,
    /// 이번 프레임은 건너뛰었습니다.
    Skipped,
}

/// 렌더링 중 surface 획득 단계에서 발생한 오류를 감쌉니다.
pub(crate) enum RenderFrameError {
    /// `wgpu::Surface` 텍스처 획득 상태가 재구성/재시도/종료를 요구합니다.
    Surface(SurfaceAcquireError),
}

/// surface 텍스처 획득 중 만난 상태를 단순화한 오류입니다.
pub(crate) enum SurfaceAcquireError {
    /// surface를 다시 configure 해야 합니다.
    Outdated,
    /// surface를 다시 만들어야 합니다.
    Lost,
    /// 이번 프레임은 건너뛰어야 합니다.
    Timeout,
    /// 가려진 창이므로 이번 프레임은 건너뜁니다.
    Occluded,
    /// 검증 오류가 발생했습니다.
    Validation,
}

impl UiGpuContext {
    /// 공통 `wgpu` 디바이스와 파이프라인 자원을 초기화합니다.
    pub(crate) fn new() -> Result<Self, String> {
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| format!("tokio runtime init failed: {err}"))?;

        let adapter = runtime
            .block_on(instance.request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .map_err(|err| format!("wgpu adapter request failed: {err}"))?;

        let (device, queue) = runtime
            .block_on(adapter.request_device(&DeviceDescriptor {
                label: Some("ui-device"),
                required_features: Features::empty(),
                required_limits: adapter.limits(),
                experimental_features: Default::default(),
                memory_hints: MemoryHints::Performance,
                trace: Trace::Off,
            }))
            .map_err(|err| format!("wgpu device request failed: {err}"))?;

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("ui-bind-group-layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("ui-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("ui-shader"),
            source: ShaderSource::Wgsl(UI_SHADER.into()),
        });
        let geometry_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("ui-geometry-shader"),
            source: ShaderSource::Wgsl(GEOMETRY_SHADER.into()),
        });
        let text_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("ui-text-bind-group-layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let text_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("ui-text-pipeline-layout"),
            bind_group_layouts: &[Some(&text_bind_group_layout)],
            immediate_size: 0,
        });
        let text_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("ui-text-shader"),
            source: ShaderSource::Wgsl(TEXT_SHADER.into()),
        });
        let blit_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("ui-blit-bind-group-layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("ui-blit-pipeline-layout"),
            bind_group_layouts: &[Some(&blit_bind_group_layout)],
            immediate_size: 0,
        });
        let blit_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("ui-blit-shader"),
            source: ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("ui-sampler"),
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            bind_group_layout,
            pipeline_layout,
            shader,
            geometry_shader,
            text_bind_group_layout,
            text_pipeline_layout,
            text_shader,
            blit_bind_group_layout,
            blit_pipeline_layout,
            blit_shader,
            sampler,
        })
    }

    fn create_pipeline(&self, surface_format: TextureFormat) -> RenderPipeline {
        self.device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("ui-render-pipeline"),
                layout: Some(&self.pipeline_layout),
                vertex: VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState {
                        format: surface_format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    fn create_fill_pipeline(&self, target_format: TextureFormat) -> RenderPipeline {
        self.device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("ui-fill-pipeline"),
                layout: None,
                vertex: VertexState {
                    module: &self.geometry_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[geometry_vertex_layout()],
                },
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    ..PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &self.geometry_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState {
                        format: target_format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    fn create_text_pipeline(&self, target_format: TextureFormat) -> RenderPipeline {
        self.device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("ui-text-pipeline"),
                layout: Some(&self.text_pipeline_layout),
                vertex: VertexState {
                    module: &self.text_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[text_vertex_layout()],
                },
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    ..PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &self.text_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState {
                        format: target_format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    fn create_blit_pipeline(&self, target_format: TextureFormat) -> RenderPipeline {
        self.device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("ui-blit-pipeline"),
                layout: Some(&self.blit_pipeline_layout),
                vertex: VertexState {
                    module: &self.blit_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    buffers: &[text_vertex_layout()],
                },
                primitive: PrimitiveState {
                    topology: PrimitiveTopology::TriangleList,
                    ..PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &self.blit_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: PipelineCompilationOptions::default(),
                    targets: &[Some(ColorTargetState {
                        format: target_format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }
}

impl TextAtlas {
    fn new(gpu: &UiGpuContext, width: u32, height: u32) -> Self {
        let (texture, view) = create_texture(
            &gpu.device,
            "ui-text-atlas-texture",
            TextureFormat::R8Unorm,
            width,
            height,
            mask_texture_usages(),
        );
        let bind_group = create_text_bind_group(gpu, &view);
        Self {
            texture,
            bind_group,
            width,
            height,
            next_x: 0,
            next_y: 0,
            row_height: 0,
            cache: HashMap::new(),
        }
    }

    fn reset(&mut self, gpu: &UiGpuContext) {
        let fresh = Self::new(gpu, self.width, self.height);
        *self = fresh;
    }

    fn slot_for(
        &mut self,
        gpu: &UiGpuContext,
        width: u32,
        height: u32,
        alpha: &[u8],
    ) -> Option<TextAtlasSlot> {
        if width == 0 || height == 0 || alpha.is_empty() {
            return None;
        }

        let key = TextAtlasKey {
            width,
            height,
            alpha: alpha.to_vec(),
        };
        if let Some(slot) = self.cache.get(&key).copied() {
            return Some(slot);
        }

        let slot = allocate_text_atlas_slot(
            &mut self.next_x,
            &mut self.next_y,
            &mut self.row_height,
            self.width,
            self.height,
            width,
            height,
            TEXT_ATLAS_PADDING,
        )
        .or_else(|| {
            self.reset(gpu);
            allocate_text_atlas_slot(
                &mut self.next_x,
                &mut self.next_y,
                &mut self.row_height,
                self.width,
                self.height,
                width,
                height,
                TEXT_ATLAS_PADDING,
            )
        })?;

        gpu.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: slot.x,
                    y: slot.y,
                    z: 0,
                },
                aspect: Default::default(),
            },
            alpha,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.max(1)),
                rows_per_image: Some(height.max(1)),
            },
            Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
        self.cache.insert(key, slot);
        Some(slot)
    }
}

impl WindowRenderTarget {
    /// guest surface bitmap 창용 렌더 타깃을 생성합니다.
    pub(crate) fn new_guest(
        gpu: &UiGpuContext,
        window: Arc<Window>,
        hwnd: u32,
        surface_bitmap: u32,
    ) -> Result<Self, String> {
        Self::new(
            gpu,
            window,
            WindowRenderContent::GuestBitmap {
                hwnd,
                surface_bitmap,
            },
        )
    }

    /// CPU painter 창용 렌더 타깃을 생성합니다.
    pub(crate) fn new_cpu_painter(
        gpu: &UiGpuContext,
        window: Arc<Window>,
        painter: Box<dyn Painter>,
    ) -> Result<Self, String> {
        Self::new(gpu, window, WindowRenderContent::CpuPainter(painter))
    }

    fn new(
        gpu: &UiGpuContext,
        window: Arc<Window>,
        content: WindowRenderContent,
    ) -> Result<Self, String> {
        let surface = gpu
            .instance
            .create_surface(window.clone())
            .map_err(|err| format!("surface creation failed: {err}"))?;
        let size = window.inner_size();
        let capabilities = surface.get_capabilities(&gpu.adapter);
        let surface_format = choose_surface_format(&capabilities.formats);
        let present_mode = choose_present_mode(&capabilities.present_modes);
        let alpha_mode = choose_alpha_mode(&capabilities.alpha_modes);
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        let mask_size = (config.width, config.height);

        surface.configure(&gpu.device, &config);

        let content_format = choose_frame_texture_format(&gpu.adapter);
        let pipeline = gpu.create_pipeline(config.format);
        let fill_pipeline = gpu.create_fill_pipeline(content_format);
        let text_pipeline = gpu.create_text_pipeline(content_format);
        let blit_pipeline = gpu.create_blit_pipeline(content_format);
        let (frame_texture, frame_view) = create_texture(
            &gpu.device,
            "ui-frame-texture",
            content_format,
            1,
            1,
            frame_texture_usages(),
        );
        let (mask_texture, mask_view) = create_texture(
            &gpu.device,
            "ui-mask-texture",
            TextureFormat::R8Unorm,
            config.width,
            config.height,
            mask_texture_usages(),
        );
        let text_atlas = TextAtlas::new(gpu, TEXT_ATLAS_SIZE, TEXT_ATLAS_SIZE);
        let bind_group = create_bind_group(gpu, &frame_view, &mask_view);

        Ok(Self {
            window,
            surface,
            config,
            pipeline,
            fill_pipeline,
            text_pipeline,
            blit_pipeline,
            content_format,
            frame_texture,
            frame_view,
            frame_size: (1, 1),
            needs_full_frame_upload: true,
            mask_texture,
            mask_view,
            mask_size,
            text_atlas,
            bind_group,
            last_mask: None,
            scratch_pixels: Vec::new(),
            scratch_bytes: Vec::new(),
            geometry_bytes: Vec::new(),
            geometry_vertex_count: 0,
            content,
        })
    }

    /// 이 타깃이 소유한 실제 호스트 창을 반환합니다.
    pub(crate) fn window(&self) -> &Window {
        self.window.as_ref()
    }

    /// 창별 이벤트를 내부 painter가 처리하도록 위임합니다.
    pub(crate) fn handle_event(
        &mut self,
        event: &winit::event::WindowEvent,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> bool {
        match &mut self.content {
            WindowRenderContent::GuestBitmap { .. } => false,
            WindowRenderContent::CpuPainter(painter) => painter.handle_event(event, event_loop),
        }
    }

    /// 내부 painter의 주기 처리를 실행합니다.
    pub(crate) fn tick(&mut self) -> bool {
        match &mut self.content {
            WindowRenderContent::GuestBitmap { .. } => false,
            WindowRenderContent::CpuPainter(painter) => painter.tick(),
        }
    }

    /// 창이 닫혀야 하는지 확인합니다.
    pub(crate) fn should_close(&self) -> bool {
        match &self.content {
            WindowRenderContent::GuestBitmap { .. } => false,
            WindowRenderContent::CpuPainter(painter) => painter.should_close(),
        }
    }

    /// 닫기 요청 시 즉시 앱 종료가 필요한지 확인합니다.
    pub(crate) fn quit_on_close(&self) -> bool {
        match &self.content {
            WindowRenderContent::GuestBitmap { .. } => false,
            WindowRenderContent::CpuPainter(painter) => painter.quit_on_close(),
        }
    }

    /// 내부 painter의 폴링 간격을 반환합니다.
    pub(crate) fn poll_interval(&self) -> Option<std::time::Duration> {
        match &self.content {
            WindowRenderContent::GuestBitmap { .. } => None,
            WindowRenderContent::CpuPainter(painter) => painter.poll_interval(),
        }
    }

    /// 창 크기 변화에 맞춰 surface 설정과 마스크 텍스처를 동기화합니다.
    pub(crate) fn reconfigure_surface(&mut self, gpu: &UiGpuContext) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.config.width == size.width && self.config.height == size.height {
            return;
        }

        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&gpu.device, &self.config);
        self.recreate_mask_texture(gpu, size.width, size.height);
        self.last_mask = None;
    }

    /// 현재 콘텐츠를 GPU 텍스처로 업로드하고 창에 출력합니다.
    pub(crate) fn render(
        &mut self,
        gpu: &UiGpuContext,
        emu_context: &Win32Context,
    ) -> Result<RenderOutcome, RenderFrameError> {
        self.reconfigure_surface(gpu);
        if self.config.width == 0 || self.config.height == 0 {
            return Ok(RenderOutcome::Skipped);
        }

        if let WindowRenderContent::GuestBitmap {
            hwnd,
            surface_bitmap,
        } = &self.content
        {
            let hwnd = *hwnd;
            let surface_bitmap = *surface_bitmap;
            if emu_context.surface_bitmap_dc_active(surface_bitmap) {
                return Ok(RenderOutcome::Skipped);
            }
            let Some(snapshot) = snapshot_guest_bitmap(emu_context, hwnd, surface_bitmap) else {
                return Ok(RenderOutcome::Skipped);
            };
            let Ok(pixels) = snapshot.pixels.try_lock() else {
                return Ok(RenderOutcome::Skipped);
            };
            if pixels.is_empty() {
                return Ok(RenderOutcome::Skipped);
            }
            if !emu_context.surface_bitmap_has_content(surface_bitmap) {
                return Ok(RenderOutcome::Skipped);
            }

            self.ensure_frame_texture(gpu, snapshot.width.max(1), snapshot.height.max(1));
            let needs_full_upload = self.needs_full_frame_upload
                || emu_context.consume_surface_bitmap_full_upload(surface_bitmap);
            if needs_full_upload {
                upload_argb_pixels(
                    gpu,
                    &self.frame_texture,
                    self.content_format,
                    &pixels,
                    snapshot.width.max(1),
                    snapshot.height.max(1),
                    &mut self.scratch_bytes,
                );
                self.needs_full_frame_upload = false;
                emu_context.take_surface_bitmap_ops(surface_bitmap);
            } else {
                let mut frame_encoder =
                    gpu.device
                        .create_command_encoder(&CommandEncoderDescriptor {
                            label: Some("ui-frame-ops-encoder"),
                        });
                let ops = emu_context.take_surface_bitmap_ops(surface_bitmap);
                self.apply_surface_ops(gpu, &mut frame_encoder, &ops);
                gpu.queue.submit([frame_encoder.finish()]);
            }
            drop(pixels);
            self.ensure_mask_texture(
                gpu,
                MaskCacheKey {
                    width: self.config.width.max(1),
                    height: self.config.height.max(1),
                    rects: snapshot.rects,
                },
            );
        } else if let WindowRenderContent::CpuPainter(painter) = &mut self.content {
            let width = self.config.width.max(1);
            let height = self.config.height.max(1);
            let pixel_len = width.saturating_mul(height) as usize;
            self.scratch_pixels.resize(pixel_len, 0);
            self.scratch_pixels.fill(0);
            if !painter.paint(&mut self.scratch_pixels, width, height) {
                return Ok(RenderOutcome::Skipped);
            }

            self.ensure_frame_texture(gpu, width, height);
            upload_argb_pixels(
                gpu,
                &self.frame_texture,
                self.content_format,
                &self.scratch_pixels,
                width,
                height,
                &mut self.scratch_bytes,
            );
            self.ensure_mask_texture(
                gpu,
                MaskCacheKey {
                    width,
                    height,
                    rects: Vec::new(),
                },
            );
        }

        let surface_texture = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(texture)
            | CurrentSurfaceTexture::Suboptimal(texture) => texture,
            CurrentSurfaceTexture::Timeout => {
                return Err(RenderFrameError::Surface(SurfaceAcquireError::Timeout));
            }
            CurrentSurfaceTexture::Occluded => {
                return Err(RenderFrameError::Surface(SurfaceAcquireError::Occluded));
            }
            CurrentSurfaceTexture::Outdated => {
                return Err(RenderFrameError::Surface(SurfaceAcquireError::Outdated));
            }
            CurrentSurfaceTexture::Lost => {
                return Err(RenderFrameError::Surface(SurfaceAcquireError::Lost));
            }
            CurrentSurfaceTexture::Validation => {
                return Err(RenderFrameError::Surface(SurfaceAcquireError::Validation));
            }
        };
        let surface_view = surface_texture
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("ui-render-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("ui-render-pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.queue.submit([encoder.finish()]);
        // 일부 플랫폼은 present 직전 알림이 없으면 첫 프레임 노출이 늦어질 수 있습니다.
        self.window.pre_present_notify();
        surface_texture.present();
        Ok(RenderOutcome::Rendered)
    }

    fn ensure_frame_texture(&mut self, gpu: &UiGpuContext, width: u32, height: u32) {
        if self.frame_size == (width, height) {
            return;
        }

        let (texture, view) = create_texture(
            &gpu.device,
            "ui-frame-texture",
            self.content_format,
            width,
            height,
            frame_texture_usages(),
        );
        self.frame_texture = texture;
        self.frame_view = view;
        self.frame_size = (width, height);
        self.needs_full_frame_upload = true;
        self.refresh_bind_group(gpu);
    }

    fn ensure_mask_texture(&mut self, gpu: &UiGpuContext, next: MaskCacheKey) {
        if self.last_mask.as_ref() == Some(&next) {
            return;
        }

        if self.mask_size != (next.width, next.height) {
            self.recreate_mask_texture(gpu, next.width, next.height);
        }

        let mask_bytes = build_region_mask(&next.rects, next.width, next.height);
        gpu.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &self.mask_texture,
                mip_level: 0,
                origin: Default::default(),
                aspect: Default::default(),
            },
            &mask_bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(next.width.max(1)),
                rows_per_image: Some(next.height.max(1)),
            },
            Extent3d {
                width: next.width.max(1),
                height: next.height.max(1),
                depth_or_array_layers: 1,
            },
        );
        self.last_mask = Some(next);
    }

    fn recreate_mask_texture(&mut self, gpu: &UiGpuContext, width: u32, height: u32) {
        let (texture, view) = create_texture(
            &gpu.device,
            "ui-mask-texture",
            TextureFormat::R8Unorm,
            width.max(1),
            height.max(1),
            mask_texture_usages(),
        );
        self.mask_texture = texture;
        self.mask_view = view;
        self.mask_size = (width.max(1), height.max(1));
        self.refresh_bind_group(gpu);
    }

    fn refresh_bind_group(&mut self, gpu: &UiGpuContext) {
        self.bind_group = create_bind_group(gpu, &self.frame_view, &self.mask_view);
    }

    fn apply_surface_ops(
        &mut self,
        gpu: &UiGpuContext,
        encoder: &mut CommandEncoder,
        ops: &[GpuSurfaceOp],
    ) {
        for op in ops {
            match op {
                GpuSurfaceOp::Upload(update) => {
                    encode_bitmap_update(
                        gpu,
                        encoder,
                        &self.frame_texture,
                        self.content_format,
                        update,
                        &mut self.scratch_bytes,
                    );
                }
                GpuSurfaceOp::Draw(command) => {
                    self.render_draw_commands(gpu, encoder, std::slice::from_ref(command));
                }
            }
        }
    }

    fn render_draw_commands(
        &mut self,
        gpu: &UiGpuContext,
        encoder: &mut CommandEncoder,
        commands: &[GpuDrawCommand],
    ) {
        if commands.is_empty() || self.frame_size.0 == 0 || self.frame_size.1 == 0 {
            return;
        }

        self.geometry_bytes.clear();
        self.geometry_vertex_count = 0;
        for command in commands {
            match command {
                GpuDrawCommand::FillRect {
                    left,
                    top,
                    right,
                    bottom,
                    color,
                    ..
                } => self.push_fill_rect_vertices(*left, *top, *right, *bottom, *color),
                GpuDrawCommand::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                    ..
                } => self.push_line_vertices(*x1, *y1, *x2, *y2, *color),
                GpuDrawCommand::TextMask {
                    x,
                    y,
                    width,
                    height,
                    color,
                    alpha,
                    ..
                } => {
                    self.flush_geometry_batch(gpu, encoder);
                    self.render_text_mask_command(
                        gpu, encoder, *x, *y, *width, *height, *color, alpha,
                    );
                }
                GpuDrawCommand::Blit {
                    left,
                    top,
                    right,
                    bottom,
                    src_width,
                    src_height,
                    uv,
                    pixels,
                    ..
                } => {
                    self.flush_geometry_batch(gpu, encoder);
                    self.render_blit_command(
                        gpu,
                        encoder,
                        *left,
                        *top,
                        *right,
                        *bottom,
                        *src_width,
                        *src_height,
                        *uv,
                        pixels,
                    );
                }
            }
        }
        self.flush_geometry_batch(gpu, encoder);
    }

    fn push_fill_rect_vertices(
        &mut self,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        color: u32,
    ) {
        let clipped_left = left.max(0).min(self.frame_size.0 as i32);
        let clipped_top = top.max(0).min(self.frame_size.1 as i32);
        let clipped_right = right.max(0).min(self.frame_size.0 as i32);
        let clipped_bottom = bottom.max(0).min(self.frame_size.1 as i32);
        if clipped_left >= clipped_right || clipped_top >= clipped_bottom {
            return;
        }

        let (r, g, b, a) = argb_to_rgba_f32(color);
        let vertices = [
            (
                pixel_to_ndc_x(clipped_left as f32, self.frame_size.0),
                pixel_to_ndc_y(clipped_top as f32, self.frame_size.1),
            ),
            (
                pixel_to_ndc_x(clipped_right as f32, self.frame_size.0),
                pixel_to_ndc_y(clipped_top as f32, self.frame_size.1),
            ),
            (
                pixel_to_ndc_x(clipped_right as f32, self.frame_size.0),
                pixel_to_ndc_y(clipped_bottom as f32, self.frame_size.1),
            ),
            (
                pixel_to_ndc_x(clipped_left as f32, self.frame_size.0),
                pixel_to_ndc_y(clipped_top as f32, self.frame_size.1),
            ),
            (
                pixel_to_ndc_x(clipped_right as f32, self.frame_size.0),
                pixel_to_ndc_y(clipped_bottom as f32, self.frame_size.1),
            ),
            (
                pixel_to_ndc_x(clipped_left as f32, self.frame_size.0),
                pixel_to_ndc_y(clipped_bottom as f32, self.frame_size.1),
            ),
        ];

        for (x, y) in vertices {
            self.geometry_bytes.extend_from_slice(&x.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&y.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&r.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&g.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&b.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&a.to_le_bytes());
        }
        self.geometry_vertex_count += 6;
    }

    fn flush_geometry_batch(&mut self, gpu: &UiGpuContext, encoder: &mut CommandEncoder) {
        if self.geometry_vertex_count == 0 || self.geometry_bytes.is_empty() {
            return;
        }

        let vertex_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui-geometry-buffer"),
                contents: &self.geometry_bytes,
                usage: BufferUsages::VERTEX,
            });

        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("ui-geometry-pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.frame_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.fill_pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..self.geometry_vertex_count, 0..1);
        }
        self.geometry_bytes.clear();
        self.geometry_vertex_count = 0;
    }

    fn push_line_vertices(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: u32) {
        let dx = (x2 - x1) as f32;
        let dy = (y2 - y1) as f32;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f32::EPSILON {
            self.push_fill_rect_vertices(x1, y1, x1 + 1, y1 + 1, color);
            return;
        }

        let nx = -dy / length * 0.5;
        let ny = dx / length * 0.5;
        let (r, g, b, a) = argb_to_rgba_f32(color);
        let vertices = [
            (x1 as f32 + nx, y1 as f32 + ny),
            (x2 as f32 + nx, y2 as f32 + ny),
            (x2 as f32 - nx, y2 as f32 - ny),
            (x1 as f32 + nx, y1 as f32 + ny),
            (x2 as f32 - nx, y2 as f32 - ny),
            (x1 as f32 - nx, y1 as f32 - ny),
        ];

        for (x, y) in vertices {
            self.geometry_bytes
                .extend_from_slice(&pixel_to_ndc_x(x, self.frame_size.0).to_le_bytes());
            self.geometry_bytes
                .extend_from_slice(&pixel_to_ndc_y(y, self.frame_size.1).to_le_bytes());
            self.geometry_bytes.extend_from_slice(&r.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&g.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&b.to_le_bytes());
            self.geometry_bytes.extend_from_slice(&a.to_le_bytes());
        }
        self.geometry_vertex_count += 6;
    }

    fn render_text_mask_command(
        &mut self,
        gpu: &UiGpuContext,
        encoder: &mut CommandEncoder,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        color: u32,
        alpha: &[u8],
    ) {
        if width == 0 || height == 0 || alpha.is_empty() {
            return;
        }
        let Some(slot) = self.text_atlas.slot_for(gpu, width, height, alpha) else {
            return;
        };
        let vertex_bytes = build_text_vertices(
            x,
            y,
            width,
            height,
            color,
            self.frame_size,
            text_atlas_uv(slot, self.text_atlas.width, self.text_atlas.height),
        );
        if vertex_bytes.is_empty() {
            return;
        }
        let vertex_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui-text-buffer"),
                contents: &vertex_bytes,
                usage: BufferUsages::VERTEX,
            });

        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("ui-text-pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.frame_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.text_pipeline);
            pass.set_bind_group(0, &self.text_atlas.bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
    }

    fn render_blit_command(
        &mut self,
        gpu: &UiGpuContext,
        encoder: &mut CommandEncoder,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        src_width: u32,
        src_height: u32,
        uv: [f32; 4],
        pixels: &[u32],
    ) {
        if src_width == 0 || src_height == 0 || pixels.is_empty() {
            return;
        }

        let (src_texture, src_view) = create_texture(
            &gpu.device,
            "ui-blit-source-texture",
            self.content_format,
            src_width,
            src_height,
            frame_texture_usages(),
        );
        upload_argb_pixels(
            gpu,
            &src_texture,
            self.content_format,
            pixels,
            src_width,
            src_height,
            &mut self.scratch_bytes,
        );
        let bind_group = create_blit_bind_group(gpu, &src_view);
        let vertex_bytes = build_text_vertices(
            left,
            top,
            (right - left).max(0) as u32,
            (bottom - top).max(0) as u32,
            0xFFFF_FFFF,
            self.frame_size,
            [
                (uv[0], uv[1]),
                (uv[2], uv[1]),
                (uv[2], uv[3]),
                (uv[0], uv[3]),
            ],
        );
        if vertex_bytes.is_empty() {
            return;
        }
        let vertex_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ui-blit-buffer"),
                contents: &vertex_bytes,
                usage: BufferUsages::VERTEX,
            });

        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("ui-blit-pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.frame_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
    }
}

fn create_bind_group(
    gpu: &UiGpuContext,
    frame_view: &TextureView,
    mask_view: &TextureView,
) -> BindGroup {
    gpu.device.create_bind_group(&BindGroupDescriptor {
        label: Some("ui-bind-group"),
        layout: &gpu.bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(frame_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(mask_view),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
        ],
    })
}

fn create_text_bind_group(gpu: &UiGpuContext, alpha_view: &TextureView) -> BindGroup {
    gpu.device.create_bind_group(&BindGroupDescriptor {
        label: Some("ui-text-bind-group"),
        layout: &gpu.text_bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(alpha_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
        ],
    })
}

fn create_blit_bind_group(gpu: &UiGpuContext, src_view: &TextureView) -> BindGroup {
    gpu.device.create_bind_group(&BindGroupDescriptor {
        label: Some("ui-blit-bind-group"),
        layout: &gpu.blit_bind_group_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(src_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
        ],
    })
}

fn create_texture(
    device: &Device,
    label: &str,
    format: TextureFormat,
    width: u32,
    height: u32,
    usage: TextureUsages,
) -> (Texture, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    (texture, view)
}

fn geometry_vertex_layout<'a>() -> VertexBufferLayout<'a> {
    VertexBufferLayout {
        array_stride: 24,
        step_mode: VertexStepMode::Vertex,
        attributes: &[
            VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: VertexFormat::Float32x2,
            },
            VertexAttribute {
                offset: 8,
                shader_location: 1,
                format: VertexFormat::Float32x4,
            },
        ],
    }
}

fn text_vertex_layout<'a>() -> VertexBufferLayout<'a> {
    VertexBufferLayout {
        array_stride: 32,
        step_mode: VertexStepMode::Vertex,
        attributes: &[
            VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: VertexFormat::Float32x2,
            },
            VertexAttribute {
                offset: 8,
                shader_location: 1,
                format: VertexFormat::Float32x2,
            },
            VertexAttribute {
                offset: 16,
                shader_location: 2,
                format: VertexFormat::Float32x4,
            },
        ],
    }
}

fn allocate_text_atlas_slot(
    next_x: &mut u32,
    next_y: &mut u32,
    row_height: &mut u32,
    atlas_width: u32,
    atlas_height: u32,
    width: u32,
    height: u32,
    padding: u32,
) -> Option<TextAtlasSlot> {
    if width == 0 || height == 0 || width > atlas_width || height > atlas_height {
        return None;
    }

    if *next_x + width > atlas_width {
        *next_x = 0;
        *next_y = next_y.saturating_add(*row_height + padding);
        *row_height = 0;
    }
    if *next_y + height > atlas_height {
        return None;
    }

    let slot = TextAtlasSlot {
        x: *next_x,
        y: *next_y,
        width,
        height,
    };
    *next_x = next_x.saturating_add(width + padding);
    *row_height = (*row_height).max(height);
    Some(slot)
}

fn pixel_to_ndc_x(x: f32, width: u32) -> f32 {
    (x / width.max(1) as f32) * 2.0 - 1.0
}

fn pixel_to_ndc_y(y: f32, height: u32) -> f32 {
    1.0 - (y / height.max(1) as f32) * 2.0
}

fn argb_to_rgba_f32(color: u32) -> (f32, f32, f32, f32) {
    let a = ((color >> 24) & 0xFF) as f32 / 255.0;
    let r = ((color >> 16) & 0xFF) as f32 / 255.0;
    let g = ((color >> 8) & 0xFF) as f32 / 255.0;
    let b = (color & 0xFF) as f32 / 255.0;
    (r, g, b, a)
}

fn text_atlas_uv(slot: TextAtlasSlot, atlas_width: u32, atlas_height: u32) -> [(f32, f32); 4] {
    let left = slot.x as f32 / atlas_width.max(1) as f32;
    let top = slot.y as f32 / atlas_height.max(1) as f32;
    let right = (slot.x + slot.width) as f32 / atlas_width.max(1) as f32;
    let bottom = (slot.y + slot.height) as f32 / atlas_height.max(1) as f32;
    [(left, top), (right, top), (right, bottom), (left, bottom)]
}

fn build_text_vertices(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    color: u32,
    frame_size: (u32, u32),
    uv_rect: [(f32, f32); 4],
) -> Vec<u8> {
    let left = x.max(0).min(frame_size.0 as i32) as f32;
    let top = y.max(0).min(frame_size.1 as i32) as f32;
    let right = (x + width as i32).max(0).min(frame_size.0 as i32) as f32;
    let bottom = (y + height as i32).max(0).min(frame_size.1 as i32) as f32;
    if left >= right || top >= bottom {
        return Vec::new();
    }

    let (r, g, b, a) = argb_to_rgba_f32(color);
    let vertices = [
        (left, top, uv_rect[0].0, uv_rect[0].1),
        (right, top, uv_rect[1].0, uv_rect[1].1),
        (right, bottom, uv_rect[2].0, uv_rect[2].1),
        (left, top, uv_rect[0].0, uv_rect[0].1),
        (right, bottom, uv_rect[2].0, uv_rect[2].1),
        (left, bottom, uv_rect[3].0, uv_rect[3].1),
    ];

    let mut bytes = Vec::with_capacity(6 * 32);
    for (px, py, u, v) in vertices {
        bytes.extend_from_slice(&pixel_to_ndc_x(px, frame_size.0).to_le_bytes());
        bytes.extend_from_slice(&pixel_to_ndc_y(py, frame_size.1).to_le_bytes());
        bytes.extend_from_slice(&u.to_le_bytes());
        bytes.extend_from_slice(&v.to_le_bytes());
        bytes.extend_from_slice(&r.to_le_bytes());
        bytes.extend_from_slice(&g.to_le_bytes());
        bytes.extend_from_slice(&b.to_le_bytes());
        bytes.extend_from_slice(&a.to_le_bytes());
    }
    bytes
}

fn snapshot_guest_bitmap(
    emu_context: &Win32Context,
    hwnd: u32,
    surface_bitmap: u32,
) -> Option<GuestBitmapSnapshot> {
    let mask_rects = {
        let win_event = emu_context.win_event.try_lock().ok()?;
        let window_rgn = win_event
            .windows
            .get(&hwnd)
            .map(|window| window.window_rgn)
            .unwrap_or(0);
        if window_rgn == 0 {
            Vec::new()
        } else {
            let gdi_objects = emu_context.gdi_objects.try_lock().ok()?;
            match gdi_objects.get(&window_rgn) {
                Some(GdiObject::Region { rects }) => rects.clone(),
                _ => Vec::new(),
            }
        }
    };

    let gdi_objects = emu_context.gdi_objects.try_lock().ok()?;
    let (width, height, pixels) = match gdi_objects.get(&surface_bitmap) {
        Some(GdiObject::Bitmap {
            width,
            height,
            pixels,
            ..
        }) => (*width, *height, pixels.clone()),
        _ => return None,
    };
    drop(gdi_objects);

    Some(GuestBitmapSnapshot {
        width,
        height,
        pixels,
        rects: mask_rects,
    })
}

fn upload_argb_pixels(
    gpu: &UiGpuContext,
    texture: &Texture,
    format: TextureFormat,
    pixels: &[u32],
    width: u32,
    height: u32,
    scratch_bytes: &mut Vec<u8>,
) {
    upload_argb_pixels_at(
        gpu,
        texture,
        format,
        pixels,
        width,
        height,
        0,
        0,
        scratch_bytes,
    );
}

fn encode_bitmap_update(
    gpu: &UiGpuContext,
    encoder: &mut CommandEncoder,
    texture: &Texture,
    format: TextureFormat,
    update: &GpuBitmapUpdate,
    scratch_bytes: &mut Vec<u8>,
) {
    if update.width == 0 || update.height == 0 || update.pixels.is_empty() {
        return;
    }

    encode_argb_pixels_at(
        gpu,
        encoder,
        texture,
        format,
        &update.pixels,
        update.width,
        update.height,
        update.x,
        update.y,
        scratch_bytes,
    );
}

fn upload_argb_pixels_at(
    gpu: &UiGpuContext,
    texture: &Texture,
    format: TextureFormat,
    pixels: &[u32],
    width: u32,
    height: u32,
    dst_x: u32,
    dst_y: u32,
    scratch_bytes: &mut Vec<u8>,
) {
    let bytes = argb_pixels_as_bytes(format, pixels, scratch_bytes);

    gpu.queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: dst_x,
                y: dst_y,
                z: 0,
            },
            aspect: Default::default(),
        },
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width.max(1).saturating_mul(4)),
            rows_per_image: Some(height.max(1)),
        },
        Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
    );
}

fn encode_argb_pixels_at(
    gpu: &UiGpuContext,
    encoder: &mut CommandEncoder,
    texture: &Texture,
    format: TextureFormat,
    pixels: &[u32],
    width: u32,
    height: u32,
    dst_x: u32,
    dst_y: u32,
    scratch_bytes: &mut Vec<u8>,
) {
    let bytes = argb_pixels_as_bytes(format, pixels, scratch_bytes);
    let staging = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui-texture-upload-buffer"),
            contents: bytes,
            usage: BufferUsages::COPY_SRC,
        });
    encoder.copy_buffer_to_texture(
        TexelCopyBufferInfo {
            buffer: &staging,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.max(1).saturating_mul(4)),
                rows_per_image: Some(height.max(1)),
            },
        },
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: dst_x,
                y: dst_y,
                z: 0,
            },
            aspect: Default::default(),
        },
        Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
    );
}

fn argb_pixels_as_bytes<'a>(
    format: TextureFormat,
    pixels: &'a [u32],
    scratch_bytes: &'a mut Vec<u8>,
) -> &'a [u8] {
    if format == TextureFormat::Bgra8Unorm {
        // `0xAARRGGBB` u32는 리틀엔디언 메모리에서 BGRA 바이트 순서와 같으므로 그대로 업로드합니다.
        unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4) }
    } else {
        scratch_bytes.clear();
        scratch_bytes.reserve(pixels.len().saturating_mul(4));
        for &pixel in pixels {
            let a = (pixel >> 24) as u8;
            let r = (pixel >> 16) as u8;
            let g = (pixel >> 8) as u8;
            let b = pixel as u8;
            scratch_bytes.extend_from_slice(&[r, g, b, a]);
        }
        scratch_bytes.as_slice()
    }
}

/// surface format 후보 중 BGRA를 우선 선택합니다.
pub(crate) fn choose_surface_format(formats: &[TextureFormat]) -> TextureFormat {
    for format in [TextureFormat::Bgra8Unorm, TextureFormat::Rgba8Unorm] {
        if formats.contains(&format) {
            return format;
        }
    }
    formats
        .first()
        .copied()
        .unwrap_or(TextureFormat::Bgra8Unorm)
}

fn choose_present_mode(present_modes: &[PresentMode]) -> PresentMode {
    present_modes
        .iter()
        .copied()
        .find(|mode| *mode == PresentMode::Fifo)
        .or_else(|| present_modes.first().copied())
        .unwrap_or(PresentMode::Fifo)
}

fn choose_alpha_mode(alpha_modes: &[CompositeAlphaMode]) -> CompositeAlphaMode {
    alpha_modes
        .iter()
        .copied()
        .find(|mode| *mode == CompositeAlphaMode::PreMultiplied)
        .or_else(|| {
            alpha_modes
                .iter()
                .copied()
                .find(|mode| *mode == CompositeAlphaMode::PreMultiplied)
        })
        .or_else(|| alpha_modes.first().copied())
        .unwrap_or(CompositeAlphaMode::Auto)
}

fn choose_frame_texture_format(adapter: &Adapter) -> TextureFormat {
    for format in [TextureFormat::Bgra8Unorm, TextureFormat::Rgba8Unorm] {
        let features = adapter.get_texture_format_features(format);
        if features.allowed_usages.contains(frame_texture_usages()) {
            return format;
        }
    }
    TextureFormat::Rgba8Unorm
}

fn frame_texture_usages() -> TextureUsages {
    TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT
}

fn mask_texture_usages() -> TextureUsages {
    TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING
}

/// 영역 직사각형 목록을 `R8Unorm` 마스크 바이트로 변환합니다.
pub(crate) fn build_region_mask(
    rects: &[(i32, i32, i32, i32)],
    width: u32,
    height: u32,
) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    if rects.is_empty() {
        return vec![0xFF; width.saturating_mul(height) as usize];
    }

    let mut mask = vec![0u8; width.saturating_mul(height) as usize];
    for &(left, top, right, bottom) in rects {
        let left = left.clamp(0, width as i32);
        let right = right.clamp(0, width as i32);
        let top = top.clamp(0, height as i32);
        let bottom = bottom.clamp(0, height as i32);
        if left >= right || top >= bottom {
            continue;
        }
        for y in top..bottom {
            let row = y as usize * width as usize;
            for x in left..right {
                mask[row + x as usize] = 0xFF;
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::{
        TextAtlasSlot, allocate_text_atlas_slot, build_region_mask, choose_surface_format,
        text_atlas_uv,
    };
    use wgpu::TextureFormat;

    #[test]
    fn choose_surface_format_prefers_bgra() {
        let chosen = choose_surface_format(&[TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Unorm]);
        assert_eq!(chosen, TextureFormat::Bgra8Unorm);
    }

    #[test]
    fn build_region_mask_marks_inside_pixels() {
        let mask = build_region_mask(&[(1, 1, 3, 3)], 4, 4);
        assert_eq!(
            mask,
            vec![0, 0, 0, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 0, 0, 0,]
        );
    }

    #[test]
    fn atlas_allocator_wraps_to_next_row() {
        let mut next_x = 0;
        let mut next_y = 0;
        let mut row_height = 0;

        let first =
            allocate_text_atlas_slot(&mut next_x, &mut next_y, &mut row_height, 8, 8, 5, 2, 1)
                .unwrap();
        let second =
            allocate_text_atlas_slot(&mut next_x, &mut next_y, &mut row_height, 8, 8, 4, 3, 1)
                .unwrap();

        assert_eq!(first.x, 0);
        assert_eq!(first.y, 0);
        assert_eq!(second.x, 0);
        assert_eq!(second.y, 3);
    }

    #[test]
    fn text_atlas_uv_maps_slot_bounds() {
        let uv = text_atlas_uv(
            TextAtlasSlot {
                x: 2,
                y: 4,
                width: 4,
                height: 2,
            },
            8,
            8,
        );

        assert_eq!(uv[0], (0.25, 0.5));
        assert_eq!(uv[2], (0.75, 0.75));
    }
}
