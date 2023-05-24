use anyhow::Result;
use wasmer::{Imports, Instance, Module, Store};
use wasmer_wasix::WasiEnvBuilder;

test_helpers::runtime_tests_wasmer!();

pub fn instantiate<T, I>(
    wasm: &str,
    store: &mut Store,
    add_imports: impl FnOnce(&mut Store, &mut Imports) -> I,
    mk_exports: impl FnOnce(&mut Store, &Module, &mut Imports) -> Result<(T, Instance)>,
) -> Result<T>
where
    I: FnOnce(&Instance, &dyn wasmer::AsStoreRef) -> Result<(), anyhow::Error>,
{
    let module = Module::from_file(&*store, wasm)?;

    let wasi_env = WasiEnvBuilder::new("test").finalize(store)?;
    let mut imports = wasi_env
        .import_object(store, &module)
        .unwrap_or(Imports::new());

    let initializer = add_imports(store, &mut imports);

    let (exports, instance) = mk_exports(store, &module, &mut imports)?;

    initializer(&instance, store)?;

    Ok(exports)
}
