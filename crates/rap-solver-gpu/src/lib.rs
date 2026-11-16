//! Single-precision MPM solver running in wgpu compute shaders.
//!
//! The scheme is the one of the CPU reference solver, ported to WGSL (see
//! `mpm.wgsl`). The whole state lives on the GPU; it is read back only when
//! a run or step returns, so [`Solver::run_until`] is the efficient way to
//! advance. Arithmetic is `f32`, so results agree with the CPU solver to
//! single-precision accuracy rather than bit for bit.

mod layout;

use std::borrow::Cow;
use std::fmt;
use std::sync::mpsc;

use rap_solver::{Model, Particle, Solver, SolverError};
use wgpu::util::DeviceExt;

use crate::layout::{
    CLOCK_WORDS, Clock, PARTICLE_WORDS, decode_particle, encode_materials, encode_params,
    encode_particles, single,
};

/// Number of time steps encoded per submission while running; the clock is
/// read back between submissions to see whether the end time was reached.
const STEPS_PER_BATCH: u32 = 64;
/// Byte offset of `end_time` in the clock buffer.
const END_TIME_OFFSET: u64 = 12;
/// Bytes per buffer word.
const WORD: u64 = 4;
/// Number of buffers bound to the shader.
const BINDINGS: u32 = 9;

fn gpu_error(error: impl fmt::Display) -> SolverError {
    SolverError::Gpu(error.to_string())
}

/// Number of workgroups of `size` invocations needed to cover `count`.
fn workgroups(count: u32, size: u32) -> u32 {
    count.div_ceil(size).max(1)
}

/// Sizes of a model, as the shader counts them.
#[derive(Debug, Clone, Copy)]
struct Dimensions {
    nodes: [u32; 2],
    node_count: u32,
    body_count: u32,
    particle_count: u32,
}

impl Dimensions {
    fn of(model: &Model) -> Result<Self, SolverError> {
        let [nodes_x, nodes_y] = model.grid.nodes();
        Ok(Self {
            nodes: [
                u32::try_from(nodes_x).map_err(gpu_error)?,
                u32::try_from(nodes_y).map_err(gpu_error)?,
            ],
            node_count: u32::try_from(model.grid.node_count()).map_err(gpu_error)?,
            body_count: u32::try_from(model.body_count()).map_err(gpu_error)?,
            particle_count: u32::try_from(model.seeds.len()).map_err(gpu_error)?,
        })
    }

    fn node_bytes(&self) -> u64 {
        u64::from(self.node_count) * u64::from(self.body_count) * 4 * WORD
    }

    fn particle_bytes(&self) -> u64 {
        u64::from(self.particle_count) * PARTICLE_WORDS as u64 * WORD
    }

    /// Checks that the buffers and dispatches fit the device.
    fn check(&self, limits: &wgpu::Limits) -> Result<(), SolverError> {
        let largest = self.node_bytes().max(self.particle_bytes());
        if largest > u64::from(limits.max_storage_buffer_binding_size)
            || largest > limits.max_buffer_size
        {
            return Err(SolverError::Gpu(format!(
                "the model needs a {largest}-byte buffer, more than this GPU allows; lower the \
                 resolution"
            )));
        }
        let widest = workgroups(self.particle_count.max(self.node_count), 64);
        if widest > limits.max_compute_workgroups_per_dimension {
            return Err(SolverError::Gpu(
                "the model has more particles or nodes than this GPU can dispatch".to_owned(),
            ));
        }
        Ok(())
    }
}

/// One compute pipeline of a time step with the number of workgroups it is
/// dispatched with.
struct Stage {
    pipeline: wgpu::ComputePipeline,
    workgroups: [u32; 2],
}

/// The pipelines of one time step, in execution order.
fn create_stages(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    dimensions: Dimensions,
) -> Vec<Stage> {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rap solver"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mpm.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!("mpm.wgsl").into()),
    });
    let per_particle = [workgroups(dimensions.particle_count, 64), 1];
    let per_node = [workgroups(dimensions.node_count, 64), 1];
    let per_node_2d = [
        workgroups(dimensions.nodes[0], 8),
        workgroups(dimensions.nodes[1], 8),
    ];
    [
        ("prepare", [1, 1]),
        ("clear_bins", per_node),
        ("count_particles", per_particle),
        ("row_prefix", [workgroups(dimensions.nodes[1], 64), 1]),
        ("row_offsets", [1, 1]),
        ("fill_bins", per_particle),
        ("grid", per_node_2d),
        ("particles_update", per_particle),
        ("finish", [1, 1]),
    ]
    .into_iter()
    .map(|(entry_point, workgroups)| Stage {
        pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry_point),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        }),
        workgroups,
    })
    .collect()
}

