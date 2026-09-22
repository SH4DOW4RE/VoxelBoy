use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use glam::camera::rh::{proj::directx::perspective, view::look_at_mat4};
use pixels::wgpu;
use pixels::wgpu::util::DeviceExt;
use voxelboy_voxel::Voxel;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CubeVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VoxelInstance {
    position: [f32; 3],
    size: [f32; 3],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct CameraState {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl CameraState {
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn for_frame(width: u32, height: u32) -> Self {
        Self {
            yaw: 0.35,
            pitch: -0.25,
            distance: width.max(height) as f32 * 1.75,
        }
    }

    fn uniform(self, aspect_ratio: f32) -> CameraUniform {
        let horizontal = self.distance * self.pitch.cos();
        let eye = Vec3::new(
            horizontal * self.yaw.sin(),
            self.distance * self.pitch.sin(),
            horizontal * self.yaw.cos(),
        );
        let view = look_at_mat4(eye, Vec3::ZERO, Vec3::Y);
        let projection = perspective(45_f32.to_radians(), aspect_ratio, 0.1, 2_000.0);
        CameraUniform {
            view_projection: (projection * view).to_cols_array_2d(),
        }
    }
}

pub struct VoxelRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    surface_width: u32,
    surface_height: u32,
}

impl VoxelRenderer {
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        surface_width: u32,
        surface_height: u32,
    ) -> Self {
        let (vertices, indices) = cube_mesh();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelboy_cube_vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelboy_cube_indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_buffer = create_instance_buffer(device, 1);
        let camera_uniform = CameraState::for_frame(160, 144).uniform(1.0);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("voxelboy_camera_uniform"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("voxelboy_camera_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<CameraUniform>() as u64),
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxelboy_camera_bind_group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("voxel.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("voxelboy_voxel_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("voxelboy_voxel_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout(), instance_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let depth_view = create_depth_view(device, surface_width, surface_height);

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: u32::try_from(indices.len()).unwrap_or(u32::MAX),
            instance_buffer,
            instance_capacity: 1,
            instance_count: 0,
            camera_buffer,
            camera_bind_group,
            depth_view,
            surface_width,
            surface_height,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_width = width;
        self.surface_height = height;
        self.depth_view = create_depth_view(device, width, height);
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        voxels: &[Voxel],
        frame_width: u32,
        frame_height: u32,
        camera: CameraState,
    ) {
        let width_offset = frame_width as f32 * 0.5;
        let height_offset = frame_height as f32 * 0.5;
        let instances: Vec<VoxelInstance> = voxels
            .iter()
            .map(|voxel| VoxelInstance {
                position: [
                    voxel.position[0] - width_offset,
                    voxel.position[1] + height_offset,
                    voxel.size[2] * 0.5,
                ],
                size: voxel.size,
                color: [
                    f32::from(voxel.color.red) / 255.0,
                    f32::from(voxel.color.green) / 255.0,
                    f32::from(voxel.color.blue) / 255.0,
                    f32::from(voxel.color.alpha) / 255.0,
                ],
            })
            .collect();
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instance_buffer = create_instance_buffer(device, self.instance_capacity);
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
        self.instance_count = u32::try_from(instances.len()).unwrap_or(u32::MAX);

        let aspect_ratio = self.surface_width as f32 / self.surface_height.max(1) as f32;
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&camera.uniform(aspect_ratio)),
        );
    }

    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("voxelboy_voxel_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.025,
                        g: 0.03,
                        b: 0.045,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);
    }
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<CubeVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
        ],
    }
}

fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<VoxelInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 24,
                shader_location: 4,
            },
        ],
    }
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("voxelboy_voxel_instances"),
        size: capacity.max(1).saturating_mul(size_of::<VoxelInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("voxelboy_depth_texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn cube_mesh() -> ([CubeVertex; 24], [u16; 36]) {
    let vertices = [
        face_vertex([-0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
        face_vertex([0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
        face_vertex([0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
        face_vertex([-0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
        face_vertex([0.5, -0.5, -0.5], [0.0, 0.0, -1.0]),
        face_vertex([-0.5, -0.5, -0.5], [0.0, 0.0, -1.0]),
        face_vertex([-0.5, 0.5, -0.5], [0.0, 0.0, -1.0]),
        face_vertex([0.5, 0.5, -0.5], [0.0, 0.0, -1.0]),
        face_vertex([0.5, -0.5, 0.5], [1.0, 0.0, 0.0]),
        face_vertex([0.5, -0.5, -0.5], [1.0, 0.0, 0.0]),
        face_vertex([0.5, 0.5, -0.5], [1.0, 0.0, 0.0]),
        face_vertex([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
        face_vertex([-0.5, -0.5, -0.5], [-1.0, 0.0, 0.0]),
        face_vertex([-0.5, -0.5, 0.5], [-1.0, 0.0, 0.0]),
        face_vertex([-0.5, 0.5, 0.5], [-1.0, 0.0, 0.0]),
        face_vertex([-0.5, 0.5, -0.5], [-1.0, 0.0, 0.0]),
        face_vertex([-0.5, 0.5, 0.5], [0.0, 1.0, 0.0]),
        face_vertex([0.5, 0.5, 0.5], [0.0, 1.0, 0.0]),
        face_vertex([0.5, 0.5, -0.5], [0.0, 1.0, 0.0]),
        face_vertex([-0.5, 0.5, -0.5], [0.0, 1.0, 0.0]),
        face_vertex([-0.5, -0.5, -0.5], [0.0, -1.0, 0.0]),
        face_vertex([0.5, -0.5, -0.5], [0.0, -1.0, 0.0]),
        face_vertex([0.5, -0.5, 0.5], [0.0, -1.0, 0.0]),
        face_vertex([-0.5, -0.5, 0.5], [0.0, -1.0, 0.0]),
    ];
    let mut indices = [0_u16; 36];
    for face in 0..6_u16 {
        let vertex = face * 4;
        let index = usize::from(face) * 6;
        indices[index..index + 6].copy_from_slice(&[
            vertex,
            vertex + 1,
            vertex + 2,
            vertex,
            vertex + 2,
            vertex + 3,
        ]);
    }
    (vertices, indices)
}

const fn face_vertex(position: [f32; 3], normal: [f32; 3]) -> CubeVertex {
    CubeVertex { position, normal }
}
