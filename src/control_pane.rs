use {
    crate::{
        cmm::{Luminance, NamedPrimaries, TransferFunction},
        protocols::{
            color_management_v1::wp_color_manager_v1::WpColorManagerV1Feature,
            wayland::{wl_callback::WlCallback, wl_surface::WlSurface},
        },
        test_pane::{Color, TestColorDescription, TestPane, TestScene},
    },
    egui::{
        CentralPanel, ComboBox, Context, FullOutput, Grid, RawInput, Slider, Ui, ViewportBuilder,
        ViewportInfo, Widget, WidgetText,
    },
    egui_wgpu::{
        wgpu::{Backends, InstanceDescriptor, PresentMode},
        winit::Painter,
        WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew,
    },
    egui_winit::winit::{
        event::WindowEvent,
        event_loop::ActiveEventLoop,
        window::{Window, WindowId},
    },
    isnt::std_1::collections::IsntHashSetExt,
    linearize::{Linearize, LinearizeExt},
    pollster::block_on,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    std::{
        cell::Cell,
        mem,
        num::NonZeroU32,
        rc::Rc,
        sync::Arc,
        time::{Duration, Instant},
    },
    wl_client::proxy,
};

pub struct ControlPane {
    ctx: Context,
    wl_surface: WlSurface,
    window: Arc<Window>,
    window_id: WindowId,
    state: egui_winit::State,
    painter: Painter,
    pub need_repaint: bool,
    have_frame: Rc<Cell<bool>>,
    output: FullOutput,
    config: ControlPaneConfig,
    pub repaint_after: Option<Instant>,
}

