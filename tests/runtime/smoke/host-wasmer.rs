use anyhow::Result;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

wit_bindgen_wasmer::export!("../../tests/runtime/smoke/imports.wit");

#[derive(Clone)]
pub struct MyImports {
    hit: Arc<AtomicBool>,
}

impl imports::Imports for MyImports {
    fn thunk(&mut self) {
        self.hit.store(true, Ordering::Relaxed);
        println!("in the host");
    }
}

wit_bindgen_wasmer::import!("../../tests/runtime/smoke/exports.wit");

fn run(wasm: &str) -> Result<()> {
    use wasmer::AsStoreMut as _;

    let hit = Arc::new(AtomicBool::new(false));

    let mut store = wasmer::Store::default();

    let exports = crate::instantiate(
        wasm,
        &mut store,
        |store, imports| {
            imports::add_to_imports(
                store,
                imports,
                MyImports { hit: hit.clone() },
            )
        },
        |store, module, imports| {
            exports::Exports::instantiate(
                &mut store.as_store_mut().as_store_mut(),
                module,
                imports,
            )
        },
    )?;

    exports.thunk(&mut store)?;

    assert!(hit.load(Ordering::Relaxed));

    Ok(())
}
