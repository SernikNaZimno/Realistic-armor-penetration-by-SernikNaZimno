//! The device side of a solver: buffers, pipelines and the stepping loop.
//! It knows nothing about the number of dimensions; a [`Program`] tells it
//! what to upload and which passes make up a time step.

use std::fmt;
use std::sync::mpsc;

use rap_solver::SolverError;
use wgpu::util::DeviceExt;

use crate::layout::{CLOCK_WORDS, Clock, single};

/// Number of time steps encoded per submission while running; the clock is
/// read back between submissions to see whether the end time was reached.
const STEPS_PER_BATCH: u32 = 64;
/// Byte offset of `end_time` in the clock buffer.
const END_TIME_OFFSET: u64 = 12;
/// Bytes per buffer word.
const WORD: u64 = 4;
/// Number of buffers bound to the shader.
const BINDINGS: u32 = 9;
/// Invocations per workgroup of the passes that run once per particle, per
/// node or per row.
pub(crate) const LINEAR_GROUP: u32 = 64;

pub(crate) fn gpu_error(error: impl fmt::Display) -> SolverError {
    SolverError::Gpu(error.to_string())
}

/// Number of workgroups of `size` invocations needed to cover `count`.
pub(crate) fn workgroups(count: u32, size: u32) -> u32 {
    count.div_ceil(size).max(1)
}

/// A model as the device sees it: the shader, the initial contents of its
/// buffers and the sizes that decide the rest.
pub(crate) struct Program {
    /// Name of the shader, for error messages of the device.
    pub(crate) label: &'static str,
    /// WGSL source.
    pub(crate) source: String,
    /// Contents of the `params` uniform.
    pub(crate) params: Vec<u32>,
    /// Contents of the `materials` buffer.
    pub(crate) materials: Vec<u32>,
    /// Contents of the `particles` buffer.
    pub(crate) particles: Vec<u32>,
    /// Initial clock.
    pub(crate) clock: Clock,
    /// Number of grid nodes, which is also the number of bins.
    pub(crate) node_count: u32,
    /// Number of bodies; each has its own node velocities.
    pub(crate) body_count: u32,
    /// Words per node and body in the `nodes` buffer.
    pub(crate) node_words: u32,
    /// Number of rows the prefix sum over the bins is split into.
    pub(crate) rows: u32,
    /// Number of particles.
    pub(crate) particle_count: u32,
    /// Workgroups of the `grid` pass along each axis.
    pub(crate) grid_workgroups: [u32; 3],
}

impl Program {
    fn node_bytes(&self) -> u64 {
        u64::from(self.node_count) * u64::from(self.body_count) * u64::from(self.node_words) * WORD
    }

    fn particle_bytes(&self) -> u64 {
        self.particles.len() as u64 * WORD
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
        let widest = workgroups(self.particle_count.max(self.node_count), LINEAR_GROUP);
        if widest > limits.max_compute_workgroups_per_dimension {
            return Err(SolverError::Gpu(
                "the model has more particles or nodes than this GPU can dispatch".to_owned(),
            ));
        }
        Ok(())
    }

