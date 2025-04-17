use {
    crate::{
        cmm::{
            matrix_from_lms, ColorMatrix, Lms, Local, Luminance, NamedPrimaries, Primaries,
            TransferFunction,
        },
        ordered_float::F64,
        protocols::{
            color_management_v1::{
                wp_color_management_surface_v1::WpColorManagementSurfaceV1,
                wp_color_manager_v1::{
                    WpColorManagerV1, WpColorManagerV1EventHandler, WpColorManagerV1Feature,
                    WpColorManagerV1Primaries, WpColorManagerV1Ref, WpColorManagerV1RenderIntent,
                    WpColorManagerV1TransferFunction,
                },
                wp_image_description_v1::{
                    WpImageDescriptionV1, WpImageDescriptionV1Cause,
                    WpImageDescriptionV1EventHandler, WpImageDescriptionV1Ref,
                },
            },
            wayland::{
                wl_compositor::WlCompositor, wl_display::WlDisplay,
                wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface,
                wl_surface::WlSurface,
            },
            xdg_shell::{
                xdg_surface::{XdgSurface, XdgSurfaceEventHandler, XdgSurfaceRef},
                xdg_toplevel::{XdgToplevel, XdgToplevelEventHandler, XdgToplevelRef},
                xdg_wm_base::XdgWmBase,
            },
        },
        singletons::get_singletons,
        vulkan::{Scene, VulkanDevice, VulkanSurface},
    },
    egui_winit::winit::{
        event_loop::{EventLoop, OwnedDisplayHandle},
        raw_window_handle::HasDisplayHandle,
    },
    raw_window_handle::RawDisplayHandle,
    std::{cell::RefCell, collections::HashSet, f32::consts::PI, mem, ptr::NonNull, rc::Rc},
    wl_client::{
        proxy::{self},
        Libwayland, QueueOwner,
    },
};

pub struct TestPane {
    pub queue: QueueOwner,
    pub features: HashSet<WpColorManagerV1Feature>,
    pub tf: HashSet<WpColorManagerV1TransferFunction>,
    pub primaries: HashSet<WpColorManagerV1Primaries>,
    state: Rc<State>,
    _display_handle: OwnedDisplayHandle,
}

struct State {
    _xdg_wm_base: XdgWmBase,
    _wl_compositor: WlCompositor,
    wl_subcompositor: WlSubcompositor,
    wp_color_manager_v1: WpColorManagerV1,
    wl_surface: WlSurface,
    wl_blend_surface: WlSurface,
    wp_color_management_surface_v1: WpColorManagementSurfaceV1,
    wp_color_management_blend_surface_v1: WpColorManagementSurfaceV1,
    xdg_surface: XdgSurface,
    _xdg_toplevel: XdgToplevel,
    vulkan_surface: VulkanSurface,
    vulkan_blend_surface: VulkanSurface,
    mutable: RefCell<Mutable>,
}

struct Mutable {
    scene: TestScene,
    width: i32,
    height: i32,
    description: TestColorDescription,
    matrix: ColorMatrix<Local, Lms>,
    need_render: bool,
    pending_description: Option<WpImageDescriptionV1>,
    blend_subsurface: Option<WlSubsurface>,
}

#[derive(Copy, Clone, PartialEq)]
pub enum TestPrimaries {
    Named(NamedPrimaries),
    Custom(Primaries),
}

#[derive(Copy, Clone, PartialEq)]
pub enum TestColorDescription {
    None,
    ScRgb,
    Parametric {
        primaries: TestPrimaries,
        transfer_function: TransferFunction,
        luminance: Option<Luminance>,
    },
}

#[derive(Copy, Clone, PartialEq)]
pub enum TestScene {
    Fill(Color),
    FillLeftRight([Color; 2]),
    FillTopBottom([Color; 2]),
    FillFour([Color; 4]),
    CenterBox([Color; 2], f32),
    Grid([Color; 2], u32, u32),
    Blend([Color; 2], f32),
}

#[derive(Copy, Clone, PartialEq, Default)]
pub struct Color {
    pub lumen: f32,
    pub lightness: f32,
    pub chroma: f32,
    pub hue: f32,
}

impl Color {
    fn to_lab(self) -> [f32; 4] {
        self.to_lab_alpha(1.0)
    }

