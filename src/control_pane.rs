use {
    crate::{
        cmm::{
            Luminance, NamedPrimaries, NamedTransferFunction, Primaries, TransferFunction,
            TransferFunctionWithArgs,
        },
        ordered_float::F64,
        protocols::color_management_v1::wp_color_manager_v1::WpColorManagerV1Feature,
        test_pane::{
            Color, DescriptionData, TestColorDescription, TestPane, TestPrimaries, TestScene,
        },
    },
    bytemuck::{bytes_of, NoUninit},
    egui::{
        vec2, CentralPanel, Color32, ComboBox, Context, DragValue, FullOutput, Grid, Image,
        RawInput, Slider, TextureId, Ui, ViewportBuilder, ViewportInfo, Widget, WidgetText,
    },
    egui_wgpu::{
        wgpu::{
            Backends, BlendComponent, BlendState, ColorTargetState, DeviceDescriptor, Extent3d,
            Features, FilterMode, FragmentState, IndexFormat, InstanceDescriptor, Limits, LoadOp,
            Operations, PipelineLayoutDescriptor, PresentMode, PrimitiveState, PrimitiveTopology,
            PushConstantRange, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
            RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp,
            TexelCopyTextureInfo, Texture, TextureDescriptor, TextureDimension, TextureFormat,
            TextureUsages, TextureView, TextureViewDescriptor, VertexState,
        },
        winit::Painter,
        RenderState, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew,
    },
    egui_winit::winit::{
        event::WindowEvent,
        event_loop::ActiveEventLoop,
        window::{Window, WindowId},
    },
    isnt::std_1::collections::IsntHashSetExt,
    linearize::{Linearize, LinearizeExt},
    pollster::block_on,
    std::{
        cell::Cell,
        mem,
        num::NonZeroU32,
        rc::Rc,
        sync::Arc,
        time::{Duration, Instant},
    },
};

pub struct ControlPane {
    ctx: Context,
    window: Arc<Window>,
    window_id: WindowId,
    state: egui_winit::State,
    painter: Painter,
    pub need_repaint: bool,
    have_frame: Rc<Cell<bool>>,
    output: FullOutput,
    pub draw_state: DrawState,
    pub repaint_after: Option<Instant>,
}

pub struct DrawState {
    renderer: RenderState,
    max_size: u32,
    config: ControlPaneConfig,
    cie_diagram: Option<CieDiagram>,
    horseshoe_pipeline: RenderPipeline,
    triangle_pipeline: RenderPipeline,
    pub create_description_error_message: Option<String>,
    pub preferred_description_error_message: Option<String>,
    pub preferred_description_data: Option<DescriptionData>,
}

struct CieDiagram {
    horseshoe_tex: Texture,
    _horseshoe_view: TextureView,
    tex: Texture,
    view: TextureView,
    id: TextureId,
    size: u32,
}

