export type Result<T, E> = { tag: "ok", val: T } | { tag: "err", val: E };
export type Files = [string, string][];
export type WasmtimeAsync = WasmtimeAsyncAll | WasmtimeAsyncNone | WasmtimeAsyncOnly;
export interface WasmtimeAsyncAll {
  tag: "all",
}
export interface WasmtimeAsyncNone {
  tag: "none",
}
export interface WasmtimeAsyncOnly {
  tag: "only",
  val: string[],
}
/**
* # Variants
* 
* ## `"js"`
* 
* ## `"rust"`
* 
* ## `"wasmtime"`
* 
* ## `"wasmtime-py"`
* 
* ## `"c"`
* 
* ## `"markdown"`
* 
* ## `"spidermonkey"`
* 
* ## `"wasmer"`
* 
* ## `"wasmer-py"`
*/
export type Lang = "js" | "rust" | "wasmtime" | "wasmtime-py" | "c" | "markdown" | "spidermonkey" | "wasmer" | "wasmer-py";
export class Demo {
  
  /**
  * The WebAssembly instance that this class is operating with.
  * This is only available after the `instantiate` method has
  * been called.
  */
  instance: WebAssembly.Instance;
  
  /**
  * Constructs a new instance with internal state necessary to
  * manage a wasm instance.
  *
  * Note that this does not actually instantiate the WebAssembly
  * instance or module, you'll need to call the `instantiate`
  * method below to "activate" this class.
  */
  constructor();
  
  /**
  * This is a low-level method which can be used to add any
  * intrinsics necessary for this instance to operate to an
  * import object.
  *
  * The `import` object given here is expected to be used later
  * to actually instantiate the module this class corresponds to.
  * If the `instantiate` method below actually does the
  * instantiation then there's no need to call this method, but
  * if you're instantiating manually elsewhere then this can be
  * used to prepare the import object for external instantiation.
  */
  addToImports(imports: any): void;
  
  /**
  * Initializes this object with the provided WebAssembly
  * module/instance.
  *
  * This is intended to be a flexible method of instantiating
  * and completion of the initialization of this class. This
  * method must be called before interacting with the
  * WebAssembly object.
  *
  * The first argument to this method is where to get the
  * wasm from. This can be a whole bunch of different types,
  * for example:
  *
  * * A precompiled `WebAssembly.Module`
  * * A typed array buffer containing the wasm bytecode.
  * * A `Promise` of a `Response` which is used with
  *   `instantiateStreaming`
  * * A `Response` itself used with `instantiateStreaming`.
  * * An already instantiated `WebAssembly.Instance`
  *
  * If necessary the module is compiled, and if necessary the
  * module is instantiated. Whether or not it's necessary
  * depends on the type of argument provided to
  * instantiation.
  *
  * If instantiation is performed then the `imports` object
  * passed here is the list of imports used to instantiate
  * the instance. This method may add its own intrinsics to
  * this `imports` object too.
  */
  instantiate(
  module: WebAssembly.Module | BufferSource | Promise<Response> | Response | WebAssembly.Instance,
  imports?: any,
  ): Promise<void>;
}

export class Config {
  // Creates a new strong reference count as a new
  // object.  This is only required if you're also
  // calling `drop` below and want to manually manage
  // the reference count from JS.
  //
  // If you don't call `drop`, you don't need to call
  // this and can simply use the object from JS.
  clone(): Config;
  
  // Explicitly indicate that this JS object will no
  // longer be used. If the internal reference count
  // reaches zero then this will deterministically
  // destroy the underlying wasm object.
  //
  // This is not required to be called from JS. Wasm
  // destructors will be automatically called for you
  // if this is not called using the JS
  // `FinalizationRegistry`.
  //
  // Calling this method does not guarantee that the
  // underlying wasm object is deallocated. Something
  // else (including wasm) may be holding onto a
  // strong reference count.
  drop(): void;
  static new(demo: Demo): Config;
  render(lang: Lang, wai: string, import_: boolean): Result<Files, string>;
  setRustUnchecked(unchecked: boolean): void;
  setWasmtimeTracing(unchecked: boolean): void;
  setWasmtimeAsync(val: WasmtimeAsync): void;
  setWasmtimeCustomError(custom: boolean): void;
  setWasmerTracing(unchecked: boolean): void;
  setWasmerAsync(val: WasmtimeAsync): void;
  setWasmerCustomError(custom: boolean): void;
}