impl ControlPane {
    pub fn new(event_loop: &ActiveEventLoop, test_pane: &TestPane) -> Self {
        let ctx = Context::default();
        let viewport_builder = ViewportBuilder::default().with_title("control pane");
        let window = egui_winit::create_window(&ctx, event_loop, &viewport_builder).unwrap();
        let RawWindowHandle::Wayland(wl_surface) = window.window_handle().unwrap().as_raw() else {
            unreachable!();
        };
        let wl_surface: WlSurface =
            unsafe { test_pane.queue.wrap_wl_proxy(wl_surface.surface.cast()) };
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
            wl_surface,
            window,
            state,
            painter,
            need_repaint: true,
            have_frame: Rc::new(Cell::new(true)),
            output: Default::default(),
            config: Default::default(),
            repaint_after: None,
        };
        slf.run(test_pane, raw_input);
        slf
    }

    fn run(&mut self, test_pane: &TestPane, raw_input: RawInput) {
        let new_output = self.ctx.run(raw_input, |ctx| {
            draw_egui(ctx, test_pane, &mut self.config);
            let scene = match self.config.scene {
                SelectedScene::Fill => TestScene::Fill(self.config.fill),
                SelectedScene::FillLeftRight => TestScene::FillLeftRight(self.config.left_right),
                SelectedScene::FillTopBottom => TestScene::FillTopBottom(self.config.top_bottom),
                SelectedScene::FillFour => TestScene::FillFour(self.config.four_corners),
                SelectedScene::CenterBox => {
                    TestScene::CenterBox(self.config.center_box, self.config.center_box_size)
                }
                SelectedScene::Grid => TestScene::Grid(
                    self.config.grid,
                    self.config.grid_rows,
                    self.config.grid_cols,
                ),
                SelectedScene::Blend => {
                    TestScene::Blend(self.config.blend, self.config.blend_alpha)
                }
            };
            let desc = match self.config.cd_type {
                ColorDescriptionType::None => TestColorDescription::None,
                ColorDescriptionType::ScRgb => TestColorDescription::ScRgb,
                ColorDescriptionType::Parametric => TestColorDescription::Parametric {
                    primaries: self.config.primaries,
                    transfer_function: self.config.tf,
                    luminance: self
                        .config
                        .enable_luminance
                        .then_some(self.config.luminance),
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
        self.need_repaint |= !self.output.shapes.is_empty();
        if !self.need_repaint {
            return;
        }
        if !self.have_frame.replace(false) {
            return;
        }
        self.window.pre_present_notify();
        let frame = self.wl_surface.frame();
        let have_frame = self.have_frame.clone();
        proxy::set_event_handler_local(
            &frame.clone(),
            WlCallback::on_done(move |_, _| {
                proxy::destroy(&frame);
                have_frame.set(true);
            }),
        );
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
            WindowEvent::CloseRequested => {
                std::process::exit(0);
            }
            _ => {}
        }
        self.need_repaint |= self.state.on_window_event(&self.window, &event).repaint;
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
        let txt = match val {
            TransferFunction::Srgb => "srgb",
            TransferFunction::Linear => "ext_linear",
            TransferFunction::St2084Pq => "st2084_pq",
            TransferFunction::Bt1886 => "bt1886",
            TransferFunction::Gamma22 => "gamma22",
            TransferFunction::Gamma28 => "gamma28",
            TransferFunction::St240 => "st240",
            TransferFunction::ExtSrgb => "ext_srgb",
            TransferFunction::Log100 => "log_100",
            TransferFunction::Log316 => "log_316",
            TransferFunction::St428 => "st428",
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
    primaries: NamedPrimaries,
    tf: TransferFunction,
    enable_luminance: bool,
    luminance: Luminance,

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
            primaries: NamedPrimaries::Srgb,
            tf: TransferFunction::Srgb,
            enable_luminance: false,
            luminance: Default::default(),
            scene: Default::default(),
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

fn draw_egui(ctx: &Context, test_pane: &TestPane, config: &mut ControlPaneConfig) {
    CentralPanel::default().show(ctx, |ui| {
        ComboBox::from_label("View")
            .selected_text(config.view)
            .show_ui(ui, |ui| {
                for s in View::variants() {
                    ui.selectable_value(&mut config.view, s, s);
                }
            });
        ui.add_space(10.0);
        match config.view {
            View::Scenes => draw_scenes(ui, config),
            View::Settings => draw_settings(ui, config),
            View::ColorDescription => draw_color_description(ui, test_pane, config),
        }
    });
}

fn draw_color_description(ui: &mut Ui, test_pane: &TestPane, config: &mut ControlPaneConfig) {
    ui.label("Changing these settings should not affect the output.");
    ui.add_space(20.0);
    ComboBox::from_label("Type")
        .selected_text(config.cd_type)
        .show_ui(ui, |ui| {
            let mut val = |ty: ColorDescriptionType| {
                ui.selectable_value(&mut config.cd_type, ty, ty);
            };
            val(ColorDescriptionType::None);
            if test_pane
                .features
                .contains(&WpColorManagerV1Feature::WINDOWS_SCRGB)
            {
                val(ColorDescriptionType::ScRgb);
            }
            if test_pane
                .features
                .contains(&WpColorManagerV1Feature::PARAMETRIC)
                && test_pane.primaries.is_not_empty()
                && test_pane.tf.is_not_empty()
            {
                val(ColorDescriptionType::Parametric);
            }
        });
    ui.add_space(20.0);
    if config.cd_type == ColorDescriptionType::Parametric {
        ComboBox::from_label("Primaries")
            .selected_text(config.primaries)
            .show_ui(ui, |ui| {
                for primary in NamedPrimaries::variants() {
                    if test_pane.primaries.contains(&primary.wayland()) {
                        ui.selectable_value(&mut config.primaries, primary, primary);
                    }
                }
            });
        ComboBox::from_label("Transfer function")
            .selected_text(config.tf)
            .show_ui(ui, |ui| {
                for tf in TransferFunction::variants() {
                    if test_pane.tf.contains(&tf.wayland()) {
                        ui.selectable_value(&mut config.tf, tf, tf);
                    }
                }
            });
        if test_pane
            .features
            .contains(&WpColorManagerV1Feature::SET_LUMINANCES)
        {
            ui.checkbox(&mut config.enable_luminance, "Luminance");
            if config.enable_luminance {
                ui.horizontal_top(|ui| {
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
                });
            }
        }
    }
}

fn draw_settings(ui: &mut Ui, config: &mut ControlPaneConfig) {
    Slider::new(&mut config.max_lumen, 0.0..=10000.0)
        .prefix("Max lumen: ")
        .drag_value_speed(10.0)
        .ui(ui);
    Slider::new(&mut config.max_chroma, 0.0..=10.0)
        .prefix("Max chroma: ")
        .drag_value_speed(0.1)
        .ui(ui);
}

fn draw_scenes(ui: &mut Ui, config: &mut ControlPaneConfig) {
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
    let colors = |ui: &mut Ui, colors: &mut [(&str, &mut Color)]| {
        Grid::new("colors").spacing([20.0, 20.0]).show(ui, |ui| {
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