impl ControlPane {
    pub fn new(event_loop: &ActiveEventLoop, test_pane: &TestPane) -> Self {
        let ctx = Context::default();
        let viewport_builder = ViewportBuilder::default().with_title("control pane");
        let window = egui_winit::create_window(&ctx, event_loop, &viewport_builder).unwrap();
        let window = Arc::new(window);
        let state = egui_winit::State::new(
            ctx.clone(),
            ctx.viewport_id(),
            &event_loop.owned_display_handle(),
            None,
            None,
            None,
        );
        let wgpu_configuration = WgpuConfiguration {
            present_mode: PresentMode::Mailbox,
            wgpu_setup: WgpuSetup::CreateNew(WgpuSetupCreateNew {
                instance_descriptor: InstanceDescriptor {
                    backends: Backends::VULKAN,
                    ..Default::default()
                },
                device_descriptor: Arc::new(|_| DeviceDescriptor {
                    required_features: Features::PUSH_CONSTANTS,
                    required_limits: Limits {
                        max_push_constant_size: 128,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut painter = block_on(Painter::new(
            ctx.clone(),
            wgpu_configuration,
            1,
            None,
            false,
            false,
        ));
        block_on(painter.set_window(ctx.viewport_id(), Some(window.clone()))).unwrap();
        let mut viewport_info = ViewportInfo::default();
        egui_winit::update_viewport_info(&mut viewport_info, &ctx, &window, false);
        let mut raw_input = RawInput::default();
        raw_input.viewports.insert(ctx.viewport_id(), viewport_info);
        let mut slf = Self {
            ctx,
            window_id: window.id(),
            window,
            state,
            draw_state: init_wgpu(&painter, test_pane),
            painter,
            need_repaint: true,
            have_frame: Rc::new(Cell::new(true)),
            output: Default::default(),
            repaint_after: None,
        };
        slf.run(test_pane, raw_input);
        slf
    }

    fn run(&mut self, test_pane: &TestPane, raw_input: RawInput) {
        let new_output = self.ctx.run(raw_input, |ctx| {
            draw_egui(ctx, test_pane, &mut self.draw_state);
            let config = &self.draw_state.config;
            let scene = match config.scene {
                SelectedScene::Fill => TestScene::Fill(config.fill),
                SelectedScene::FillLeftRight => TestScene::FillLeftRight(config.left_right),
                SelectedScene::FillTopBottom => TestScene::FillTopBottom(config.top_bottom),
                SelectedScene::FillFour => TestScene::FillFour(config.four_corners),
                SelectedScene::CenterBox => {
                    TestScene::CenterBox(config.center_box, config.center_box_size)
                }
                SelectedScene::Grid => {
                    TestScene::Grid(config.grid, config.grid_rows, config.grid_cols)
                }
                SelectedScene::Blend => TestScene::Blend(config.blend, config.blend_alpha),
            };
            let desc = match config.cd_type {
                ColorDescriptionType::None => TestColorDescription::None,
                ColorDescriptionType::ScRgb => TestColorDescription::ScRgb,
                ColorDescriptionType::Parametric => TestColorDescription::Parametric {
                    primaries: match config.use_custom_primaries {
                        true => TestPrimaries::Custom(config.primaries),
                        false => TestPrimaries::Named(config.named_primaries),
                    },
                    transfer_function: TransferFunctionWithArgs {
                        tf: config.tf,
                        pow: config.tf_power,
                    },
                    luminance: config.enable_luminance.then_some(config.luminance),
                },
            };
            test_pane.apply_config(desc, scene);
        });
        self.output.append(new_output);
        let repaint_delay = self
            .output
            .viewport_output
            .values_mut()
            .map(|v| mem::replace(&mut v.repaint_delay, Duration::MAX))
            .min()
            .unwrap_or(Duration::MAX);
        if let Some(instant) = Instant::now().checked_add(repaint_delay) {
            self.repaint_after = Some(instant);
        }
        if !self.need_repaint {
            return;
        }
        if !self.have_frame.replace(false) {
            return;
        }
        self.window.pre_present_notify();
        self.window.request_redraw();
        self.need_repaint = false;
        self.have_frame.set(false);
        self.state
            .handle_platform_output(&self.window, mem::take(&mut self.output.platform_output));
        let clipped_primitives = self.ctx.tessellate(
            mem::take(&mut self.output.shapes),
            self.output.pixels_per_point,
        );
        self.painter.paint_and_update_textures(
            self.ctx.viewport_id(),
            self.output.pixels_per_point,
            [0.0; 4],
            &clipped_primitives,
            &self.output.textures_delta,
            vec![],
        );
        self.output.textures_delta.clear();
        self.output.viewport_output.clear();
    }

    pub fn handle_event(&mut self, window_id: WindowId, event: WindowEvent, test_pane: &TestPane) {
        if window_id != self.window_id {
            return;
        }
        match event {
            WindowEvent::Resized(size) => {
                let width = NonZeroU32::new(size.width).unwrap_or(NonZeroU32::new(800).unwrap());
                let height = NonZeroU32::new(size.height).unwrap_or(NonZeroU32::new(600).unwrap());
                self.painter
                    .on_window_resized(self.ctx.viewport_id(), width, height);
                self.have_frame.set(true);
                self.need_repaint = true;
            }
            WindowEvent::RedrawRequested => {
                self.have_frame.set(true);
            }
            WindowEvent::CloseRequested => {
                std::process::exit(0);
            }
            _ => {}
        }
        if event != WindowEvent::RedrawRequested {
            self.need_repaint |= self.state.on_window_event(&self.window, &event).repaint;
        }
        self.maybe_run(test_pane);
    }

    pub fn maybe_run(&mut self, test_pane: &TestPane) {
        if !self.have_frame.get() {
            return;
        }
        let mut raw_input = self.state.take_egui_input(&self.window);
        egui_winit::update_viewport_info(
            raw_input.viewports.get_mut(&raw_input.viewport_id).unwrap(),
            &self.ctx,
            &self.window,
            false,
        );
        self.run(test_pane, raw_input);
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Default, Linearize)]
enum View {
    #[default]
    Scenes,
    ColorDescription,
    Feedback,
    Settings,
}

#[derive(Copy, Clone, Eq, PartialEq, Default, Linearize)]
enum SelectedScene {
    #[default]
    Fill,
    FillLeftRight,
    FillTopBottom,
    FillFour,
    CenterBox,
    Grid,
    Blend,
}

#[derive(Copy, Clone, Eq, PartialEq, Default, Linearize)]
enum ColorDescriptionType {
    #[default]
    None,
    ScRgb,
    Parametric,
}

impl From<View> for WidgetText {
    fn from(val: View) -> Self {
        let txt = match val {
            View::Settings => "settings",
            View::ColorDescription => "color description",
            View::Scenes => "scenes",
            View::Feedback => "feedback",
        };
        txt.into()
    }
}

impl From<SelectedScene> for WidgetText {
    fn from(val: SelectedScene) -> Self {
        let txt = match val {
            SelectedScene::Fill => "fill",
            SelectedScene::FillLeftRight => "gradient (L -> R)",
            SelectedScene::FillTopBottom => "gradient (T -> B)",
            SelectedScene::FillFour => "four corners",
            SelectedScene::CenterBox => "center box",
            SelectedScene::Grid => "grid",
            SelectedScene::Blend => "blend",
        };
        txt.into()
    }
}

impl From<ColorDescriptionType> for WidgetText {
    fn from(val: ColorDescriptionType) -> Self {
        let txt = match val {
            ColorDescriptionType::None => "none",
            ColorDescriptionType::ScRgb => "scRGB",
            ColorDescriptionType::Parametric => "parametric",
        };
        txt.into()
    }
}

impl From<NamedPrimaries> for WidgetText {
    fn from(val: NamedPrimaries) -> Self {
        let txt = match val {
            NamedPrimaries::Srgb => "srgb",
            NamedPrimaries::PalM => "pal_m",
            NamedPrimaries::Pal => "pal",
            NamedPrimaries::Ntsc => "ntsc",
            NamedPrimaries::GenericFilm => "generic_film",
            NamedPrimaries::Bt2020 => "bt2020",
            NamedPrimaries::Cie1931Xyz => "cie1931_xyz",
            NamedPrimaries::DciP3 => "dci_p3",
            NamedPrimaries::DisplayP3 => "display_p3",
            NamedPrimaries::AdobeRgb => "adobe_rgb",
        };
        txt.into()
    }
}

impl From<TransferFunction> for WidgetText {
    fn from(val: TransferFunction) -> Self {
        match val {
            TransferFunction::Named(n) => n.into(),
            TransferFunction::Pow => "power".into(),
        }
    }
}

impl From<NamedTransferFunction> for WidgetText {
    fn from(val: NamedTransferFunction) -> Self {
        let txt = match val {
            NamedTransferFunction::Srgb => "srgb",
            NamedTransferFunction::Linear => "ext_linear",
            NamedTransferFunction::St2084Pq => "st2084_pq",
            NamedTransferFunction::Bt1886 => "bt1886",
            NamedTransferFunction::Gamma22 => "gamma22",
            NamedTransferFunction::Gamma28 => "gamma28",
            NamedTransferFunction::St240 => "st240",
            NamedTransferFunction::ExtSrgb => "ext_srgb",
            NamedTransferFunction::Log100 => "log_100",
            NamedTransferFunction::Log316 => "log_316",
            NamedTransferFunction::St428 => "st428",
        };
        txt.into()
    }
}

struct ControlPaneConfig {
    view: View,

    // settings
    max_lumen: f32,
    max_chroma: f32,

    // color description
    cd_type: ColorDescriptionType,
    named_primaries: NamedPrimaries,
    use_custom_primaries: bool,
    tf: TransferFunction,
    tf_power: f32,
    enable_luminance: bool,
    luminance: Luminance,
    primaries: Primaries,

    // scene
    scene: SelectedScene,

    fill: Color,

    left_right: [Color; 2],

    top_bottom: [Color; 2],

    four_corners: [Color; 4],

    center_box: [Color; 2],
    center_box_size: f32,

    grid: [Color; 2],
    grid_rows: u32,
    grid_cols: u32,

    blend: [Color; 2],
    blend_alpha: f32,
}

impl Default for ControlPaneConfig {
    fn default() -> Self {
        let default_lumen = 203.0;
        let default_lightness = 0.7;
        let default_chroma = 0.2;
        Self {
            view: Default::default(),
            max_lumen: 1000.0,
            max_chroma: 0.5,
            cd_type: ColorDescriptionType::None,
            named_primaries: NamedPrimaries::Srgb,
            use_custom_primaries: false,
            tf: TransferFunction::Named(NamedTransferFunction::Gamma22),
            tf_power: 2.2,
            enable_luminance: false,
            luminance: Default::default(),
            primaries: Primaries::SRGB,
            scene: SelectedScene::FillFour,
            fill: Color {
                lumen: default_lumen,
                lightness: 1.0,
                chroma: 0.0,
                hue: 0.0,
            },
            left_right: [
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 0.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 180.0,
                },
            ],
            top_bottom: [
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 90.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 270.0,
                },
            ],
            four_corners: [
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 0.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 90.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 270.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 180.0,
                },
            ],
            center_box: [
                Color {
                    lumen: 0.0,
                    lightness: 0.0,
                    chroma: 0.0,
                    hue: 0.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: 1.0,
                    chroma: 0.0,
                    hue: 0.0,
                },
            ],
            center_box_size: 50.0,
            grid: [
                Color {
                    lumen: 0.0,
                    lightness: 0.0,
                    chroma: 0.0,
                    hue: 0.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: 1.0,
                    chroma: 0.0,
                    hue: 0.0,
                },
            ],
            grid_rows: 4,
            grid_cols: 4,
            blend: [
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 40.0,
                },
                Color {
                    lumen: default_lumen,
                    lightness: default_lightness,
                    chroma: default_chroma,
                    hue: 140.0,
                },
            ],
            blend_alpha: 0.5,
        }
    }
}

fn draw_egui(ctx: &Context, test_pane: &TestPane, ds: &mut DrawState) {
    CentralPanel::default().show(ctx, |ui| {
        ComboBox::from_label("View")
            .selected_text(ds.config.view)
            .show_ui(ui, |ui| {
                for s in View::variants() {
                    ui.selectable_value(&mut ds.config.view, s, s);
                }
            });
        ui.add_space(10.0);
        match ds.config.view {
            View::Scenes => draw_scenes(ui, ds),
            View::Settings => draw_settings(ui, ds),
            View::ColorDescription => draw_color_description(ui, test_pane, ds),
            View::Feedback => draw_feedback(ui, ds),
        }
    });
}

fn draw_color_description(ui: &mut Ui, test_pane: &TestPane, ds: &mut DrawState) {
    ui.label("Changing these settings should not affect the output.");
    ui.add_space(20.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(270.0);
            draw_color_description_settings(ui, test_pane, ds);
            if let Some(err) = &ds.create_description_error_message {
                ui.add_space(20.0);
                ui.colored_label(Color32::from_rgb(255, 128, 128), err);
            }
        });
        ui.vertical(|ui| {
            let primaries = match ds.config.cd_type {
                ColorDescriptionType::None => Primaries::SRGB,
                ColorDescriptionType::ScRgb => Primaries::SRGB,
                ColorDescriptionType::Parametric => match ds.config.use_custom_primaries {
                    true => ds.config.primaries,
                    false => ds.config.named_primaries.primaries(),
                },
            };
            draw_chromaticity_diagram(ui, ds, primaries);
        });
    });
}

