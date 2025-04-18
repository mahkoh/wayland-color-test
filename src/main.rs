use {
    crate::{control_pane::ControlPane, test_pane::TestPane},
    async_io::{Async, Timer},
    egui_winit::winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        platform::pump_events::EventLoopExtPumpEvents,
        window::WindowId,
    },
    futures_util::{select, FutureExt},
    std::{future::pending, os::fd::AsFd, time::Duration},
};

mod cmm;
mod control_pane;
mod ordered_float;
mod protocols;
mod singletons;
mod test_pane;
mod vulkan;

struct WinitApp {
    test_pane: TestPane,
    control_pane: Option<ControlPane>,
}

fn main() {
    async_io::block_on(async {
        async_main().await;
    });
}

async fn async_main() {
    let mut event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = WinitApp {
        test_pane: TestPane::new(&event_loop).await,
        control_pane: None,
    };
    let fd = event_loop.as_fd().try_clone_to_owned().unwrap();
    let fd = Async::new_nonblocking(fd).unwrap();
    loop {
        event_loop.pump_app_events(Some(Duration::ZERO), &mut app);
        app.test_pane.dispatch();
        let control_pane = app.control_pane.as_mut().unwrap();
        if let Some(error_message) = app.test_pane.description_error_message() {
            control_pane.draw_state.error_message = error_message;
            control_pane.need_repaint = true;
        }
        if control_pane.need_repaint {
            control_pane.maybe_run(&app.test_pane);
        }
        let timer = control_pane.repaint_after.map(Timer::at);
        let fut = async move {
            if let Some(timer) = timer {
                timer.await;
            } else {
                pending().await
            }
        };
        select! {
            _ = app.test_pane.wait_for_events().fuse() => { },
            res = fd.readable().fuse() => {
                res.unwrap();
            },
            _ = fut.fuse() => {
                control_pane.repaint_after = None;
                control_pane.need_repaint = true;
                control_pane.maybe_run(&app.test_pane);
            }
        }
    }
}

impl ApplicationHandler for WinitApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.control_pane.is_none() {
            self.control_pane = Some(ControlPane::new(event_loop, &self.test_pane));
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.control_pane
            .as_mut()
            .unwrap()
            .handle_event(window_id, event, &self.test_pane);
    }
}
