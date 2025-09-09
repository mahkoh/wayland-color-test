use {
    crate::{
        cmm::{ColorMatrix, Lms, Local, NamedTransferFunction, TransferFunction},
        protocols::wayland::wl_surface::WlSurface,
    },
    ash::{
        ext::swapchain_maintenance1,
        khr::{surface, swapchain, wayland_surface},
        vk::{
            self, AccessFlags2, AcquireNextImageInfoKHR, ApplicationInfo, AttachmentLoadOp,
            AttachmentStoreOp, BlendFactor, BlendOp, Buffer, BufferCreateInfo,
            BufferDeviceAddressInfo, BufferMemoryBarrier2, BufferUsageFlags, ClearColorValue,
            ClearValue, ColorComponentFlags, ColorSpaceKHR, CommandBuffer,
            CommandBufferAllocateInfo, CommandBufferBeginInfo, CommandBufferLevel,
            CommandBufferUsageFlags, CommandPool, CommandPoolCreateInfo, CompositeAlphaFlagsKHR,
            DependencyInfo, DeviceCreateInfo, DeviceMemory, DeviceQueueCreateInfo, DynamicState,
            Extent2D, Fence, FenceCreateInfo, Format, GraphicsPipelineCreateInfo, Image,
            ImageAspectFlags, ImageLayout, ImageMemoryBarrier2, ImageSubresourceRange,
            ImageUsageFlags, ImageView, ImageViewCreateInfo, ImageViewType, InstanceCreateInfo,
            PhysicalDevice, PhysicalDeviceSwapchainMaintenance1FeaturesEXT,
            PhysicalDeviceVulkan12Features, PhysicalDeviceVulkan13Features, Pipeline,
            PipelineBindPoint, PipelineCache, PipelineColorBlendAttachmentState,
            PipelineColorBlendStateCreateInfo, PipelineDepthStencilStateCreateInfo,
            PipelineDynamicStateCreateInfo, PipelineInputAssemblyStateCreateInfo, PipelineLayout,
            PipelineLayoutCreateInfo, PipelineMultisampleStateCreateInfo,
            PipelineRasterizationStateCreateInfo, PipelineRenderingCreateInfo,
            PipelineShaderStageCreateInfo, PipelineStageFlags, PipelineStageFlags2,
            PipelineTessellationStateCreateInfo, PipelineVertexInputStateCreateInfo,
            PipelineViewportStateCreateInfo, PresentInfoKHR, PresentModeKHR, PrimitiveTopology,
            PushConstantRange, Queue, Rect2D, RenderingAttachmentInfo, RenderingInfo,
            SampleCountFlags, Semaphore, SemaphoreCreateInfo, ShaderModule, ShaderModuleCreateInfo,
            ShaderStageFlags, SharingMode, SubmitInfo, SurfaceFormatKHR, SurfaceKHR,
            SurfaceTransformFlagsKHR, SwapchainCreateInfoKHR, SwapchainKHR,
            SwapchainPresentFenceInfoEXT, Viewport, WaylandSurfaceCreateInfoKHR,
            EXT_SURFACE_MAINTENANCE1_NAME, EXT_SWAPCHAIN_COLORSPACE_NAME,
            EXT_SWAPCHAIN_MAINTENANCE1_NAME, KHR_GET_SURFACE_CAPABILITIES2_NAME, KHR_SURFACE_NAME,
            KHR_SWAPCHAIN_NAME, KHR_WAYLAND_SURFACE_NAME,
        },
        Device, Entry, Instance,
    },
    bytemuck::{bytes_of, NoUninit},
    gpu_alloc::{AllocationError, Config, GpuAllocator, MemoryBlock, Request, UsageFlags},
    gpu_alloc_ash::AshMemoryDevice,
    itertools::Itertools,
    run_on_drop::on_drop,
    std::{
        cell::{Cell, RefCell, RefMut},
        collections::VecDeque,
        iter,
        ptr::NonNull,
        rc::Rc,
        slice,
    },
    thiserror::Error,
    wl_client::{ffi::wl_display, proxy},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not create an instance")]
    CreateInstance(#[source] vk::Result),
    #[error("could not enumerate physical devices")]
    EnumeratePhysicalDevices(#[source] vk::Result),
    #[error("there are no physical devices")]
    NoPhysicalDevices,
    #[error("physical device has no graphics queues")]
    NoQueues,
    #[error("could not create device")]
    CreateDevice(#[source] vk::Result),
    #[error("could not create wayland surface")]
    CreateWaylandSurface(#[source] vk::Result),
    #[error("could not get the supported surface formats")]
    GetSurfaceFormats(#[source] vk::Result),
    #[error("surface does not support F16 pass through format")]
    F16NotSupported,
    #[error("could not wait for device idle")]
    WaitIdle(#[source] vk::Result),
    #[error("could not create a swapchain")]
    CreateSwapchain(#[source] vk::Result),
    #[error("could not retrieve swapchain images")]
    GetSwapchainImages(#[source] vk::Result),
    #[error("could not create command pool")]
    CreateCommandPool(#[source] vk::Result),
    #[error("could not allocate command buffer")]
    AllocateCommandBuffer(#[source] vk::Result),
    #[error("could not create a fence")]
    CreateFence(#[source] vk::Result),
    #[error("could not get fence status")]
    GetFenceStatus(#[source] vk::Result),
    #[error("could not acquire the next swapchain image")]
    AcquireNextImage(#[source] vk::Result),
    #[error("could not begin command buffer")]
    BeginCommandBuffer(#[source] vk::Result),
    #[error("could not end command buffer")]
    EndCommandBuffer(#[source] vk::Result),
    #[error("could not create image view")]
    CreateImageView(#[source] vk::Result),
    #[error("could not submit command buffer")]
    Submit(#[source] vk::Result),
    #[error("could not create a semaphore")]
    CreateSemaphore(#[source] vk::Result),
    #[error("could not present image")]
    Present(#[source] vk::Result),
    #[error("could not create a shader module")]
    CreateShaderModule(#[source] vk::Result),
    #[error("could not create a pipeline layout")]
    CreatePipelineLayout(#[source] vk::Result),
    #[error("could not create a pipeline")]
    CreateGraphicsPipeline(#[source] vk::Result),
    #[error("could not get gpu_alloc device properties")]
    GpuAllocDeviceProperties(#[source] vk::Result),
    #[error("could not create a buffer")]
    CreateBuffer(#[source] vk::Result),
    #[error("could not allocate buffer memory")]
    AllocateMemory(#[source] AllocationError),
    #[error("could not bind buffer memory")]
    BindBufferMemory(#[source] vk::Result),
}

struct VulkanSwapchain {
    swapchain: SwapchainKHR,
    images: Vec<Image>,
    image_views: Vec<ImageView>,
    width: u32,
    height: u32,
}

struct VulkanSubmission {
    release_fence: Fence,
    acquire_semaphore: Semaphore,
    release_semaphore: Semaphore,
    owns_release_semaphore: Rc<Cell<bool>>,
    command_buffer: CommandBuffer,
    fill_buffers: Vec<FillBuffer>,
}

struct VulkanPresentation {
    release_fence: Fence,
    release_semaphore: Semaphore,
}

pub struct VulkanSurface {
    submissions: RefCell<VecDeque<VulkanSubmission>>,
    presents: RefCell<VecDeque<VulkanPresentation>>,
    swapchain: RefCell<Option<VulkanSwapchain>>,
    suboptimal: Cell<bool>,
    surface: SurfaceKHR,
    fill_buffers: RefCell<Vec<FillBuffer>>,
    device: Rc<VulkanDevice>,
    _wl_surface: WlSurface,
}

pub struct VulkanDevice {
    queue: Queue,
    queue_idx: u32,
    khr_swapchain: swapchain::Device,
    _ext_swapchain_maintenance1: swapchain_maintenance1::Device,
    command_pool: CommandPool,
    pipeline: Pipeline,
    pipeline_layout: PipelineLayout,
    fill_vert: ShaderModule,
    fill_frag: ShaderModule,
    allocator: RefCell<GpuAllocator<DeviceMemory>>,
    device: Device,
    physical_device: PhysicalDevice,
    khr_wayland_surface: wayland_surface::Instance,
    khr_surface: surface::Instance,
    instance: Instance,
}

struct FillBuffer {
    buffer: Buffer,
    addr: u64,
    size: u64,
    memory: Cell<Option<MemoryBlock<DeviceMemory>>>,
    device: Rc<VulkanDevice>,
}

pub enum Scene {
    Fill([f32; 4]),
    FillLeftRight([[f32; 4]; 2]),
    FillTopBottom([[f32; 4]; 2]),
    FillFour([[f32; 4]; 4]),
    CenterBox([[f32; 4]; 2], f32),
    Grid([[f32; 4]; 2], u32, u32),
    BlendLeft([f32; 4]),
    BlendRight([[f32; 4]; 2]),
}

impl Drop for FillBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.device.destroy_buffer(self.buffer, None);
            self.device.allocator.borrow_mut().dealloc(
                AshMemoryDevice::wrap(&self.device.device),
                self.memory.take().unwrap(),
            );
        }
    }
}

impl Drop for VulkanSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device.device_wait_idle();
        }
        let _ = self.gc(true);
        if let Some(sc) = self.swapchain.take() {
            unsafe { sc.destroy(&self.device.device, &self.device.khr_swapchain) }
        }
        self.fill_buffers.borrow_mut().clear();
        unsafe {
            self.device.khr_surface.destroy_surface(self.surface, None);
        }
    }
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_shader_module(self.fill_vert, None);
            self.device.destroy_shader_module(self.fill_frag, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

impl VulkanSwapchain {
    unsafe fn destroy(&self, device: &Device, khr_swapchain: &swapchain::Device) {
        for view in &self.image_views {
            unsafe {
                device.destroy_image_view(*view, None);
            }
        }
        unsafe {
            khr_swapchain.destroy_swapchain(self.swapchain, None);
        }
    }
}

impl VulkanDevice {
    pub fn create() -> Result<Rc<Self>, Error> {
        let entry = Entry::linked();
        let app_info = ApplicationInfo::default()
            .api_version(vk::API_VERSION_1_3)
            .application_name(c"wayland-color-test");
        let extensions = [
            KHR_SURFACE_NAME.as_ptr(),
            EXT_SURFACE_MAINTENANCE1_NAME.as_ptr(),
            KHR_GET_SURFACE_CAPABILITIES2_NAME.as_ptr(),
            KHR_WAYLAND_SURFACE_NAME.as_ptr(),
            EXT_SWAPCHAIN_COLORSPACE_NAME.as_ptr(),
        ];
        let create_info = InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extensions);
        let instance = unsafe {
            entry
                .create_instance(&create_info, None)
                .map_err(Error::CreateInstance)?
        };
        let destroy_instance = on_drop(|| unsafe { instance.destroy_instance(None) });
        let khr_surface = surface::Instance::new(&entry, &instance);
        let khr_wayland_surface = wayland_surface::Instance::new(&entry, &instance);
        let physical_devices = unsafe {
            instance
                .enumerate_physical_devices()
                .map_err(Error::EnumeratePhysicalDevices)?
        };
        if physical_devices.is_empty() {
            return Err(Error::NoPhysicalDevices);
        }
        let physical_device = physical_devices[0];
        let queues =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let queue_idx = 'queue: {
            for (idx, queue) in queues.into_iter().enumerate() {
                if queue.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    break 'queue idx as u32;
                }
            }
            return Err(Error::NoQueues);
        };
        let queue_create_info = DeviceQueueCreateInfo::default()
            .queue_family_index(queue_idx)
            .queue_priorities(&[0.0]);
        let extensions = [
            KHR_SWAPCHAIN_NAME.as_ptr(),
            EXT_SWAPCHAIN_MAINTENANCE1_NAME.as_ptr(),
        ];
        let mut device_features12 =
            PhysicalDeviceVulkan12Features::default().buffer_device_address(true);
        let mut device_features13 = PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true);
        let mut swapchain_maintenance1_features =
            PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default().swapchain_maintenance1(true);
        let create_info = DeviceCreateInfo::default()
            .queue_create_infos(slice::from_ref(&queue_create_info))
            .enabled_extension_names(&extensions)
            .push_next(&mut device_features12)
            .push_next(&mut device_features13)
            .push_next(&mut swapchain_maintenance1_features);
        let device = unsafe {
            instance
                .create_device(physical_device, &create_info, None)
                .map_err(Error::CreateDevice)?
        };
        let destroy_device = on_drop(|| unsafe { device.destroy_device(None) });
        let allocator = {
            let device_properties = unsafe {
                gpu_alloc_ash::device_properties(&instance, vk::API_VERSION_1_3, physical_device)
                    .map_err(Error::GpuAllocDeviceProperties)?
            };
            GpuAllocator::new(Config::i_am_prototyping(), device_properties)
        };
        let queue = unsafe { device.get_device_queue(queue_idx, 0) };
        let khr_swapchain = swapchain::Device::new(&instance, &device);
        let ext_swapchain_maintenance1 = swapchain_maintenance1::Device::new(&instance, &device);
        let create_info = CommandPoolCreateInfo::default().queue_family_index(queue_idx);
        let command_pool = unsafe {
            device
                .create_command_pool(&create_info, None)
                .map_err(Error::CreateCommandPool)?
        };
        let destroy_command_pool =
            on_drop(|| unsafe { device.destroy_command_pool(command_pool, None) });
        const FILL_VERT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fill.vert.spv"));
        const FILL_FRAG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fill.frag.spv"));
        let create_shader = |bytes: &[u8]| {
            let mut iter = bytes.iter().copied();
            let code: Vec<_> = iter::from_fn(|| iter.next_array::<4>())
                .map(u32::from_ne_bytes)
                .collect();
            let create_info = ShaderModuleCreateInfo::default().code(&code);
            unsafe {
                device
                    .create_shader_module(&create_info, None)
                    .map_err(Error::CreateShaderModule)
            }
        };
        let fill_vert = create_shader(FILL_VERT)?;
        let destroy_fill_vert =
            on_drop(|| unsafe { device.destroy_shader_module(fill_vert, None) });
        let fill_frag = create_shader(FILL_FRAG)?;
        let destroy_fill_frag =
            on_drop(|| unsafe { device.destroy_shader_module(fill_frag, None) });
        let pipeline_layout = {
            let range = PushConstantRange::default()
                .size(size_of::<FillPushConstant>() as _)
                .stage_flags(ShaderStageFlags::FRAGMENT | ShaderStageFlags::VERTEX);
            let create_info =
                PipelineLayoutCreateInfo::default().push_constant_ranges(slice::from_ref(&range));
            unsafe {
                device
                    .create_pipeline_layout(&create_info, None)
                    .map_err(Error::CreatePipelineLayout)?
            }
        };
        let destroy_pipeline_layout =
            on_drop(|| unsafe { device.destroy_pipeline_layout(pipeline_layout, None) });
        let pipeline = {
            let stages = [
                PipelineShaderStageCreateInfo::default()
                    .stage(ShaderStageFlags::VERTEX)
                    .name(c"main")
                    .module(fill_vert),
                PipelineShaderStageCreateInfo::default()
                    .stage(ShaderStageFlags::FRAGMENT)
                    .name(c"main")
                    .module(fill_frag),
            ];
            let vertex_input_state = PipelineVertexInputStateCreateInfo::default();
            let input_assembly_state = PipelineInputAssemblyStateCreateInfo::default()
                .topology(PrimitiveTopology::TRIANGLE_STRIP);
            let tessellation_state = PipelineTessellationStateCreateInfo::default();
            let rasterization_state =
                PipelineRasterizationStateCreateInfo::default().line_width(1.0);
            let multisample_state = PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(SampleCountFlags::TYPE_1);
            let depth_stencil_state = PipelineDepthStencilStateCreateInfo::default();
            let viewport_state = PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1);
            let color_blend_attachment_state = PipelineColorBlendAttachmentState::default()
                .blend_enable(true)
                .src_color_blend_factor(BlendFactor::SRC_ALPHA)
                .dst_color_blend_factor(BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(BlendOp::ADD)
                .src_alpha_blend_factor(BlendFactor::ONE)
                .dst_alpha_blend_factor(BlendFactor::ONE_MINUS_SRC_ALPHA)
                .alpha_blend_op(BlendOp::ADD)
                .color_write_mask(ColorComponentFlags::RGBA);
            let color_blend_state = PipelineColorBlendStateCreateInfo::default()
                .attachments(slice::from_ref(&color_blend_attachment_state));
            let dynamic_states = [DynamicState::VIEWPORT, DynamicState::SCISSOR];
            let dynamic_state =
                PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
            let mut rendering_create_info = PipelineRenderingCreateInfo::default()
                .color_attachment_formats(&[Format::R16G16B16A16_SFLOAT]);
            let create_info = GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input_state)
                .input_assembly_state(&input_assembly_state)
                .tessellation_state(&tessellation_state)
                .rasterization_state(&rasterization_state)
                .multisample_state(&multisample_state)
                .depth_stencil_state(&depth_stencil_state)
                .color_blend_state(&color_blend_state)
                .viewport_state(&viewport_state)
                .dynamic_state(&dynamic_state)
                .layout(pipeline_layout)
                .push_next(&mut rendering_create_info);
            let pipeline = unsafe {
                device
                    .create_graphics_pipelines(
                        PipelineCache::null(),
                        slice::from_ref(&create_info),
                        None,
                    )
                    .map_err(|(_, e)| Error::CreateGraphicsPipeline(e))?
            };
            assert_eq!(pipeline.len(), 1);
            pipeline[0]
        };
        let destroy_pipeline = on_drop(|| unsafe { device.destroy_pipeline(pipeline, None) });
        destroy_pipeline.forget();
        destroy_pipeline_layout.forget();
        destroy_fill_frag.forget();
        destroy_fill_vert.forget();
        destroy_command_pool.forget();
        destroy_device.forget();
        destroy_instance.forget();
        Ok(Rc::new(VulkanDevice {
            queue,
            queue_idx,
            khr_swapchain,
            _ext_swapchain_maintenance1: ext_swapchain_maintenance1,
            command_pool,
            pipeline,
            pipeline_layout,
            fill_vert,
            fill_frag,
            allocator: RefCell::new(allocator),
            physical_device,
            device,
            khr_wayland_surface,
            khr_surface,
            instance,
        }))
    }

    pub fn create_surface(
        self: &Rc<Self>,
        wl_display: NonNull<wl_display>,
        wl_surface: &WlSurface,
    ) -> Result<VulkanSurface, Error> {
        let create_info = WaylandSurfaceCreateInfoKHR::default()
            .display(wl_display.as_ptr().cast())
            .surface(proxy::wl_proxy(&**wl_surface).unwrap().cast().as_ptr());
        let surface = unsafe {
            self.khr_wayland_surface
                .create_wayland_surface(&create_info, None)
                .map_err(Error::CreateWaylandSurface)?
        };
        let destroy_surface =
            on_drop(|| unsafe { self.khr_surface.destroy_surface(surface, None) });
        let formats = unsafe {
            self.khr_surface
                .get_physical_device_surface_formats(self.physical_device, surface)
                .map_err(Error::GetSurfaceFormats)?
        };
        let supports_format = formats.contains(&SurfaceFormatKHR {
            format: Format::R16G16B16A16_SFLOAT,
            color_space: ColorSpaceKHR::PASS_THROUGH_EXT,
        });
        if !supports_format {
            return Err(Error::F16NotSupported);
        };
        destroy_surface.forget();
        Ok(VulkanSurface {
            submissions: Default::default(),
            presents: Default::default(),
            swapchain: Default::default(),
            suboptimal: Default::default(),
            surface,
            fill_buffers: Default::default(),
            device: self.clone(),
            _wl_surface: wl_surface.clone(),
        })
    }
}

impl VulkanSurface {
    fn ensure_swapchain(
        &self,
        width: u32,
        height: u32,
    ) -> Result<RefMut<'_, VulkanSwapchain>, Error> {
        let mut sc = self.swapchain.borrow_mut();
        let mut recreate = false;
        if !recreate {
            recreate = sc.is_none();
        }
        if !recreate {
            recreate = self.suboptimal.get();
        }
        if !recreate {
            if let Some(sc) = &*sc {
                if sc.width != width || sc.height != height {
                    recreate = true;
                }
            }
        }
        if recreate {
            let old = sc.take();
            if old.is_some() {
                unsafe {
                    self.device
                        .device
                        .device_wait_idle()
                        .map_err(Error::WaitIdle)?;
                }
            }
            let create_info = SwapchainCreateInfoKHR::default()
                .surface(self.surface)
                .pre_transform(SurfaceTransformFlagsKHR::IDENTITY)
                .composite_alpha(CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
                .image_extent(Extent2D { width, height })
                .min_image_count(3)
                .image_format(Format::R16G16B16A16_SFLOAT)
                .image_color_space(ColorSpaceKHR::PASS_THROUGH_EXT)
                .image_array_layers(1)
                .image_usage(ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(SharingMode::EXCLUSIVE)
                .present_mode(PresentModeKHR::MAILBOX)
                .clipped(true)
                .old_swapchain(old.as_ref().map(|o| o.swapchain).unwrap_or_default());
            let swapchain = unsafe {
                self.device
                    .khr_swapchain
                    .create_swapchain(&create_info, None)
                    .map_err(Error::CreateSwapchain)?
            };
            if let Some(sc) = old {
                unsafe {
                    sc.destroy(&self.device.device, &self.device.khr_swapchain);
                }
            }
            let destroy_swapchain =
                on_drop(|| unsafe { self.device.khr_swapchain.destroy_swapchain(swapchain, None) });
            let images = unsafe {
                self.device
                    .khr_swapchain
                    .get_swapchain_images(swapchain)
                    .map_err(Error::GetSwapchainImages)?
            };
            let mut image_views = vec![];
            let mut destroy_image_views = vec![];
            for image in &images {
                let create_info = ImageViewCreateInfo::default()
                    .image(*image)
                    .view_type(ImageViewType::TYPE_2D)
                    .format(Format::R16G16B16A16_SFLOAT)
                    .subresource_range(IMAGE_SUBRESOURCE_RANGE);
                let view = unsafe {
                    self.device
                        .device
                        .create_image_view(&create_info, None)
                        .map_err(Error::CreateImageView)?
                };
                image_views.push(view);
                destroy_image_views.push(on_drop(move || unsafe {
                    self.device.device.destroy_image_view(view, None);
                }));
            }
            destroy_image_views.into_iter().for_each(|d| d.forget());
            destroy_swapchain.forget();
            *sc = Some(VulkanSwapchain {
                swapchain,
                images,
                image_views,
                width,
                height,
            });
            self.suboptimal.set(false);
        }
        Ok(RefMut::map(sc, |sc| sc.as_mut().unwrap()))
    }

    fn gc(&self, force: bool) -> Result<(), Error> {
        let dev = &self.device.device;
        let submissions = &mut *self.submissions.borrow_mut();
        while let Some(first) = submissions.front_mut() {
            let done = unsafe {
                dev.get_fence_status(first.release_fence)
                    .map_err(Error::GetFenceStatus)?
            };
            if !done && !force {
                break;
            }
            unsafe {
                dev.free_command_buffers(self.device.command_pool, &[first.command_buffer]);
            }
            unsafe {
                dev.destroy_semaphore(first.acquire_semaphore, None);
            }
            if first.owns_release_semaphore.get() {
                unsafe {
                    dev.destroy_semaphore(first.release_semaphore, None);
                }
            }
            unsafe {
                dev.destroy_fence(first.release_fence, None);
            }
            for buffer in first.fill_buffers.drain(..) {
                self.fill_buffers.borrow_mut().push(buffer);
            }
            submissions.pop_front();
        }
        let presents = &mut *self.presents.borrow_mut();
        while let Some(first) = presents.front() {
            let done = unsafe {
                dev.get_fence_status(first.release_fence)
                    .map_err(Error::GetFenceStatus)?
            };
            if !done && !force {
                break;
            }
            unsafe {
                dev.destroy_semaphore(first.release_semaphore, None);
            }
            unsafe {
                dev.destroy_fence(first.release_fence, None);
            }
            presents.pop_front();
        }
        Ok(())
    }

    fn get_command_buffer(&self) -> Result<CommandBuffer, Error> {
        let allocate_info = CommandBufferAllocateInfo::default()
            .command_pool(self.device.command_pool)
            .level(CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let buffers = unsafe {
            self.device
                .device
                .allocate_command_buffers(&allocate_info)
                .map_err(Error::AllocateCommandBuffer)?
        };
        assert_eq!(buffers.len(), 1);
        Ok(buffers[0])
    }

    pub fn render(
        &self,
        width: u32,
        height: u32,
        scene: Scene,
        lms_to_local: ColorMatrix<Local, Lms>,
        tf: TransferFunction,
        tf_args: [f32; 4],
    ) -> Result<(), Error> {
        self.gc(false)?;
        let dev = &self.device.device;
        let swapchain = self.ensure_swapchain(width, height)?;
        let create_semaphore = || {
            let create_info = SemaphoreCreateInfo::default();
            unsafe {
                dev.create_semaphore(&create_info, None)
                    .map_err(Error::CreateSemaphore)
            }
        };
        let acquire_semaphore = create_semaphore()?;
        let destroy_acquire_semaphore =
            on_drop(|| unsafe { dev.destroy_semaphore(acquire_semaphore, None) });
        let release_semaphore = create_semaphore()?;
        let destroy_release_semaphore =
            on_drop(|| unsafe { dev.destroy_semaphore(release_semaphore, None) });
        let create_fence = || {
            let create_info = FenceCreateInfo::default();
            unsafe {
                dev.create_fence(&create_info, None)
                    .map_err(Error::CreateFence)
            }
        };
        let queue_release_fence = create_fence()?;
        let destroy_queue_release_fence =
            on_drop(|| unsafe { dev.destroy_fence(queue_release_fence, None) });
        let present_release_fence = create_fence()?;
        let destroy_present_release_fence =
            on_drop(|| unsafe { dev.destroy_fence(present_release_fence, None) });
        let (image, suboptimal) = {
            let acquire_info = AcquireNextImageInfoKHR::default()
                .device_mask(1)
                .swapchain(swapchain.swapchain)
                .timeout(u64::MAX)
                .semaphore(acquire_semaphore);
            unsafe {
                self.device
                    .khr_swapchain
                    .acquire_next_image2(&acquire_info)
                    .map_err(Error::AcquireNextImage)?
            }
        };
        if suboptimal {
            self.suboptimal.set(true);
        }
        let buffer = self.get_command_buffer()?;
        let free_buffer =
            on_drop(|| unsafe { dev.free_command_buffers(self.device.command_pool, &[buffer]) });
        {
            let begin_info =
                CommandBufferBeginInfo::default().flags(CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            unsafe {
                dev.begin_command_buffer(buffer, &begin_info)
                    .map_err(Error::BeginCommandBuffer)?;
            }
        }
        struct Op {
            fill: FillBuffer,
        }
        let mut ops = vec![];
        let lms_to_local = lms_to_local.to_f32();
        let eotf = match tf {
            TransferFunction::Named(n) => match n {
                NamedTransferFunction::Srgb => 4,
                NamedTransferFunction::Linear => 1,
                NamedTransferFunction::St2084Pq => 2,
                NamedTransferFunction::Bt1886 => 3,
                NamedTransferFunction::Gamma22 => 4,
                NamedTransferFunction::Gamma28 => 5,
                NamedTransferFunction::St240 => 6,
                NamedTransferFunction::ExtSrgb => 4,
                NamedTransferFunction::Log100 => 8,
                NamedTransferFunction::Log316 => 9,
                NamedTransferFunction::St428 => 10,
            },
            TransferFunction::Pow => 11,
        };
        let mut fill = |x1: f32, y1: f32, x2: f32, y2: f32, color: [[f32; 4]; 4]| {
            let fill = self.allocate_fill_buffer()?;
            let data = FillData {
                lms_to_local,
                x1,
                y1,
                x2,
                y2,
                color,
                eotf,
                eotf_args: tf_args,
            };
            unsafe {
                dev.cmd_update_buffer(buffer, fill.buffer, 0, bytes_of(&data));
            }
            ops.push(Op { fill });
            Ok(())
        };
        match scene {
            Scene::Fill(c) => {
                fill(-1.0, -1.0, 1.0, 1.0, [lch_to_lab(c); 4])?;
            }
            Scene::FillLeftRight([l, r]) => {
                fill(
                    -1.0,
                    -1.0,
                    1.0,
                    1.0,
                    [lch_to_lab(r), lch_to_lab(l), lch_to_lab(r), lch_to_lab(l)],
                )?;
            }
            Scene::FillTopBottom([t, b]) => {
                fill(
                    -1.0,
                    -1.0,
                    1.0,
                    1.0,
                    [lch_to_lab(t), lch_to_lab(t), lch_to_lab(b), lch_to_lab(b)],
                )?;
            }
            Scene::FillFour(c) => {
                fill(-1.0, -1.0, 1.0, 1.0, c.map(lch_to_lab))?;
            }
            Scene::CenterBox(c, size) => {
                fill(-1.0, -1.0, 1.0, 1.0, [lch_to_lab(c[0]); 4])?;
                fill(-size, -size, size, size, [lch_to_lab(c[1]); 4])?;
            }
            Scene::Grid(c, rows, cols) => {
                fill(-1.0, -1.0, 1.0, 1.0, [lch_to_lab(c[0]); 4])?;
                let c1 = [lch_to_lab(c[1]); 4];
                let height = 2.0 / rows as f32;
                let width = 2.0 / cols as f32;
                for row in 0..rows {
                    let y1 = -1.0 + height * row as f32;
                    for col in 0..cols {
                        if (row + col) % 2 == 0 {
                            continue;
                        }
                        let x1 = -1.0 + width * col as f32;
                        fill(x1, y1, x1 + width, y1 + height, c1)?;
                    }
                }
            }
            Scene::BlendLeft(c) => {
                fill(-1.0, 0.0, 1.0, 1.0, [lch_to_lab(c); 4])?;
            }
            Scene::BlendRight(c) => {
                let mut b = lch_to_lab(c[0]);
                let mut f = lch_to_lab(c[1]);
                let mut r = f;
                let a = f[3];
                for i in 0..3 {
                    r[i] = r[i] * a + (1.0 - a) * b[i];
                }
                r[3] = 1.0;
                b[3] = 1.0;
                f[3] = 1.0;
                fill(-1.0, -1.0, 0.0, 1.0, [b; 4])?;
                fill(0.0, -1.0, 1.0, 0.0, [f; 4])?;
                fill(0.0, 0.0, 1.0, 1.0, [r; 4])?;
            }
        }
        {
            let image_barrier = ImageMemoryBarrier2::default()
                .src_stage_mask(PipelineStageFlags2::BOTTOM_OF_PIPE)
                .dst_stage_mask(PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(ImageLayout::UNDEFINED)
                .new_layout(ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(swapchain.images[image as usize])
                .subresource_range(IMAGE_SUBRESOURCE_RANGE);
            let mut buffer_barriers = vec![];
            for op in &ops {
                let buffer_barrier = BufferMemoryBarrier2::default()
                    .src_stage_mask(PipelineStageFlags2::TRANSFER)
                    .src_access_mask(AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(
                        PipelineStageFlags2::VERTEX_SHADER | PipelineStageFlags2::FRAGMENT_SHADER,
                    )
                    .dst_access_mask(AccessFlags2::SHADER_READ)
                    .buffer(op.fill.buffer)
                    .size(op.fill.size);
                buffer_barriers.push(buffer_barrier);
            }
            let dependency_info = DependencyInfo::default()
                .image_memory_barriers(slice::from_ref(&image_barrier))
                .buffer_memory_barriers(&buffer_barriers);
            unsafe {
                self.device
                    .device
                    .cmd_pipeline_barrier2(buffer, &dependency_info);
            }
        }
        {
            let attachment_info = RenderingAttachmentInfo::default()
                .image_view(swapchain.image_views[image as usize])
                .image_layout(ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(AttachmentLoadOp::CLEAR)
                .store_op(AttachmentStoreOp::STORE)
                .clear_value(ClearValue {
                    color: ClearColorValue {
                        float32: [0.0, 0.0, 0.0, 0.0],
                    },
                });
            let rendering_info = RenderingInfo::default()
                .render_area(Rect2D {
                    offset: Default::default(),
                    extent: Extent2D { width, height },
                })
                .layer_count(1)
                .color_attachments(slice::from_ref(&attachment_info));
            unsafe {
                dev.cmd_begin_rendering(buffer, &rendering_info);
            }
        }
        {
            let viewport = Viewport {
                x: 0.0,
                y: 0.0,
                width: width as _,
                height: height as _,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            unsafe {
                dev.cmd_set_viewport(buffer, 0, slice::from_ref(&viewport));
            }
            let scissor = Rect2D {
                offset: Default::default(),
                extent: Extent2D { width, height },
            };
            unsafe {
                dev.cmd_set_scissor(buffer, 0, slice::from_ref(&scissor));
            }
        }
        {
            let fill = |addr: u64| {
                unsafe {
                    dev.cmd_bind_pipeline(
                        buffer,
                        PipelineBindPoint::GRAPHICS,
                        self.device.pipeline,
                    );
                }
                let constants = FillPushConstant { data: addr };
                unsafe {
                    dev.cmd_push_constants(
                        buffer,
                        self.device.pipeline_layout,
                        ShaderStageFlags::VERTEX | ShaderStageFlags::FRAGMENT,
                        0,
                        bytes_of(&constants),
                    );
                }
                unsafe {
                    dev.cmd_draw(buffer, 4, 1, 0, 0);
                }
            };
            for op in &ops {
                fill(op.fill.addr);
            }
        }
        unsafe {
            dev.cmd_end_rendering(buffer);
        }
        {
            let image_barrier = ImageMemoryBarrier2::default()
                .src_stage_mask(PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(PipelineStageFlags2::BOTTOM_OF_PIPE)
                .old_layout(ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(ImageLayout::PRESENT_SRC_KHR)
                .image(swapchain.images[image as usize])
                .subresource_range(IMAGE_SUBRESOURCE_RANGE);
            let dependency_info =
                DependencyInfo::default().image_memory_barriers(slice::from_ref(&image_barrier));
            unsafe {
                dev.cmd_pipeline_barrier2(buffer, &dependency_info);
            }
        }
        unsafe {
            dev.end_command_buffer(buffer)
                .map_err(Error::EndCommandBuffer)?;
        }
        {
            let submit_info = SubmitInfo::default()
                .wait_semaphores(slice::from_ref(&acquire_semaphore))
                .wait_dst_stage_mask(&[PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT])
                .signal_semaphores(slice::from_ref(&release_semaphore))
                .command_buffers(slice::from_ref(&buffer));
            unsafe {
                dev.queue_submit(self.device.queue, &[submit_info], queue_release_fence)
                    .map_err(Error::Submit)?;
            }
        }
        destroy_acquire_semaphore.forget();
        destroy_release_semaphore.forget();
        destroy_queue_release_fence.forget();
        free_buffer.forget();
        let owns_release_semaphore = Rc::new(Cell::new(true));
        self.submissions.borrow_mut().push_back(VulkanSubmission {
            acquire_semaphore,
            release_semaphore,
            owns_release_semaphore: owns_release_semaphore.clone(),
            release_fence: queue_release_fence,
            command_buffer: buffer,
            fill_buffers: ops.into_iter().map(|op| op.fill).collect(),
        });
        let suboptimal = {
            let mut fence_info = SwapchainPresentFenceInfoEXT::default()
                .fences(slice::from_ref(&present_release_fence));
            let present_info = PresentInfoKHR::default()
                .wait_semaphores(slice::from_ref(&release_semaphore))
                .swapchains(slice::from_ref(&swapchain.swapchain))
                .image_indices(slice::from_ref(&image))
                .push_next(&mut fence_info);
            unsafe {
                self.device
                    .khr_swapchain
                    .queue_present(self.device.queue, &present_info)
                    .map_err(Error::Present)?
            }
        };
        if suboptimal {
            self.suboptimal.set(true);
        }
        owns_release_semaphore.set(false);
        destroy_present_release_fence.forget();
        self.presents.borrow_mut().push_back(VulkanPresentation {
            release_fence: present_release_fence,
            release_semaphore,
        });
        Ok(())
    }

    fn allocate_fill_buffer(&self) -> Result<FillBuffer, Error> {
        let buffers = &mut *self.fill_buffers.borrow_mut();
        if let Some(buffer) = buffers.pop() {
            return Ok(buffer);
        }
        let size = size_of::<FillData>().next_multiple_of(16) as u64;
        let create_info = BufferCreateInfo::default()
            .size(size)
            .usage(BufferUsageFlags::TRANSFER_DST | BufferUsageFlags::SHADER_DEVICE_ADDRESS)
            .queue_family_indices(slice::from_ref(&self.device.queue_idx));
        let buffer = unsafe {
            self.device
                .device
                .create_buffer(&create_info, None)
                .map_err(Error::CreateBuffer)?
        };
        let destroy_buffer = on_drop(|| unsafe { self.device.device.destroy_buffer(buffer, None) });
        let req = unsafe { self.device.device.get_buffer_memory_requirements(buffer) };
        let request = Request {
            size: req.size,
            align_mask: req.alignment - 1,
            usage: UsageFlags::FAST_DEVICE_ACCESS | UsageFlags::DEVICE_ADDRESS,
            memory_types: req.memory_type_bits,
        };
        let alloc = unsafe {
            self.device
                .allocator
                .borrow_mut()
                .alloc(AshMemoryDevice::wrap(&self.device.device), request)
                .map_err(Error::AllocateMemory)?
        };
        let memory = *alloc.memory();
        let offset = alloc.offset();
        let alloc = Cell::new(Some(alloc));
        let dealloc_memory = on_drop(|| unsafe {
            self.device.allocator.borrow_mut().dealloc(
                AshMemoryDevice::wrap(&self.device.device),
                alloc.take().unwrap(),
            );
        });
        unsafe {
            self.device
                .device
                .bind_buffer_memory(buffer, memory, offset)
                .map_err(Error::BindBufferMemory)?;
        }
        let addr = {
            let info = BufferDeviceAddressInfo::default().buffer(buffer);
            unsafe { self.device.device.get_buffer_device_address(&info) }
        };
        dealloc_memory.forget();
        destroy_buffer.forget();
        Ok(FillBuffer {
            buffer,
            addr,
            size,
            memory: alloc,
            device: self.device.clone(),
        })
    }
}

const IMAGE_SUBRESOURCE_RANGE: ImageSubresourceRange = ImageSubresourceRange {
    aspect_mask: ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

#[derive(NoUninit, Copy, Clone)]
#[repr(C)]
struct FillData {
    lms_to_local: [[f32; 4]; 4],
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: [[f32; 4]; 4],
    eotf: u32,
    eotf_args: [f32; 4],
}

#[derive(NoUninit, Copy, Clone)]
#[repr(C)]
struct FillPushConstant {
    data: u64,
}

fn lch_to_lab(mut lch: [f32; 4]) -> [f32; 4] {
    let a = lch[1] * lch[2].cos();
    let b = lch[1] * lch[2].sin();
    lch[1] = a;
    lch[2] = b;
    lch
}