fn draw_color_description_settings(ui: &mut Ui, test_pane: &TestPane, ds: &mut DrawState) {
    let supported_features = &test_pane.caps.features;
    let supported_tf = &test_pane.caps.tf;
    let supported_primaries = &test_pane.caps.primaries;

    let config = &mut ds.config;
    ComboBox::from_label("Type")
        .selected_text(config.cd_type)
        .show_ui(ui, |ui| {
            let mut val = |ty: ColorDescriptionType| {
                ui.selectable_value(&mut config.cd_type, ty, ty);
            };
            val(ColorDescriptionType::None);
            if supported_features.contains(&WpColorManagerV1Feature::WINDOWS_SCRGB) {
                val(ColorDescriptionType::ScRgb);
            }
            if supported_features.contains(&WpColorManagerV1Feature::PARAMETRIC)
                && supported_primaries.is_not_empty()
                && supported_tf.is_not_empty()
            {
                val(ColorDescriptionType::Parametric);
            }
        });
    ui.add_space(20.0);
    let mut primaries;
    if config.cd_type == ColorDescriptionType::Parametric {
        if supported_features.contains(&WpColorManagerV1Feature::SET_PRIMARIES) {
            ui.checkbox(&mut config.use_custom_primaries, "Custom primaries");
        }
        ui.add_enabled_ui(!config.use_custom_primaries, |ui| {
            ComboBox::from_label("Named primaries")
                .selected_text(config.named_primaries)
                .show_ui(ui, |ui| {
                    for primary in NamedPrimaries::variants() {
                        if supported_primaries.contains(&primary.wayland()) {
                            ui.selectable_value(&mut config.named_primaries, primary, primary);
                        }
                    }
                });
        });
        if config.use_custom_primaries {
            primaries = config.primaries;
        } else {
            primaries = config.named_primaries.primaries();
        }
        ui.add_enabled_ui(config.use_custom_primaries, |ui| {
            Grid::new("custom primaries").show(ui, |ui| {
                for (name, cp) in [
                    ("r", &mut primaries.r),
                    ("g", &mut primaries.g),
                    ("b", &mut primaries.b),
                    ("wp", &mut primaries.wp),
                ] {
                    let (x, y) = cp;
                    ui.label(name);
                    DragValue::new(&mut x.0).speed(0.001).ui(ui);
                    DragValue::new(&mut y.0).speed(0.001).ui(ui);
                    ui.end_row();
                }
            });
        });
        if config.use_custom_primaries {
            config.primaries = primaries;
        }
        ComboBox::from_label("Transfer function")
            .selected_text(config.tf)
            .show_ui(ui, |ui| {
                for tf in NamedTransferFunction::variants() {
                    if supported_tf.contains(&tf.wayland()) {
                        ui.selectable_value(&mut config.tf, TransferFunction::Named(tf), tf);
                    }
                }
                if supported_features.contains(&WpColorManagerV1Feature::SET_TF_POWER) {
                    ui.selectable_value(
                        &mut config.tf,
                        TransferFunction::Pow,
                        TransferFunction::Pow,
                    );
                }
            });
        if config.tf == TransferFunction::Pow {
            Slider::new(&mut config.tf_power, 1.0..=10.0)
                .prefix("Power: ")
                .drag_value_speed(0.1)
                .ui(ui);
        }
        if supported_features.contains(&WpColorManagerV1Feature::SET_LUMINANCES) {
            ui.checkbox(&mut config.enable_luminance, "Luminance");
            if config.enable_luminance {
                Slider::new(&mut config.luminance.min.0, 0.0..=100.0)
                    .prefix("Min: ")
                    .drag_value_speed(1.0)
                    .ui(ui);
                let min = config.luminance.min.0 + 1.0;
                Slider::new(&mut config.luminance.max.0, min..=10000.0)
                    .prefix("Max: ")
                    .drag_value_speed(1.0)
                    .ui(ui);
                Slider::new(&mut config.luminance.white.0, min..=config.luminance.max.0)
                    .prefix("White: ")
                    .drag_value_speed(1.0)
                    .ui(ui);
            }
        }
    }
}