    fn to_lab_alpha(self, alpha: f32) -> [f32; 4] {
        let mul = (self.lumen / 203.0).cbrt();
        [
            mul * self.lightness,
            mul * self.chroma,
            self.hue / 180.0 * PI,
            alpha,
        ]
    }
}

impl TestPane {
    pub async fn new<T>(event_loop: &EventLoop<T>) -> Self {
        let display_handle = event_loop.owned_display_handle();
        let RawDisplayHandle::Wayland(wl) = *display_handle.display_handle().unwrap().as_ref()
        else {
            unreachable!();
        };
        let wl_display = NonNull::new(wl.display.as_ptr().cast()).unwrap();
        // SAFETY: The OwnedDisplayHandle outlives all other objects.
        let con = unsafe {
            Libwayland::open()
                .unwrap()
                .wrap_borrowed_pointer(wl_display)
                .unwrap()
        };
        let queue = con.create_local_queue(c"color-test");
        let display = queue.display::<WlDisplay>();
        let singletons = get_singletons(&display);
        let wl_compositor: WlCompositor = singletons.get(1, 4);
        let wl_subcompositor: WlSubcompositor = singletons.get(1, 1);
        let xdg_wm_base: XdgWmBase = singletons.get(1, 1);
        proxy::set_event_handler(&xdg_wm_base, XdgWmBase::on_ping(|p, serial| p.pong(serial)));
        let wp_color_manager_v1: WpColorManagerV1 = singletons.get(1, 1);
        let supported_features = RefCell::new(HashSet::new());
        let supported_tf = RefCell::new(HashSet::new());
        let supported_primaries = RefCell::new(HashSet::new());
        queue
            .dispatch_scope_async(async |scope| {
                scope.set_event_handler_local(
                    &wp_color_manager_v1,
                    ColorManagerEventHandler {
                        features: &supported_features,
                        tf: &supported_tf,
                        primaries: &supported_primaries,
                    },
                );
                queue.dispatch_roundtrip_async().await.unwrap();
            })
            .await;
        let wl_surface = wl_compositor.create_surface();
        let wp_color_management_surface_v1 = wp_color_manager_v1.get_surface(&wl_surface);
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface);
        let xdg_toplevel = xdg_surface.get_toplevel();
        xdg_toplevel.set_title("test pane");
        wl_surface.commit();
        let wl_blend_surface = wl_compositor.create_surface();
        let wp_color_management_blend_surface_v1 =
            wp_color_manager_v1.get_surface(&wl_blend_surface);
        let vulkan_device = VulkanDevice::create().unwrap();
        let vulkan_surface = vulkan_device
            .create_surface(wl_display, &wl_surface)
            .unwrap();
        let vulkan_blend_surface = vulkan_device
            .create_surface(wl_display, &wl_blend_surface)
            .unwrap();
        let state = Rc::new(State {
            _xdg_wm_base: xdg_wm_base,
            _wl_compositor: wl_compositor,
            wl_subcompositor,
            wp_color_manager_v1,
            wl_surface,
            wl_blend_surface,
            wp_color_management_surface_v1,
            wp_color_management_blend_surface_v1,
            xdg_surface: xdg_surface.clone(),
            _xdg_toplevel: xdg_toplevel.clone(),
            vulkan_surface,
            vulkan_blend_surface,
            mutable: RefCell::new(Mutable {
                scene: TestScene::Fill(Color::default()),
                width: 0,
                height: 0,
                description: TestColorDescription::None,
                matrix: matrix_from_lms(Primaries::SRGB, Luminance::SRGB),
                need_render: false,
                pending_description: None,
                blend_subsurface: None,
            }),
        });
        proxy::set_event_handler_local(&xdg_surface, state.clone());
        proxy::set_event_handler_local(&xdg_toplevel, state.clone());
        TestPane {
            queue,
            features: supported_features.into_inner(),
            tf: supported_tf.into_inner(),
            primaries: supported_primaries.into_inner(),
            state,
            _display_handle: display_handle,
        }
    }

    pub fn apply_config(&self, description: TestColorDescription, scene: TestScene) {
        let m = &mut *self.state.mutable.borrow_mut();
        if m.description != description {
            m.description = description;
            m.need_render = true;
            if let Some(prev) = m.pending_description.take() {
                prev.destroy();
            }
            let s1 = &self.state.wp_color_management_surface_v1;
            let s2 = &self.state.wp_color_management_blend_surface_v1;
            match description {
                TestColorDescription::None => {
                    m.matrix = matrix_from_lms(Primaries::SRGB, Luminance::SRGB);
                    s1.unset_image_description();
                    s2.unset_image_description();
                }
                TestColorDescription::ScRgb => {
                    m.matrix = matrix_from_lms(Primaries::SRGB, Luminance::WINDOWS_SCRGB);
                    let scrgb = self.state.wp_color_manager_v1.create_windows_scrgb();
                    s1.set_image_description(&scrgb, WpColorManagerV1RenderIntent::PERCEPTUAL);
                    s2.set_image_description(&scrgb, WpColorManagerV1RenderIntent::PERCEPTUAL);
                    scrgb.destroy();
                }
                TestColorDescription::Parametric {
                    primaries,
                    transfer_function,
                    luminance,
                } => {
                    {
                        let mut lum = match transfer_function {
                            TransferFunction::St2084Pq => Luminance::ST2084_PQ,
                            TransferFunction::Bt1886 => Luminance::BT1886,
                            _ => Luminance::SRGB,
                        };
                        if let Some(l) = luminance {
                            lum.min = l.min;
                            lum.white = l.white;
                            if transfer_function == TransferFunction::St2084Pq {
                                lum.max.0 = l.min.0 + 10000.0;
                            } else {
                                lum.max = l.max;
                            }
                        }
                        let primaries = match primaries {
                            TestPrimaries::Named(n) => n.primaries(),
                            TestPrimaries::Custom(c) => c,
                        };
                        m.matrix = matrix_from_lms(primaries, lum);
                    }
                    let c = self.state.wp_color_manager_v1.create_parametric_creator();
                    match primaries {
                        TestPrimaries::Named(n) => c.set_primaries_named(n.wayland()),
                        TestPrimaries::Custom(p) => {
                            let map = |p: F64| (p.0 * 1_000_000.0) as i32;
                            c.set_primaries(
                                map(p.r.0),
                                map(p.r.1),
                                map(p.g.0),
                                map(p.g.1),
                                map(p.b.0),
                                map(p.b.1),
                                map(p.wp.0),
                                map(p.wp.1),
                            );
                        }
                    }
                    c.set_tf_named(transfer_function.wayland());
                    if let Some(l) = luminance {
                        c.set_luminances(
                            (l.min.0 * 10000.0) as u32,
                            l.max.0 as u32,
                            l.white.0 as u32,
                        );
                    }
                    let desc = c.create();
                    struct Eh(WpImageDescriptionV1, Rc<State>);
                    impl WpImageDescriptionV1EventHandler for Eh {
                        fn failed(
                            &self,
                            _slf: &WpImageDescriptionV1Ref,
                            _cause: WpImageDescriptionV1Cause,
                            msg: &str,
                        ) {
                            self.1.mutable.borrow_mut().pending_description = None;
                            eprintln!("Could not create image description: {}", msg);
                            self.0.destroy();
                        }

                        fn ready(&self, slf: &WpImageDescriptionV1Ref, _identity: u32) {
                            self.1.mutable.borrow_mut().pending_description = None;
                            self.1.wp_color_management_surface_v1.set_image_description(
                                slf,
                                WpColorManagerV1RenderIntent::PERCEPTUAL,
                            );
                            self.1
                                .wp_color_management_blend_surface_v1
                                .set_image_description(
                                    slf,
                                    WpColorManagerV1RenderIntent::PERCEPTUAL,
                                );
                            self.0.destroy();
                            self.1.render_frame(&mut self.1.mutable.borrow_mut());
                        }
                    }
                    proxy::set_event_handler_local(&desc, Eh(desc.clone(), self.state.clone()));
                    m.pending_description = Some(desc);
                }
            }
        }
        if m.scene != scene {
            m.scene = scene;
            match scene {
                TestScene::Blend(..) => {
                    if m.blend_subsurface.is_none() {
                        let ss = self
                            .state
                            .wl_subcompositor
                            .get_subsurface(&self.state.wl_blend_surface, &self.state.wl_surface);
                        m.blend_subsurface = Some(ss);
                    }
                }
                _ => {
                    if let Some(ss) = m.blend_subsurface.take() {
                        ss.destroy();
                    }
                }
            }
            m.need_render = true;
        }
        self.state.render_frame(m);
    }

    pub fn dispatch(&self) {
        self.queue.dispatch_pending().unwrap();
    }

    pub async fn wait_for_events(&self) {
        self.queue.wait_for_events().await.unwrap()
    }
}

