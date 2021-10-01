use std::cell::RefCell;
use std::sync::Once;
use witx_bindgen_gen_core::{witx2, Generator};
use witx_bindgen_rust::Handle;

witx_bindgen_rust::export!("./crates/witx-bindgen-demo/demo.witx");
witx_bindgen_rust::import!("./crates/witx-bindgen-demo/browser.witx");

struct Demo;

impl demo::Demo for Demo {}

#[derive(Default)]
pub struct Config {
    js: RefCell<witx_bindgen_gen_js::Opts>,
    c: RefCell<witx_bindgen_gen_c::Opts>,
    rust: RefCell<witx_bindgen_gen_rust_wasm::Opts>,
    wasmtime: RefCell<witx_bindgen_gen_wasmtime::Opts>,
    wasmtime_py: RefCell<witx_bindgen_gen_wasmtime_py::Opts>,
    markdown: RefCell<witx_bindgen_gen_markdown::Opts>,
    wasmer: RefCell<witx_bindgen_gen_wasmer::Opts>,
}

impl demo::Config for Config {
    fn new() -> Handle<Config> {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                browser::error(&info.to_string());
                prev_hook(info);
            }));
        });

        Config::default().into()
    }

    fn render(
        &self,
        lang: demo::Lang,
        witx: String,
        import: bool,
    ) -> Result<Vec<(String, String)>, String> {
        let mut gen: Box<dyn Generator> = match lang {
            demo::Lang::Rust => Box::new(self.rust.borrow().clone().build()),
            demo::Lang::Wasmtime => Box::new(self.wasmtime.borrow().clone().build()),
            demo::Lang::WasmtimePy => Box::new(self.wasmtime_py.borrow().clone().build()),
            demo::Lang::Js => Box::new(self.js.borrow().clone().build()),
            demo::Lang::C => Box::new(self.c.borrow().clone().build()),
            demo::Lang::Markdown => Box::new(self.markdown.borrow().clone().build()),
            demo::Lang::Wasmer => Box::new(self.wasmer.borrow().clone().build()),
        };
        let iface = witx2::Interface::parse("input", &witx).map_err(|e| format!("{:?}", e))?;
        let mut files = Default::default();
        let (imports, exports) = if import {
            (vec![iface], vec![])
        } else {
            (vec![], vec![iface])
        };
        gen.generate_all(&imports, &exports, &mut files);
        Ok(files
            .iter()
            .map(|(name, contents)| (name.to_string(), String::from_utf8_lossy(&contents).into()))
            .collect())
    }

    fn set_rust_unchecked(&self, unchecked: bool) {
        self.rust.borrow_mut().unchecked = unchecked;
    }
    fn set_wasmtime_tracing(&self, tracing: bool) {
        self.wasmtime.borrow_mut().tracing = tracing;
    }
    fn set_wasmtime_custom_error(&self, custom_error: bool) {
        browser::log("custom error");
        self.wasmtime.borrow_mut().custom_error = custom_error;
    }
    fn set_wasmtime_async(&self, async_: demo::WasmtimeAsync) {
        use witx_bindgen_gen_wasmtime::Async;

        self.wasmtime.borrow_mut().async_ = match async_ {
            demo::WasmtimeAsync::All => Async::All,
            demo::WasmtimeAsync::None => Async::None,
            demo::WasmtimeAsync::Only(list) => Async::Only(list.into_iter().collect()),
        };
    }
}