fn draw_feedback(ui: &mut Ui, ds: &mut DrawState) {
    if let Some(err) = &ds.preferred_description_error_message {
        ui.colored_label(Color32::from_rgb(255, 128, 128), err);
        return;
    }
    let Some(data) = ds.preferred_description_data else {
        return;
    };
    let primaries = match data.primaries {
        TestPrimaries::Named(p) => p.primaries(),
        TestPrimaries::Custom(p) => p,
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(270.0);
            ui.horizontal_top(|ui| {
                ui.label("Primaries:");
                if let TestPrimaries::Named(p) = data.primaries {
                    ui.label(p);
                };
            });
            ui.indent("primaries", |ui| {
                Grid::new("primaries").show(ui, |ui| {
                    for (name, cp) in [
                        ("r", primaries.r),
                        ("g", primaries.g),
                        ("b", primaries.b),
                        ("wp", primaries.wp),
                    ] {
                        let (x, y) = cp;
                        ui.label(name);
                        ui.label(x.0.to_string());
                        ui.label(y.0.to_string());
                        ui.end_row();
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.label("Transfer function:");
                match data.tf {
                    TransferFunction::Named(n) => {
                        ui.label(n);
                    }
                    TransferFunction::Pow => {
                        ui.label(format!("pow({})", data.tf_power));
                    }
                }
            });
            ui.add_space(10.0);
            if let Some(lum) = data.luminance {
                ui.label("Luminance:");
                ui.indent("luminance", |ui| {
                    Grid::new("luminance").show(ui, |ui| {
                        ui.label("Min");
                        ui.label(lum.min.to_string());
                        ui.end_row();
                        ui.label("Max");
                        ui.label(lum.max.to_string());
                        ui.end_row();
                        ui.label("White");
                        ui.label(lum.white.to_string());
                        ui.end_row();
                    });
                });
            }
        });
        ui.vertical(|ui| {
            draw_chromaticity_diagram(ui, ds, primaries);
        });
    });
}