struct ColorManagerEventHandler<'a> {
    features: &'a RefCell<HashSet<WpColorManagerV1Feature>>,
    tf: &'a RefCell<HashSet<WpColorManagerV1TransferFunction>>,
    primaries: &'a RefCell<HashSet<WpColorManagerV1Primaries>>,
}

impl WpColorManagerV1EventHandler for ColorManagerEventHandler<'_> {
    fn supported_feature(&self, _slf: &WpColorManagerV1Ref, feature: WpColorManagerV1Feature) {
        self.features.borrow_mut().insert(feature);
    }

    fn supported_tf_named(&self, _slf: &WpColorManagerV1Ref, tf: WpColorManagerV1TransferFunction) {
        self.tf.borrow_mut().insert(tf);
    }

    fn supported_primaries_named(
        &self,
        _slf: &WpColorManagerV1Ref,
        primaries: WpColorManagerV1Primaries,
    ) {
        self.primaries.borrow_mut().insert(primaries);
    }
}

impl State {
    fn render_frame(&self, m: &mut Mutable) {
        if !m.need_render || m.pending_description.is_some() {
            return;
        }
        if m.width <= 1 || m.height <= 1 {
            return;
        }
        let tf = match m.description {
            TestColorDescription::None => TransferFunction::Srgb,
            TestColorDescription::ScRgb => TransferFunction::Linear,
            TestColorDescription::Parametric {
                transfer_function, ..
            } => transfer_function,
        };
        let scene = match m.scene {
            TestScene::Fill(color) => Scene::Fill(color.to_lab()),
            TestScene::FillLeftRight(colors) => Scene::FillLeftRight(colors.map(|c| c.to_lab())),
            TestScene::FillTopBottom(colors) => Scene::FillTopBottom(colors.map(|c| c.to_lab())),
            TestScene::FillFour(colors) => Scene::FillFour(colors.map(|c| c.to_lab())),
            TestScene::CenterBox(colors, size) => {
                Scene::CenterBox(colors.map(|c| c.to_lab()), size / 100.0)
            }
            TestScene::Grid(colors, rows, cols) => {
                Scene::Grid(colors.map(|c| c.to_lab()), rows, cols)
            }
            TestScene::Blend(colors, alpha) => {
                self.vulkan_blend_surface
                    .render(
                        m.width as u32 / 2,
                        m.height as _,
                        Scene::BlendLeft(colors[1].to_lab_alpha(alpha)),
                        m.matrix,
                        tf,
                    )
                    .unwrap();
                Scene::BlendRight([colors[0].to_lab(), colors[1].to_lab_alpha(alpha)])
            }
        };
        self.vulkan_surface
            .render(m.width as _, m.height as _, scene, m.matrix, tf)
            .unwrap();
        m.need_render = false;
    }
}

impl XdgSurfaceEventHandler for Rc<State> {
    fn configure(&self, _slf: &XdgSurfaceRef, serial: u32) {
        self.xdg_surface.ack_configure(serial);
        self.render_frame(&mut self.mutable.borrow_mut());
    }
}

impl XdgToplevelEventHandler for Rc<State> {
    fn configure(&self, _slf: &XdgToplevelRef, mut width: i32, mut height: i32, _states: &[u8]) {
        if width <= 0 {
            width = 800;
        }
        if height <= 0 {
            height = 600;
        }
        let m = &mut *self.mutable.borrow_mut();
        m.need_render |= mem::replace(&mut m.width, width) != width;
        m.need_render |= mem::replace(&mut m.height, height) != height;
    }

    fn close(&self, _slf: &XdgToplevelRef) {
        std::process::exit(0);
    }
}