/// Buffers of a model on the device and the bind group that exposes them
/// to the shader.
struct Resources {
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    particles: wgpu::Buffer,
    particle_staging: wgpu::Buffer,
    clock: wgpu::Buffer,
    clock_staging: wgpu::Buffer,
}

impl Resources {
    fn new(
        device: &wgpu::Device,
        model: &Model,
        dimensions: Dimensions,
    ) -> Result<Self, SolverError> {
        let storage = wgpu::BufferUsages::STORAGE;
        let readable = storage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let staging = wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST;
        let init = |label: &str, words: &[u32], usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(words),
                usage,
            })
        };
        let empty = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let per_node = u64::from(dimensions.node_count) * WORD;
        let per_particle = u64::from(dimensions.particle_count.max(1)) * WORD;

        let params = init("params", &encode_params(model), wgpu::BufferUsages::UNIFORM);
        let materials = init("materials", &encode_materials(model)?, storage);
        let particles = init("particles", &encode_particles(model), readable);
        let nodes = empty("nodes", dimensions.node_bytes(), storage);
        let node_heat = empty("node heat", per_node, storage);
        let bin_count = empty("bin count", per_node, storage);
        // One entry per bin and, after them, one per grid row.
        let bin_start_bytes =
            (u64::from(dimensions.node_count) + u64::from(dimensions.nodes[1])) * WORD;
        let bin_start = empty("bin start", bin_start_bytes, storage);
        let bin_items = empty("bin items", per_particle, storage);
        let clock = init("clock", &Clock::initial(model).encode(), readable);

        // Binding numbers follow the declarations in the shader.
        let bound = [
            &params, &materials, &particles, &nodes, &node_heat, &bin_count, &bin_start,
            &bin_items, &clock,
        ];
        let layout_entries: Vec<_> = (0..BINDINGS)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: match binding {
                        0 => wgpu::BufferBindingType::Uniform,
                        1 => wgpu::BufferBindingType::Storage { read_only: true },
                        _ => wgpu::BufferBindingType::Storage { read_only: false },
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rap solver"),
            entries: &layout_entries,
        });
        let bind_entries: Vec<_> = (0..BINDINGS)
            .zip(bound)
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rap solver"),
            layout: &bind_group_layout,
            entries: &bind_entries,
        });
        Ok(Self {
            bind_group_layout,
            bind_group,
            particle_staging: empty("particle staging", particles.size(), staging),
            particles,
            clock_staging: empty("clock staging", CLOCK_WORDS as u64 * WORD, staging),
            clock,
        })
    }
}

/// Explicit MLS-MPM solver running on the GPU.
pub struct GpuSolver {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    stages: Vec<Stage>,
    resources: Resources,
    time: f64,
    steps: u32,
    particles: Vec<Particle>,
    departed: Vec<Particle>,
}

impl fmt::Debug for GpuSolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuSolver")
            .field("adapter", &self.adapter_name)
            .field("time", &self.time)
            .field("particles", &self.particles.len())
            .field("departed", &self.departed.len())
            .finish_non_exhaustive()
    }
}