fn draw_settings(ui: &mut Ui, ds: &mut DrawState) {
    let config = &mut ds.config;
    Slider::new(&mut config.max_lumen, 0.0..=10000.0)
        .prefix("Max lumen: ")
        .drag_value_speed(10.0)
        .ui(ui);
    Slider::new(&mut config.max_chroma, 0.0..=10.0)
        .prefix("Max chroma: ")
        .drag_value_speed(0.1)
        .ui(ui);
}

fn draw_scenes(ui: &mut Ui, ds: &mut DrawState) {
    let config = &mut ds.config;
    ComboBox::from_label("Scene")
        .selected_text(config.scene)
        .show_ui(ui, |ui| {
            for s in SelectedScene::variants() {
                ui.selectable_value(&mut config.scene, s, s);
            }
        });
    ui.add_space(20.0);
    let max_lumen = config.max_lumen;
    let max_chroma = config.max_chroma;
    let mut idx = 0;
    let mut colors = |ui: &mut Ui, colors: &mut [(&str, &mut Color)]| {
        let salt = format!("c{}", idx);
        idx += 1;
        Grid::new(salt).spacing([20.0, 20.0]).show(ui, |ui| {
            for (name, c) in colors {
                ui.label(*name);
                ui.vertical(|ui| {
                    Slider::new(&mut c.lumen, 0.0..=max_lumen)
                        .prefix("Lumen: ")
                        .drag_value_speed(1.0)
                        .ui(ui);
                    Slider::new(&mut c.lightness, 0.0..=1.0)
                        .prefix("Lightness: ")
                        .drag_value_speed(0.01)
                        .ui(ui);
                    Slider::new(&mut c.chroma, 0.0..=max_chroma)
                        .prefix("Chroma: ")
                        .drag_value_speed(0.01)
                        .ui(ui);
                    Slider::new(&mut c.hue, 0.0..=360.0)
                        .prefix("Hue: ")
                        .drag_value_speed(0.1)
                        .ui(ui);
                });
                ui.end_row();
            }
        });
    };
    match config.scene {
        SelectedScene::Fill => {
            colors(ui, &mut [("color: ", &mut config.fill)]);
        }
        SelectedScene::FillLeftRight => {
            let [left, right] = &mut config.left_right;
            ui.horizontal_top(|ui| {
                colors(ui, &mut [("left: ", left)]);
                colors(ui, &mut [("right: ", right)]);
            });
        }
        SelectedScene::FillTopBottom => {
            let [top, bottom] = &mut config.top_bottom;
            colors(ui, &mut [("top: ", top), ("bottom: ", bottom)]);
        }
        SelectedScene::FillFour => {
            let [top_right, top_left, bottom_right, bottom_left] = &mut config.four_corners;
            ui.horizontal_top(|ui| {
                colors(
                    ui,
                    &mut [("top left: ", top_left), ("bottom left: ", bottom_left)],
                );
                colors(
                    ui,
                    &mut [("top right: ", top_right), ("bottom right: ", bottom_right)],
                );
            });
        }
        SelectedScene::CenterBox => {
            let [bg, fg] = &mut config.center_box;
            Slider::new(&mut config.center_box_size, 0.0..=100.0)
                .prefix("Size: ")
                .drag_value_speed(1.0)
                .ui(ui);
            ui.horizontal_top(|ui| {
                colors(ui, &mut [("background: ", bg)]);
                colors(ui, &mut [("foreground: ", fg)]);
            });
        }
        SelectedScene::Grid => {
            let [bg, fg] = &mut config.grid;
            ui.horizontal_top(|ui| {
                Slider::new(&mut config.grid_rows, 1..=25)
                    .prefix("Rows: ")
                    .ui(ui);
                Slider::new(&mut config.grid_cols, 1..=25)
                    .prefix("Columns: ")
                    .ui(ui);
            });
            ui.horizontal_top(|ui| {
                colors(ui, &mut [("background: ", bg)]);
                colors(ui, &mut [("foreground: ", fg)]);
            });
        }
        SelectedScene::Blend => {
            ui.label(concat!(
                "Top left shows the background color.\n",
                "Top right shows the foreground color.\n",
                "Bottom left blends with a sub-surface.\n",
                "Bottom right blends in the client.\n",
                "\n",
                "Bottom left and bottom right are not expect to be identical due to the ",
                "different blend space."
            ));
            ui.add_space(10.0);
            let [bg, fg] = &mut config.blend;
            Slider::new(&mut config.blend_alpha, 0.0..=1.0)
                .prefix("Alpha: ")
                .ui(ui);
            colors(ui, &mut [("background: ", bg), ("foreground: ", fg)]);
        }
    }
}

