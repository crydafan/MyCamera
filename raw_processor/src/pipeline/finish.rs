use std::slice;

use vulkano::{
    DeviceSize,
    buffer::{Buffer, BufferContents, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{
        AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo, CopyImageToBufferInfo,
        PrimaryAutoCommandBuffer, PrimaryCommandBufferAbstract,
    },
    descriptor_set::{DescriptorSet, WriteDescriptorSet},
    format::Format,
    image::{Image, ImageCreateInfo, ImageUsage, view::ImageView},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter},
    pipeline::{
        ComputePipeline, Pipeline, PipelineBindPoint, PipelineLayout,
        PipelineShaderStageCreateInfo, compute::ComputePipelineCreateInfo,
        layout::PipelineDescriptorSetLayoutCreateInfo,
    },
    sync::{self, GpuFuture},
};

use crate::pipeline::{
    context,
    stage::{StageInPipeline, StageOutput, StageResources},
};

struct Stage0 {
    color_filter_arrangement: i32,

    // Bayer raw image buffer
    buffer: *const u8,
    buffer_len: usize,

    extent: [u32; 3],
}

struct Stage1 {
    color_gains: [f32; 4],

    black_level: [i32; 4],
    white_level: i32,

    extent: [u32; 3],
}

struct Stage2 {
    extent: [u32; 3],
}

struct Stage3 {
    // forward_matrix_1: [f32; 9],
    // forward_matrix_2: [f32; 9],
    color_correction_transform: [f32; 9],
    // neutral_point: [f32; 3],
}

struct Stage4 {}

struct Stage5 {
    extent: [u32; 3],
}

