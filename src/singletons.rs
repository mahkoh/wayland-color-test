use {
    crate::protocols::wayland::{
        wl_display::WlDisplay, wl_fixes::WlFixes, wl_registry::WlRegistry,
    },
    std::{cell::RefCell, collections::HashMap},
    wl_client::{proxy, proxy::OwnedProxy},
};

pub struct Singletons {
    wl_registry: WlRegistry,
    map: HashMap<String, (u32, u32)>,
}

impl Singletons {
    pub fn get<P>(&self, min: u32, max: u32) -> P
    where
        P: OwnedProxy,
    {
        match self.get_opt(min, max) {
            Some(p) => p,
            _ => {
                panic!(
                    "Compositor does not support {} ({}..={}) but it is required",
                    P::INTERFACE,
                    min,
                    max,
                );
            }
        }
    }

    pub fn get_opt<P>(&self, min: u32, max: u32) -> Option<P>
    where
        P: OwnedProxy,
    {
        let &(name, comp_max) = self.map.get(P::INTERFACE)?;
        let version = comp_max.min(max);
        if version < min {
            return None;
        }
        Some(self.wl_registry.bind(name, version))
    }
}

impl Drop for Singletons {
    fn drop(&mut self) {
        if let Some(fixes) = self.get_opt::<WlFixes>(1, 1) {
            fixes.destroy_registry(&self.wl_registry);
            fixes.destroy();
        }
        proxy::destroy(&self.wl_registry);
    }
}

pub fn get_singletons(display: &WlDisplay) -> Singletons {
    let map = RefCell::new(HashMap::new());

    let queue = proxy::queue(display);
    let wl_registry = display.get_registry();

    queue.dispatch_scope_blocking(|scope| {
        scope.set_event_handler_local(
            &wl_registry,
            WlRegistry::on_global(|_, name, interface, version| {
                map.borrow_mut()
                    .insert(interface.to_owned(), (name, version));
            }),
        );
        queue.dispatch_roundtrip_blocking().unwrap();
    });

    Singletons {
        wl_registry,
        map: map.into_inner(),
    }
}