fn draw_chromaticity_diagram(ui: &mut Ui, ds: &mut DrawState, primaries: Primaries) {
    let available = ui.available_size();
    let available = available.x.min(available.y).round();
    let size = (ui.pixels_per_point() * available).round() as u32;
    let size = size.min(ds.max_size);
    if let Some(cie) = &mut ds.cie_diagram {
        if cie.size != size {
            ds.renderer.renderer.write().free_texture(&cie.id);
            ds.cie_diagram = None;
        }
    }
    let cie = match &mut ds.cie_diagram {
        Some(c) => c,
        _ => {
            let horseshoe_tex = ds.renderer.device.create_texture(&TextureDescriptor {
                label: None,
                size: Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[TextureFormat::Rgba8Unorm],
            });
            let horseshoe_view = horseshoe_tex.create_view(&TextureViewDescriptor {
                ..Default::default()
            });
            let mut encoder = ds
                .renderer
                .device
                .create_command_encoder(&Default::default());
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &horseshoe_view,
                    resolve_target: None,
                    ops: Default::default(),
                })],
                ..Default::default()
            });
            pass.set_pipeline(&ds.horseshoe_pipeline);
            pass.draw(0..4, 0..1);
            drop(pass);
            ds.renderer.queue.submit([encoder.finish()]);
            let tex = ds.renderer.device.create_texture(&TextureDescriptor {
                label: None,
                size: Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8UnormSrgb,
                usage: TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_DST,
                view_formats: &[TextureFormat::Rgba8UnormSrgb],
            });
            let view = tex.create_view(&TextureViewDescriptor {
                ..Default::default()
            });
            let tex_id = ds.renderer.renderer.write().register_native_texture(
                &ds.renderer.device,
                &view,
                FilterMode::Linear,
            );
            ds.cie_diagram.insert(CieDiagram {
                horseshoe_tex,
                _horseshoe_view: horseshoe_view,
                tex,
                view,
                id: tex_id,
                size,
            })
        }
    };
    let mut encoder = ds
        .renderer
        .device
        .create_command_encoder(&Default::default());
    encoder.copy_texture_to_texture(
        TexelCopyTextureInfo {
            texture: &cie.horseshoe_tex,
            mip_level: 0,
            origin: Default::default(),
            aspect: Default::default(),
        },
        TexelCopyTextureInfo {
            texture: &cie.tex,
            mip_level: 0,
            origin: Default::default(),
            aspect: Default::default(),
        },
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &cie.view,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(&ds.triangle_pipeline);
    #[derive(NoUninit, Copy, Clone)]
    #[repr(C)]
    struct Data {
        r: [f32; 2],
        g: [f32; 2],
        b: [f32; 2],
        wp: [f32; 2],
    }
    let map = |f: (F64, F64)| [f.0 .0 as f32, f.1 .0 as f32];
    let data = Data {
        r: map(primaries.r),
        g: map(primaries.g),
        b: map(primaries.b),
        wp: map(primaries.wp),
    };
    pass.set_push_constants(ShaderStages::FRAGMENT, 0, bytes_of(&data));
    pass.draw(0..4, 0..1);
    drop(pass);
    ds.renderer.queue.submit([encoder.finish()]);
    let image = Image::from_texture((cie.id, vec2(available as _, available as _)));
    image.ui(ui);
}