impl StageInPipeline for Stage0 {
    fn create_stage_resources(
        &self,
        context: &context::Context,
        _: Option<StageOutput>,
    ) -> StageResources {
        let (_, raw_image_view) = {
            let buffer = Buffer::new_slice::<u8>(
                context.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_HOST
                        | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                self.buffer_len as DeviceSize,
            )
            .unwrap();

            // Lock subbufer and copy the entire RAW data into it
            buffer
                .write()
                .expect("Failed to lock subbufer for writing")
                .copy_from_slice(unsafe {
                    slice::from_raw_parts(self.buffer, self.buffer_len as usize)
                });

            let image = Image::new(
                context.memory_allocator.clone(),
                ImageCreateInfo {
                    format: Format::R16_UINT,
                    extent: self.extent,
                    usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_DST,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )
            .unwrap();

            let view = ImageView::new_default(image.clone()).unwrap();

            let mut command_buffer_builder = AutoCommandBufferBuilder::primary(
                context.command_buffer_allocator.clone(),
                context.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )
            .unwrap();

            command_buffer_builder
                .copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(buffer, image.clone()))
                .unwrap();

            let command_buffer = command_buffer_builder.build().unwrap();

            command_buffer
                .execute(context.queue.clone())
                .unwrap()
                .then_signal_fence_and_flush()
                .unwrap()
                .wait(None)
                .unwrap();

            (image, view)
        };

        let (_, raw_shifted_image_view) = {
            let image = Image::new(
                context.memory_allocator.clone(),
                ImageCreateInfo {
                    format: Format::R16_UINT,
                    extent: self.extent,
                    usage: ImageUsage::STORAGE,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )
            .unwrap();

            let view = ImageView::new_default(image.clone()).unwrap();

            (image, view)
        };

        mod cs {
            vulkano_shaders::shader! {
                bytes: "shaders/shiftbayer.spv"
            }
        }

        let compute_shader = cs::load(context.device.clone()).unwrap();
        let stage = PipelineShaderStageCreateInfo::new(compute_shader.entry_point("main").unwrap());
        let layout = PipelineLayout::new(
            context.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(context.device.clone())
                .unwrap(),
        )
        .unwrap();

        let compute_pipeline = ComputePipeline::new(
            context.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .expect("Failed to create compute pipeline");

        let layout = compute_pipeline.layout().set_layouts().get(0).unwrap();
        let descriptor_set = DescriptorSet::new(
            context.descriptor_set_allocator.clone(),
            layout.clone(),
            [
                WriteDescriptorSet::image_view(0, raw_image_view.clone()),
                WriteDescriptorSet::image_view(1, raw_shifted_image_view.clone()),
            ],
            [],
        )
        .unwrap();

        StageResources {
            compute_pipeline,
            descriptor_set,
            image_views: vec![raw_image_view, raw_shifted_image_view],
            buffers: vec![],
            commands: vec![],
        }
    }

    fn bind_stage_pipeline_and_dispatch(
        &self,
        command_buffer_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        resources: &StageResources,
        work_groups: [u32; 3],
    ) {
        #[derive(BufferContents)]
        #[repr(C)]
        struct Constants {
            shift_vector: [i32; 2],
        }

        let shift_vector = match self.color_filter_arrangement {
            0 /* RGGB */ => [0, 0],
            1 /* GRBG */ => [1, 0],
            2 /* GBRG */ => [0, 1],
            3 /* BGGR */ => [1, 1],
            _ => [0, 0],
        };

        let constants = Constants { shift_vector };

        command_buffer_builder
            .bind_pipeline_compute(resources.compute_pipeline.clone())
            .unwrap()
            .push_constants(resources.compute_pipeline.layout().clone(), 0, constants)
            .unwrap()
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                resources.compute_pipeline.layout().clone(),
                0,
                resources.descriptor_set.clone(),
            )
            .unwrap();

        unsafe {
            command_buffer_builder.dispatch(work_groups).unwrap();
        }
    }
}

impl StageInPipeline for Stage1 {
    fn create_stage_resources(
        &self,
        context: &context::Context,
        input: Option<StageOutput>,
    ) -> StageResources {
        let (_, raw_normalized_image_view) = {
            let image = Image::new(
                context.memory_allocator.clone(),
                ImageCreateInfo {
                    format: Format::R16_SFLOAT,
                    extent: self.extent,
                    usage: ImageUsage::STORAGE,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )
            .unwrap();

            let view = ImageView::new_default(image.clone()).unwrap();

            (image, view)
        };

        mod cs {
            vulkano_shaders::shader! {
                bytes: "shaders/normalize.spv"
            }
        }

        let compute_shader = cs::load(context.device.clone()).unwrap();
        let stage = PipelineShaderStageCreateInfo::new(compute_shader.entry_point("main").unwrap());
        let layout = PipelineLayout::new(
            context.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(context.device.clone())
                .unwrap(),
        )
        .unwrap();

        let compute_pipeline = ComputePipeline::new(
            context.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .expect("Failed to create compute pipeline");

        let layout = compute_pipeline.layout().set_layouts().get(0).unwrap();
        let descriptor_set = DescriptorSet::new(
            context.descriptor_set_allocator.clone(),
            layout.clone(),
            [
                WriteDescriptorSet::image_view(
                    0,
                    input.unwrap().image_views.get(1).unwrap().clone(),
                ),
                WriteDescriptorSet::image_view(1, raw_normalized_image_view.clone()),
            ],
            [],
        )
        .unwrap();

        StageResources {
            compute_pipeline,
            descriptor_set,
            image_views: vec![raw_normalized_image_view],
            buffers: vec![],
            commands: vec![],
        }
    }

    fn bind_stage_pipeline_and_dispatch(
        &self,
        command_buffer_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        resources: &StageResources,
        work_groups: [u32; 3],
    ) {
        #[derive(BufferContents)]
        #[repr(C)]
        struct Constants {
            color_gains: [f32; 4],
            black_level: [i32; 4],
            white_level: i32,
        }

        let constants = Constants {
            color_gains: self.color_gains,
            white_level: self.white_level,
            black_level: self.black_level,
        };

        command_buffer_builder
            .bind_pipeline_compute(resources.compute_pipeline.clone())
            .unwrap()
            .push_constants(resources.compute_pipeline.layout().clone(), 0, constants)
            .unwrap()
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                resources.compute_pipeline.layout().clone(),
                0,
                resources.descriptor_set.clone(),
            )
            .unwrap();

        unsafe {
            command_buffer_builder.dispatch(work_groups).unwrap();
        }
    }
}

impl StageInPipeline for Stage2 {
    fn create_stage_resources(
        &self,
        context: &context::Context,
        input: Option<StageOutput>,
    ) -> StageResources {
        let (_, rgba_image_view) = {
            let image = Image::new(
                context.memory_allocator.clone(),
                ImageCreateInfo {
                    format: Format::R16G16B16A16_SFLOAT,
                    extent: self.extent,
                    usage: ImageUsage::STORAGE,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )
            .unwrap();

            let view = ImageView::new_default(image.clone()).unwrap();

            (image, view)
        };

        mod cs {
            vulkano_shaders::shader! {
                bytes: "shaders/demosaic.spv"
            }
        }

        let compute_shader = cs::load(context.device.clone()).unwrap();
        let stage = PipelineShaderStageCreateInfo::new(compute_shader.entry_point("main").unwrap());
        let layout = PipelineLayout::new(
            context.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(context.device.clone())
                .unwrap(),
        )
        .unwrap();

        let compute_pipeline = ComputePipeline::new(
            context.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .expect("Failed to create compute pipeline");

        let layout = compute_pipeline.layout().set_layouts().get(0).unwrap();
        let descriptor_set = DescriptorSet::new(
            context.descriptor_set_allocator.clone(),
            layout.clone(),
            [
                WriteDescriptorSet::image_view(
                    0,
                    input.unwrap().image_views.get(0).unwrap().clone(),
                ),
                WriteDescriptorSet::image_view(1, rgba_image_view.clone()),
            ],
            [],
        )
        .unwrap();

        StageResources {
            compute_pipeline,
            descriptor_set,
            image_views: vec![rgba_image_view],
            buffers: vec![],
            commands: vec![],
        }
    }

    fn bind_stage_pipeline_and_dispatch(
        &self,
        command_buffer_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        resources: &StageResources,
        work_groups: [u32; 3],
    ) {
        #[derive(BufferContents)]
        #[repr(C)]
        struct Constants {
            size: [i32; 2],
        }

        let constants = Constants {
            size: [self.extent[0] as i32, self.extent[1] as i32],
        };

        command_buffer_builder
            .bind_pipeline_compute(resources.compute_pipeline.clone())
            .unwrap()
            .push_constants(resources.compute_pipeline.layout().clone(), 0, constants)
            .unwrap()
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                resources.compute_pipeline.layout().clone(),
                0,
                resources.descriptor_set.clone(),
            )
            .unwrap();

        unsafe {
            command_buffer_builder.dispatch(work_groups).unwrap();
        }
    }
}

impl StageInPipeline for Stage3 {
    fn create_stage_resources(
        &self,
        context: &context::Context,
        input: Option<StageOutput>,
    ) -> StageResources {
        mod cs {
            vulkano_shaders::shader! {
                bytes: "shaders/colorcorrection.spv"
            }
        }

        let compute_shader = cs::load(context.device.clone()).unwrap();
        let stage = PipelineShaderStageCreateInfo::new(compute_shader.entry_point("main").unwrap());
        let layout = PipelineLayout::new(
            context.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(context.device.clone())
                .unwrap(),
        )
        .unwrap();

        let compute_pipeline = ComputePipeline::new(
            context.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .expect("Failed to create compute pipeline");

        let layout = compute_pipeline.layout().set_layouts().get(0).unwrap();
        let descriptor_set = DescriptorSet::new(
            context.descriptor_set_allocator.clone(),
            layout.clone(),
            [WriteDescriptorSet::image_view(
                0,
                input.as_ref().unwrap().image_views.get(0).unwrap().clone(),
            )],
            [],
        )
        .unwrap();

        StageResources {
            compute_pipeline,
            descriptor_set,
            image_views: input.unwrap().image_views,
            buffers: vec![],
            commands: vec![],
        }
    }

    fn bind_stage_pipeline_and_dispatch(
        &self,
        command_buffer_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        resources: &StageResources,
        work_groups: [u32; 3],
    ) {
        #[derive(BufferContents)]
        #[repr(C)]
        struct Constants {
            // forward_matrix_1: [[f32; 4]; 3],
            // forward_matrix_2: [[f32; 4]; 3],
            color_correction_transform: [[f32; 4]; 3],
            // neutral_point: [f32; 3],
        }

        let constants = Constants {
            // forward_matrix_1: [
            //     [
            //         self.forward_matrix_1[0],
            //         self.forward_matrix_1[1],
            //         self.forward_matrix_1[2],
            //         0.0, /* padding */
            //     ],
            //     [
            //         self.forward_matrix_1[3],
            //         self.forward_matrix_1[4],
            //         self.forward_matrix_1[5],
            //         0.0, /* padding */
            //     ],
            //     [
            //         self.forward_matrix_1[6],
            //         self.forward_matrix_1[7],
            //         self.forward_matrix_1[8],
            //         0.0, /* padding */
            //     ],
            // ],
            // forward_matrix_2: [
            //     [
            //         self.forward_matrix_2[0],
            //         self.forward_matrix_2[1],
            //         self.forward_matrix_2[2],
            //         0.0, /* padding2*/
            //     ],
            //     [
            //         self.forward_matrix_2[3],
            //         self.forward_matrix_2[4],
            //         self.forward_matrix_2[5],
            //         0.0, /* padding2*/
            //     ],
            //     [
            //         self.forward_matrix_2[6],
            //         self.forward_matrix_2[7],
            //         self.forward_matrix_2[8],
            //         0.0, /* padding */
            //     ],
            // ],
            color_correction_transform: [
                [
                    self.color_correction_transform[0],
                    self.color_correction_transform[1],
                    self.color_correction_transform[2],
                    0.0, /* padding2*/
                ],
                [
                    self.color_correction_transform[3],
                    self.color_correction_transform[4],
                    self.color_correction_transform[5],
                    0.0, /* padding2*/
                ],
                [
                    self.color_correction_transform[6],
                    self.color_correction_transform[7],
                    self.color_correction_transform[8],
                    0.0, /* padding */
                ],
            ],
            // neutral_point: self.neutral_point,
        };

        command_buffer_builder
            .bind_pipeline_compute(resources.compute_pipeline.clone())
            .unwrap()
            .push_constants(resources.compute_pipeline.layout().clone(), 0, constants)
            .unwrap()
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                resources.compute_pipeline.layout().clone(),
                0,
                resources.descriptor_set.clone(),
            )
            .unwrap();

        unsafe {
            command_buffer_builder.dispatch(work_groups).unwrap();
        }
    }
}

impl StageInPipeline for Stage4 {
    fn create_stage_resources(
        &self,
        context: &context::Context,
        input: Option<StageOutput>,
    ) -> StageResources {
        mod cs {
            vulkano_shaders::shader! {
                bytes: "shaders/gammacorrection.spv"
            }
        }

        let compute_shader = cs::load(context.device.clone()).unwrap();
        let stage = PipelineShaderStageCreateInfo::new(compute_shader.entry_point("main").unwrap());
        let layout = PipelineLayout::new(
            context.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(context.device.clone())
                .unwrap(),
        )
        .unwrap();

        let compute_pipeline = ComputePipeline::new(
            context.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .expect("Failed to create compute pipeline");

        let layout = compute_pipeline.layout().set_layouts().get(0).unwrap();
        let descriptor_set = DescriptorSet::new(
            context.descriptor_set_allocator.clone(),
            layout.clone(),
            [WriteDescriptorSet::image_view(
                0,
                input.as_ref().unwrap().image_views.get(0).unwrap().clone(),
            )],
            [],
        )
        .unwrap();

        StageResources {
            compute_pipeline,
            descriptor_set,
            image_views: input.unwrap().image_views,
            buffers: vec![],
            commands: vec![],
        }
    }

    fn bind_stage_pipeline_and_dispatch(
        &self,
        command_buffer_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        resources: &StageResources,
        work_groups: [u32; 3],
    ) {
        command_buffer_builder
            .bind_pipeline_compute(resources.compute_pipeline.clone())
            .unwrap()
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                resources.compute_pipeline.layout().clone(),
                0,
                resources.descriptor_set.clone(),
            )
            .unwrap();

        unsafe {
            command_buffer_builder.dispatch(work_groups).unwrap();
        }
    }
}

impl StageInPipeline for Stage5 {
    fn create_stage_resources(
        &self,
        context: &context::Context,
        input: Option<StageOutput>,
    ) -> StageResources {
        let (_, quantized_image_view, quantized_buffer, copy_quantized_image_to_buffer) = {
            let image = Image::new(
                context.memory_allocator.clone(),
                ImageCreateInfo {
                    format: Format::R8G8B8A8_UNORM,
                    extent: self.extent,
                    usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                AllocationCreateInfo::default(),
            )
            .unwrap();

            let view = ImageView::new_default(image.clone()).unwrap();

            let buffer = Buffer::from_iter(
                context.memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::TRANSFER_DST,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_HOST
                        | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                (0..self.extent[0] * self.extent[1] * 4).map(|_| 0u8),
            )
            .unwrap();

            let mut command_buffer_builder = AutoCommandBufferBuilder::primary(
                context.command_buffer_allocator.clone(),
                context.queue.queue_family_index(),
                CommandBufferUsage::OneTimeSubmit,
            )
            .unwrap();

            command_buffer_builder
                .copy_image_to_buffer(CopyImageToBufferInfo::image_buffer(
                    image.clone(),
                    buffer.clone(),
                ))
                .unwrap();

            let command_buffer = command_buffer_builder.build().unwrap();

            (image, view, buffer, command_buffer)
        };

        mod cs {
            vulkano_shaders::shader! {
                bytes: "shaders/quantize.spv"
            }
        }

        let compute_shader = cs::load(context.device.clone()).unwrap();
        let stage = PipelineShaderStageCreateInfo::new(compute_shader.entry_point("main").unwrap());
        let layout = PipelineLayout::new(
            context.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(context.device.clone())
                .unwrap(),
        )
        .unwrap();

        let compute_pipeline = ComputePipeline::new(
            context.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .expect("Failed to create compute pipeline");

        let layout = compute_pipeline.layout().set_layouts().get(0).unwrap();
        let descriptor_set = DescriptorSet::new(
            context.descriptor_set_allocator.clone(),
            layout.clone(),
            [
                WriteDescriptorSet::image_view(
                    0,
                    input.unwrap().image_views.get(0).unwrap().clone(),
                ),
                WriteDescriptorSet::image_view(1, quantized_image_view.clone()),
            ],
            [],
        )
        .unwrap();

        StageResources {
            compute_pipeline,
            descriptor_set,
            image_views: vec![quantized_image_view],
            buffers: vec![quantized_buffer],
            commands: vec![copy_quantized_image_to_buffer],
        }
    }

    fn bind_stage_pipeline_and_dispatch(
        &self,
        command_buffer_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        resources: &StageResources,
        work_groups: [u32; 3],
    ) {
        command_buffer_builder
            .bind_pipeline_compute(resources.compute_pipeline.clone())
            .unwrap()
            .bind_descriptor_sets(
                PipelineBindPoint::Compute,
                resources.compute_pipeline.layout().clone(),
                0,
                resources.descriptor_set.clone(),
            )
            .unwrap();

        unsafe {
            command_buffer_builder.dispatch(work_groups).unwrap();
        }
    }
}

pub struct Finish {
    output: Option<Subbuffer<[u8]>>,
}

impl Finish {
    pub fn new() -> Finish {
        Finish { output: None }
    }

    pub fn finish(
        &mut self,
        context: &context::Context,
        buffer: *const u8,
        buffer_len: usize,
        size: [i32; 2],
        color_filter_arrangement: i32,
        white_level: i32,
        black_level: [i32; 4],
        neutral_point: [f32; 3],
        color_gains: [f32; 4],
        color_correction_transform: [f32; 9],
        forward_matrix_1: [f32; 9],
        forward_matrix_2: [f32; 9],
    ) {
        let extent = [size[0] as u32, size[1] as u32, 1];

        // Shift Bayer color filter arrangement to match RGGB mosaic pattern
        let stage0 = Stage0 {
            color_filter_arrangement,
            buffer,
            buffer_len,
            extent,
        };

        // Black level subtraction, white balancing and normalization
        let stage1 = Stage1 {
            color_gains,
            black_level,
            white_level,
            extent,
        };

        // Demosaicing
        let stage2 = Stage2 { extent };

        // Color correction (sensor color space to CIE XYZ and then to linear sRGB)
        let stage3 = Stage3 {
            // forward_matrix_1,
            // forward_matrix_2,
            color_correction_transform,
            // neutral_point,
        };

        // Gamma correction
        let stage4 = Stage4 {};

        // Quantization
        let stage5 = Stage5 { extent };

        let stages: Vec<&dyn StageInPipeline> =
            vec![&stage0, &stage1, &stage2, &stage3, &stage4, &stage5];

        let mut command_buffer_builder = AutoCommandBufferBuilder::primary(
            context.command_buffer_allocator.clone(),
            context.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
        .unwrap();

        let work_groups = {
            let w = extent[0];
            let h = extent[1];
            // Rounding up
            [(w + 7) / 8, (h + 7) / 8, 1]
        };

        let mut stage_output: Option<StageOutput> = None;

        for stage in stages {
            let resources = stage.create_stage_resources(&context, stage_output);
            stage.bind_stage_pipeline_and_dispatch(
                &mut command_buffer_builder,
                &resources,
                work_groups,
            );
            stage_output = Some(StageOutput {
                image_views: resources.image_views,
                buffers: resources.buffers,
                commands: resources.commands,
            })
        }

        let command_buffer = command_buffer_builder.build().unwrap();

        sync::now(context.device.clone())
            .then_execute(context.queue.clone(), command_buffer)
            .unwrap()
            .then_signal_fence_and_flush()
            .unwrap()
            .wait(None)
            .unwrap();

        // Copy quantized image to buffer
        if let Some(stage_output) = stage_output {
            stage_output.commands[0]
                .clone()
                .execute(context.queue.clone())
                .unwrap()
                .then_signal_fence_and_flush()
                .unwrap()
                .wait(None)
                .unwrap();

            // Subbufer containts metadata of the GPU buffer
            self.output = stage_output.buffers.get(0).cloned()
        }
    }

    pub fn get_buffer_output(&self) -> Option<Subbuffer<[u8]>> {
        self.output.clone()
    }
}