impl GpuSolver {
    /// Creates a solver for `model` with every particle in its material's
    /// reference state.
    ///
    /// # Errors
    /// Returns [`SolverError::Model`] if the model is inconsistent,
    /// [`SolverError::NoGpu`] if no suitable graphics adapter exists and
    /// [`SolverError::Gpu`] if the model does not fit the adapter's limits
    /// or uses a material model the GPU solver does not implement.
    pub fn new(model: &Model) -> Result<Self, SolverError> {
        model.validate()?;
        let dimensions = Dimensions::of(model)?;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok_or(SolverError::NoGpu)?;
        let limits = adapter.limits();
        dimensions.check(&limits)?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("rap solver"),
                required_limits: limits,
                ..Default::default()
            },
            None,
        ))
        .map_err(gpu_error)?;

        let resources = Resources::new(&device, model, dimensions)?;
        let stages = create_stages(&device, &resources.bind_group_layout, dimensions);
        let mut solver = Self {
            device,
            queue,
            adapter_name: adapter.get_info().name,
            stages,
            resources,
            time: 0.0,
            steps: 0,
            particles: Vec::new(),
            departed: Vec::new(),
        };
        solver.read_particles()?;
        Ok(solver)
    }

    /// Name of the graphics adapter the solver runs on.
    #[must_use]
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Encodes and submits `count` time steps.
    fn submit_steps(&self, count: u32) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rap steps"),
            });
        for _ in 0..count {
            for stage in &self.stages {
                // One pass per stage: each stage reads what the previous one
                // wrote, and passes are what orders storage-buffer access.
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&stage.pipeline);
                pass.set_bind_group(0, &self.resources.bind_group, &[]);
                pass.dispatch_workgroups(stage.workgroups[0], stage.workgroups[1], 1);
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Copies `source` to `staging` and returns its contents as words.
    fn read_words(
        &self,
        source: &wgpu::Buffer,
        staging: &wgpu::Buffer,
    ) -> Result<Vec<u32>, SolverError> {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(source, 0, staging, 0, source.size());
        self.queue.submit([encoder.finish()]);
        let slice = staging.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver outlives the poll below.
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::Maintain::Wait);
        receiver.recv().map_err(gpu_error)?.map_err(gpu_error)?;
        let words = bytemuck::pod_collect_to_vec(&slice.get_mapped_range());
        staging.unmap();
        Ok(words)
    }

    fn read_clock(&self) -> Result<Clock, SolverError> {
        let words = self.read_words(&self.resources.clock, &self.resources.clock_staging)?;
        Ok(Clock::decode(&words))
    }

    /// Refreshes the CPU copy of the particles from the GPU.
    fn read_particles(&mut self) -> Result<(), SolverError> {
        let words = self.read_words(&self.resources.particles, &self.resources.particle_staging)?;
        self.particles.clear();
        self.departed.clear();
        for chunk in words.chunks_exact(PARTICLE_WORDS) {
            let (particle, status) = decode_particle(chunk);
            match status {
                layout::ACTIVE => self.particles.push(particle),
                layout::DEPARTED => self.departed.push(particle),
                _ => {}
            }
        }
        Ok(())
    }

    /// Runs the simulation on the GPU until its clock reaches `end_time`,
    /// taking at most `max_steps` steps.
    fn advance(&mut self, end_time: f64, max_steps: Option<u32>) -> Result<u32, SolverError> {
        let target = single(end_time);
        self.queue.write_buffer(
            &self.resources.clock,
            END_TIME_OFFSET,
            bytemuck::bytes_of(&target.to_bits()),
        );
        let steps_before = self.steps;
        loop {
            let taken = self.steps - steps_before;
            let batch = max_steps.map_or(STEPS_PER_BATCH, |limit| {
                (limit - taken).min(STEPS_PER_BATCH)
            });
            if batch == 0 {
                break;
            }
            self.submit_steps(batch);
            let clock = self.read_clock()?;
            self.steps = clock.steps;
            let finished = clock.time >= target;
            self.time = if finished {
                end_time
            } else {
                f64::from(clock.time)
            };
            if clock.lost > 0 {
                self.read_particles()?;
                return Err(SolverError::Gpu(format!(
                    "{} particles left the grid at t = {:.3e} s",
                    clock.lost, self.time
                )));
            }
            if finished {
                break;
            }
        }
        self.read_particles()?;
        Ok(self.steps - steps_before)
    }
}

impl Solver for GpuSolver {
    fn time(&self) -> f64 {
        self.time
    }

    fn step(&mut self, max_time_step: f64) -> Result<f64, SolverError> {
        let before = self.time;
        self.advance(before + max_time_step, Some(1))?;
        Ok(self.time - before)
    }

    fn particles(&self) -> Cow<'_, [Particle]> {
        Cow::Borrowed(&self.particles)
    }

    fn departed(&self) -> Cow<'_, [Particle]> {
        Cow::Borrowed(&self.departed)
    }

    fn run_until(&mut self, end_time: f64) -> Result<usize, SolverError> {
        if end_time <= self.time {
            return Ok(0);
        }
        let steps = self.advance(end_time, None)?;
        Ok(usize::try_from(steps).unwrap_or(usize::MAX))
    }
}