fn init_wgpu(painter: &Painter, test_pane: &TestPane) -> DrawState {
    let renderer = painter.render_state().unwrap();
    let horseshoe_module = renderer
        .device
        .create_shader_module(ShaderModuleDescriptor {
            label: None,
            source: ShaderSource::Wgsl(include_str!("wgpu_shaders/horseshoe.wgsl").into()),
        });
    let horseshoe_pipeline = renderer
        .device
        .create_render_pipeline(&RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: VertexState {
                module: &horseshoe_module,
                entry_point: None,
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(IndexFormat::Uint32),
                ..Default::default()
            },
            fragment: Some(FragmentState {
                module: &horseshoe_module,
                entry_point: None,
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: Default::default(),
                })],
            }),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
    let triangle_module = renderer
        .device
        .create_shader_module(ShaderModuleDescriptor {
            label: None,
            source: ShaderSource::Wgsl(include_str!("wgpu_shaders/triangle.wgsl").into()),
        });
    let triangle_pipeline = renderer
        .device
        .create_render_pipeline(&RenderPipelineDescriptor {
            label: None,
            layout: Some(
                &renderer
                    .device
                    .create_pipeline_layout(&PipelineLayoutDescriptor {
                        push_constant_ranges: &[PushConstantRange {
                            stages: ShaderStages::FRAGMENT,
                            range: 0..32,
                        }],
                        ..Default::default()
                    }),
            ),
            vertex: VertexState {
                module: &triangle_module,
                entry_point: None,
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(IndexFormat::Uint32),
                ..Default::default()
            },
            fragment: Some(FragmentState {
                module: &triangle_module,
                entry_point: None,
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8UnormSrgb,
                    blend: Some(BlendState {
                        color: BlendComponent::OVER,
                        alpha: BlendComponent::OVER,
                    }),
                    write_mask: Default::default(),
                })],
            }),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
    let limits = renderer.device.limits();
    let mut config = ControlPaneConfig::default();
    for tf in NamedTransferFunction::variants() {
        if test_pane.caps.tf.contains(&tf.wayland()) {
            config.tf = TransferFunction::Named(tf);
            break;
        }
    }
    DrawState {
        renderer,
        max_size: limits.max_texture_dimension_2d,
        config,
        cie_diagram: None,
        horseshoe_pipeline,
        triangle_pipeline,
        create_description_error_message: None,
        preferred_description_error_message: None,
        preferred_description_data: None,
    }
}