    /// The entry points of one time step, in execution order, with the
    /// number of workgroups each is dispatched with.
    fn stages(&self) -> [(&'static str, [u32; 3]); 9] {
        let once = [1, 1, 1];
        let linear = |count: u32| [workgroups(count, LINEAR_GROUP), 1, 1];
        [
            ("prepare", once),
            ("clear_bins", linear(self.node_count)),
            ("count_particles", linear(self.particle_count)),
            ("row_prefix", linear(self.rows)),
            ("row_offsets", once),
            ("fill_bins", linear(self.particle_count)),
            ("grid", self.grid_workgroups),
            ("particles_update", linear(self.particle_count)),
            ("finish", once),
        ]
    }
}

/// One compute pipeline of a time step with the number of workgroups it is
/// dispatched with.
struct Stage {
    pipeline: wgpu::ComputePipeline,
    workgroups: [u32; 3],
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
    fn new(device: &wgpu::Device, program: &Program) -> Self {
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
        let per_node = u64::from(program.node_count) * WORD;
        let per_particle = u64::from(program.particle_count.max(1)) * WORD;

        let params = init("params", &program.params, wgpu::BufferUsages::UNIFORM);
        let materials = init("materials", &program.materials, storage);
        let particles = init("particles", &program.particles, readable);
        let nodes = empty("nodes", program.node_bytes(), storage);
        let node_heat = empty("node heat", per_node, storage);
        let bin_count = empty("bin count", per_node, storage);
        // One entry per bin and, after them, one per row.
        let bin_start_bytes = (u64::from(program.node_count) + u64::from(program.rows)) * WORD;
        let bin_start = empty("bin start", bin_start_bytes, storage);
        let bin_items = empty("bin items", per_particle, storage);
        let clock = init("clock", &program.clock.encode(), readable);

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
        Self {
            bind_group_layout,
            bind_group,
            particle_staging: empty("particle staging", particles.size(), staging),
            particles,
            clock_staging: empty("clock staging", CLOCK_WORDS as u64 * WORD, staging),
            clock,
        }
    }
}

/// The pipelines of one time step, in execution order.
fn create_stages(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    program: &Program,
) -> Vec<Stage> {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rap solver"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(program.label),
        source: wgpu::ShaderSource::Wgsl(program.source.as_str().into()),
    });
    program
        .stages()
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

/// A model running on a graphics adapter.
pub(crate) struct Engine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    stages: Vec<Stage>,
    resources: Resources,
    time: f64,
    steps: u32,
}

impl Engine {
    /// Uploads `program` to the most capable adapter.
    ///
    /// # Errors
    /// Returns [`SolverError::NoGpu`] if no suitable graphics adapter
    /// exists and [`SolverError::Gpu`] if the model does not fit the
    /// adapter's limits.
    pub(crate) fn new(program: &Program) -> Result<Self, SolverError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok_or(SolverError::NoGpu)?;
        let limits = adapter.limits();
        program.check(&limits)?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("rap solver"),
                required_limits: limits,
                ..Default::default()
            },
            None,
        ))
        .map_err(gpu_error)?;

        let resources = Resources::new(&device, program);
        let stages = create_stages(&device, &resources.bind_group_layout, program);
        Ok(Self {
            device,
            queue,
            adapter_name: adapter.get_info().name,
            stages,
            resources,
            time: 0.0,
            steps: 0,
        })
    }

    /// Name of the graphics adapter.
    pub(crate) fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Simulation time reached so far, in s.
    pub(crate) fn time(&self) -> f64 {
        self.time
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
                let [x, y, z] = stage.workgroups;
                pass.dispatch_workgroups(x, y, z);
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

    /// The contents of the `particles` buffer.
    ///
    /// # Errors
    /// Returns [`SolverError::Gpu`] if the buffer cannot be read back.
    pub(crate) fn read_particles(&self) -> Result<Vec<u32>, SolverError> {
        self.read_words(&self.resources.particles, &self.resources.particle_staging)
    }

    /// Runs the simulation until its clock reaches `end_time`, taking at
    /// most `max_steps` steps, and returns the number of steps taken.
    ///
    /// # Errors
    /// Returns [`SolverError::Gpu`] if particles left the grid or the
    /// device failed.
    pub(crate) fn advance(
        &mut self,
        end_time: f64,
        max_steps: Option<u32>,
    ) -> Result<u32, SolverError> {
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
                return Err(SolverError::Gpu(format!(
                    "{} particles left the grid at t = {:.3e} s",
                    clock.lost, self.time
                )));
            }
            if finished {
                break;
            }
        }
        Ok(self.steps - steps_before)
    }
}

impl fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("adapter", &self.adapter_name)
            .field("time", &self.time)
            .field("steps", &self.steps)
            .finish_non_exhaustive()
    }
}
