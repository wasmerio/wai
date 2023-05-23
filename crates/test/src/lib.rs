
#[allow(clippy::all)]
pub mod simple_lists {
  #[allow(unused_imports)]
  use wai_bindgen_wasmer::{anyhow, wasmer};
  pub trait SimpleLists: Sized + Send + Sync + 'static{
    fn simple_list1(&mut self,l: &[Le<u32>],) -> ();
    
    fn simple_list2(&mut self,) -> Vec<u32>;
    
    fn simple_list4(&mut self,l: Vec<&[Le<u32>]>,) -> Vec<Vec<u32>>;
    
  }
  pub struct LazyInitialized {
    memory: wasmer::Memory,
    func_canonical_abi_realloc: wasmer::TypedFunction<(i32, i32, i32, i32), i32>,
  }
  
  #[must_use = "The returned initializer function must be called
  with the instance and the store before starting the runtime"]
  pub fn add_to_imports<T>(store: &mut wasmer::Store, imports: &mut wasmer::Imports, data: T)
  -> impl FnOnce(&wasmer::Instance, &dyn wasmer::AsStoreRef) -> Result<(), anyhow::Error>
  where T: SimpleLists
  {
    #[derive(Clone)]struct EnvWrapper<T: SimpleLists> {
      data: T,
      lazy: std::rc::Rc<OnceCell<LazyInitialized>>,
    }
    unsafe impl<T: SimpleLists> Send for EnvWrapper<T> {}
    unsafe impl<T: SimpleLists> Sync for EnvWrapper<T> {}
    let lazy = std::rc::Rc::new(OnceCell::new());
    let env = EnvWrapper {
      data,
      lazy: std::rc::Rc::clone(&lazy),
    };
    let env = wasmer::FunctionEnv::new(&mut *store, env);
    let mut exports = wasmer::Exports::new();
    let mut store = store.as_store_mut();
    exports.insert(
    "simple-list1",
    wasmer::Function::new_typed_with_env(
    &mut store,
    &env,
    move |mut store: wasmer::FunctionEnvMut<EnvWrapper<T>>,arg0:i32,arg1:i32| -> Result<(), wasmer::RuntimeError> {
      let _memory: wasmer::Memory = store.data().lazy.get().unwrap().memory.clone();

      let (data_mut, my_store) = store.data_and_store_mut();

      let _memory_view = _memory.view(&my_store);
      let mut _bc = wai_bindgen_wasmer::BorrowChecker::new(unsafe {
        _memory_view.data_unchecked_mut()
      });
      let ptr0 = arg0;
      let len0 = arg1;
      let param0 = _bc.slice(ptr0, len0)?;
      let host = &mut data_mut.data;
      let result = host.simple_list1(param0, );
      let () = result;
      Ok(())
    }
    ));
    exports.insert(
    "simple-list2",
    wasmer::Function::new_typed_with_env(
    &mut store,
    &env,
    move |mut store: wasmer::FunctionEnvMut<EnvWrapper<T>>,arg0:i32| -> Result<(), wasmer::RuntimeError> {
      let func_canonical_abi_realloc = store
      .data()
      .lazy
      .get()
      .unwrap()
      .func_canonical_abi_realloc
      .clone();
      let _memory: wasmer::Memory = store.data().lazy.get().unwrap().memory.clone();
      let data_mut = store.data_mut();
      let host = &mut data_mut.data;
      let result = host.simple_list2();
      let vec0 = result;
      let ptr0 = func_canonical_abi_realloc.call(&mut store.as_store_mut(), 0, 0, 4, (vec0.len() as i32) * 4)?;
      let _memory_view = _memory.view(&store);
      let caller_memory = unsafe { _memory_view.data_unchecked_mut() };
      caller_memory.store_many(ptr0, &vec0)?;
      caller_memory.store(arg0 + 4, wai_bindgen_wasmer::rt::as_i32(vec0.len() as i32))?;
      caller_memory.store(arg0 + 0, wai_bindgen_wasmer::rt::as_i32(ptr0))?;
      Ok(())
    }
    ));
    exports.insert(
    "simple-list4",
    wasmer::Function::new_typed_with_env(
    &mut store,
    &env,
    move |mut store: wasmer::FunctionEnvMut<EnvWrapper<T>>,arg0:i32,arg1:i32,arg2:i32| -> Result<(), wasmer::RuntimeError> {
      let func_canonical_abi_realloc = store
      .data()
      .lazy
      .get()
      .unwrap()
      .func_canonical_abi_realloc
      .clone();
      let _memory: wasmer::Memory = store.data().lazy.get().unwrap().memory.clone();
      let (data_mut, mut store) = store.data_and_store_mut();
      let _memory_view = _memory.view(&store);
      let mut _bc = wai_bindgen_wasmer::BorrowChecker::new(unsafe {
        _memory_view.data_unchecked_mut()
      });
      let len3 = arg1;
      let base3 = arg0;
      let mut result3 = Vec::with_capacity(len3 as usize);
      for i in 0..len3 {
        let base = base3 + i *8;
        result3.push({
          let load0 = _bc.load::<i32>(base + 0)?;
          let load1 = _bc.load::<i32>(base + 4)?;
          let ptr2 = load0;
          let len2 = load1;
          _bc.slice(ptr2, len2)?
        });
      }
      let param0 = result3;
      let host = &mut data_mut.data;
      let result = host.simple_list4(param0, );
      let vec5 = result;
      let len5 = vec5.len() as i32;
      let result5 = func_canonical_abi_realloc.call(&mut store.as_store_mut(), 0, 0, 4, len5 * 8)?;
      for (i, e) in vec5.into_iter().enumerate() {
        let base = result5 + (i as i32) * 8;
        {
          let vec4 = e;
          let ptr4 = func_canonical_abi_realloc.call(&mut store.as_store_mut(), 0, 0, 4, (vec4.len() as i32) * 4)?;
          let _memory_view = _memory.view(&store);
          let caller_memory = unsafe { _memory_view.data_unchecked_mut() };
          caller_memory.store_many(ptr4, &vec4)?;
          caller_memory.store(base + 4, wai_bindgen_wasmer::rt::as_i32(vec4.len() as i32))?;
          caller_memory.store(base + 0, wai_bindgen_wasmer::rt::as_i32(ptr4))?;
        }}let _memory_view = _memory.view(&store);
        let caller_memory = unsafe { _memory_view.data_unchecked_mut() };
        caller_memory.store(arg2 + 4, wai_bindgen_wasmer::rt::as_i32(len5))?;
        caller_memory.store(arg2 + 0, wai_bindgen_wasmer::rt::as_i32(result5))?;
        Ok(())
      }
      ));
      imports.register_namespace("simple-lists", exports);
      move |_instance: &wasmer::Instance, _store: &dyn wasmer::AsStoreRef| {
        let memory = _instance.exports.get_memory("memory")?.clone();
        let func_canonical_abi_realloc = _instance
        .exports
        .get_typed_function(
        &_store.as_store_ref(),
        "canonical_abi_realloc",
        )
        .unwrap()
        .clone();
        lazy.set(LazyInitialized {
          memory,
          func_canonical_abi_realloc,
        })
        .map_err(|_e| anyhow::anyhow!("Couldn't set lazy initialized data"))?;
        Ok(())
      }
    }
    use wai_bindgen_wasmer::once_cell::unsync::OnceCell;
    #[allow(unused_imports)]
    use wasmer::AsStoreMut as _;
    #[allow(unused_imports)]
    use wasmer::AsStoreRef as _;
    use wai_bindgen_wasmer::rt::RawMem;
    use wai_bindgen_wasmer::Le;
  }
  