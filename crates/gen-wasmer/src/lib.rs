use heck::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::mem;
use std::process::{Command, Stdio};
use std::str::FromStr;
use wit_bindgen_gen_core::wit_parser::abi::{
    AbiVariant, Bindgen, Instruction, LiftLower, WasmType,
};
use wit_bindgen_gen_core::{wit_parser::*, Direction, Files, Generator, Source, TypeInfo, Types};
use wit_bindgen_gen_rust::{
    to_rust_ident, wasm_type, FnSig, RustFlagsRepr, RustFunctionGenerator, RustGenerator, TypeMode,
};

#[derive(Default)]
pub struct Wasmer {
    src: Source,
    opts: Opts,
    needs_memory: bool,
    needs_functions: BTreeMap<String, NeededFunction>,
    needs_char_from_i32: bool,
    needs_invalid_variant: bool,
    needs_validate_flags: bool,
    needs_raw_mem: bool,
    needs_bad_int: bool,
    needs_copy_slice: bool,
    needs_buffer_glue: bool,
    needs_le: bool,
    needs_custom_error_to_trap: bool,
    needs_custom_error_to_types: BTreeSet<String>,
    needs_lazy_initialized: bool,
    all_needed_handles: BTreeSet<String>,
    exported_resources: BTreeSet<ResourceId>,
    types: Types,
    guest_imports: HashMap<String, Vec<Import>>,
    guest_exports: HashMap<String, Exports>,
    in_import: bool,
    in_trait: bool,
    trait_name: String,
    sizes: SizeAlign,
}

enum NeededFunction {
    Realloc,
    Free,
}

struct Import {
    is_async: bool,
    name: String,
    trait_signature: String,
    closure: String,
}

#[derive(Default)]
struct Exports {
    fields: BTreeMap<String, (String, String)>,
    funcs: Vec<String>,
}

#[derive(Default, Debug, Clone)]
#[cfg_attr(feature = "structopt", derive(structopt::StructOpt))]
pub struct Opts {
    /// Whether or not `rustfmt` is executed to format generated code.
    #[cfg_attr(feature = "structopt", structopt(long))]
    pub rustfmt: bool,

    /// Whether or not to emit `tracing` macro calls on function entry/exit.
    #[cfg_attr(feature = "structopt", structopt(long))]
    pub tracing: bool,

    /// Indicates which functions should be `async`: `all`, `none`, or a
    /// comma-separated list.
    #[cfg_attr(
        feature = "structopt",
        structopt(long = "async", default_value = "none")
    )]
    pub async_: Async,

    /// A flag to indicate that all trait methods in imports should return a
    /// custom trait-defined error. Applicable for import bindings.
    #[cfg_attr(feature = "structopt", structopt(long))]
    pub custom_error: bool,
}

#[derive(Debug, Clone)]
pub enum Async {
    None,
    All,
    Only(HashSet<String>),
}

impl Async {
    fn includes(&self, name: &str) -> bool {
        match self {
            Async::None => false,
            Async::All => true,
            Async::Only(list) => list.contains(name),
        }
    }

    fn is_none(&self) -> bool {
        match self {
            Async::None => true,
            _ => false,
        }
    }
}

impl Default for Async {
    fn default() -> Async {
        Async::None
    }
}

impl FromStr for Async {
    type Err = String;
    fn from_str(s: &str) -> Result<Async, String> {
        Ok(if s == "all" {
            Async::All
        } else if s == "none" {
            Async::None
        } else {
            Async::Only(s.split(',').map(|s| s.trim().to_string()).collect())
        })
    }
}

impl Opts {
    pub fn build(self) -> Wasmer {
        let mut r = Wasmer::new();
        r.opts = self;
        r
    }
}

enum FunctionRet {
    /// The function return is normal and needs to extra handling.
    Normal,
    /// The function return was wrapped in a `Result` in Rust. The `Ok` variant
    /// is the actual value that will be lowered, and the `Err`, if present,
    /// means that a trap has occurred.
    CustomToTrap,
    /// The function returns a `Result` in both wasm and in Rust, but the
    /// Rust error type is a custom error and must be converted to `err`. The
    /// `ok` variant payload is provided here too.
    CustomToError { ok: Type, err: String },
}

impl Wasmer {
    pub fn new() -> Wasmer {
        Wasmer::default()
    }

    fn abi_variant(dir: Direction) -> AbiVariant {
        // This generator uses a reversed mapping! In the Wasmer host-side
        // bindings, we don't use any extra adapter layer between guest wasm
        // modules and the host. When the guest imports functions using the
        // `GuestImport` ABI, the host directly implements the `GuestImport`
        // ABI, even though the host is *exporting* functions. Similarly, when
        // the guest exports functions using the `GuestExport` ABI, the host
        // directly imports them with the `GuestExport` ABI, even though the
        // host is *importing* functions.
        match dir {
            Direction::Import => AbiVariant::GuestExport,
            Direction::Export => AbiVariant::GuestImport,
        }
    }

    fn print_intrinsics(&mut self) {
        if self.needs_lazy_initialized || !self.exported_resources.is_empty() {
            self.push_str("use wit_bindgen_wasmer::once_cell::unsync::OnceCell;\n");
        }

        self.push_str("#[allow(unused_imports)]\n");
        self.push_str("use wasmer::AsStoreMut as _;\n");
        self.push_str("#[allow(unused_imports)]\n");
        self.push_str("use wasmer::AsStoreRef as _;\n");
        if self.needs_raw_mem {
            self.push_str("use wit_bindgen_wasmer::rt::RawMem;\n");
        }
        if self.needs_char_from_i32 {
            self.push_str("use wit_bindgen_wasmer::rt::char_from_i32;\n");
        }
        if self.needs_invalid_variant {
            self.push_str("use wit_bindgen_wasmer::rt::invalid_variant;\n");
        }
        if self.needs_bad_int {
            self.push_str("use core::convert::TryFrom;\n");
            self.push_str("use wit_bindgen_wasmer::rt::bad_int;\n");
        }
        if self.needs_validate_flags {
            self.push_str("use wit_bindgen_wasmer::rt::validate_flags;\n");
        }
        if self.needs_le {
            self.push_str("use wit_bindgen_wasmer::Le;\n");
        }
        if self.needs_copy_slice {
            self.push_str("use wit_bindgen_wasmer::rt::copy_slice;\n");
        }
    }

    /// Classifies the return value of a function to see if it needs handling
    /// with respect to the `custom_error` configuration option.
    fn classify_fn_ret(&mut self, iface: &Interface, f: &Function) -> FunctionRet {
        if !self.opts.custom_error {
            return FunctionRet::Normal;
        }

        if let Type::Id(id) = &f.result {
            if let TypeDefKind::Expected(e) = &iface.types[*id].kind {
                if let Type::Id(err) = e.err {
                    if let Some(name) = &iface.types[err].name {
                        self.needs_custom_error_to_types.insert(name.clone());
                        return FunctionRet::CustomToError {
                            ok: e.ok,
                            err: name.to_string(),
                        };
                    }
                }
            }
        }

        self.needs_custom_error_to_trap = true;
        FunctionRet::CustomToTrap
    }
}

impl RustGenerator for Wasmer {
    fn default_param_mode(&self) -> TypeMode {
        if self.in_import {
            // The default here is that only leaf values can be borrowed because
            // otherwise lists and such need to be copied into our own memory.
            TypeMode::LeafBorrowed("'a")
        } else {
            // When we're calling wasm exports, however, there's no need to take
            // any ownership of anything from the host so everything is borrowed
            // in the parameter position.
            TypeMode::AllBorrowed("'a")
        }
    }

    fn handle_projection(&self) -> Option<(&'static str, String)> {
        if self.in_import {
            if self.in_trait {
                Some(("Self", self.trait_name.clone()))
            } else {
                Some(("T", self.trait_name.clone()))
            }
        } else {
            None
        }
    }

    fn handle_wrapper(&self) -> Option<&'static str> {
        None
    }

    fn push_str(&mut self, s: &str) {
        self.src.push_str(s);
    }

    fn info(&self, ty: TypeId) -> TypeInfo {
        self.types.get(ty)
    }

    fn types_mut(&mut self) -> &mut Types {
        &mut self.types
    }

    fn print_borrowed_slice(
        &mut self,
        iface: &Interface,
        mutbl: bool,
        ty: &Type,
        lifetime: &'static str,
    ) {
        if self.sizes.align(ty) > 1 && self.in_import {
            // If we're generating bindings for an import we ideally want to
            // hand out raw pointers into memory. We can't guarantee anything
            // about alignment in memory, though, so if the alignment
            // requirement is bigger than one then we have to use slices where
            // the type has a `Le<...>` wrapper.
            //
            // For exports we're generating functions that take values from
            // Rust, so we can assume alignment and use raw slices. For types
            // with an align of 1, then raw pointers are fine since Rust will
            // have the same alignment requirement.
            self.needs_le = true;
            self.push_str("&");
            if lifetime != "'_" {
                self.push_str(lifetime);
                self.push_str(" ");
            }
            if mutbl {
                self.push_str(" mut ");
            }
            self.push_str("[Le<");
            self.print_ty(iface, ty, TypeMode::AllBorrowed(lifetime));
            self.push_str(">]");
        } else {
            self.print_rust_slice(iface, mutbl, ty, lifetime);
        }
    }

    fn print_borrowed_str(&mut self, lifetime: &'static str) {
        self.push_str("&");
        if lifetime != "'_" {
            self.push_str(lifetime);
            self.push_str(" ");
        }
        self.push_str(" str");
    }
}

impl Generator for Wasmer {
    fn preprocess_one(&mut self, iface: &Interface, dir: Direction) {
        let variant = Self::abi_variant(dir);
        self.types.analyze(iface);
        self.in_import = variant == AbiVariant::GuestImport;
        self.trait_name = iface.name.to_camel_case();
        self.src.push_str(&format!(
            "#[allow(clippy::all)]\npub mod {} {{\n",
            iface.name.to_snake_case()
        ));
        self.src
            .push_str("#[allow(unused_imports)]\nuse wit_bindgen_wasmer::{anyhow, wasmer};\n");
        self.sizes.fill(iface);
    }

    fn type_record(
        &mut self,
        iface: &Interface,
        id: TypeId,
        name: &str,
        record: &Record,
        docs: &Docs,
    ) {
        self.print_typedef_record(iface, id, record, docs);

        // If this record might be used as a slice type in various places then
        // we synthesize an `Endian` implementation for it so `&[Le<ThisType>]`
        // is usable.
        if self.modes_of(iface, id).len() > 0
            && record.fields.iter().all(|f| iface.all_bits_valid(&f.ty))
        {
            self.src.push_str("impl wit_bindgen_wasmer::Endian for ");
            self.src.push_str(&name.to_camel_case());
            self.src.push_str(" {\n");

            self.src.push_str("fn into_le(self) -> Self {\n");
            self.src.push_str("Self {\n");
            for field in record.fields.iter() {
                self.src.push_str(&field.name.to_snake_case());
                self.src.push_str(": self.");
                self.src.push_str(&field.name.to_snake_case());
                self.src.push_str(".into_le(),\n");
            }
            self.src.push_str("}\n");
            self.src.push_str("}\n");

            self.src.push_str("fn from_le(self) -> Self {\n");
            self.src.push_str("Self {\n");
            for field in record.fields.iter() {
                self.src.push_str(&field.name.to_snake_case());
                self.src.push_str(": self.");
                self.src.push_str(&field.name.to_snake_case());
                self.src.push_str(".from_le(),\n");
            }
            self.src.push_str("}\n");
            self.src.push_str("}\n");

            self.src.push_str("}\n");

            // Also add an `AllBytesValid` valid impl since this structure's
            // byte representations are valid (guarded by the `all_bits_valid`
            // predicate).
            self.src
                .push_str("unsafe impl wit_bindgen_wasmer::AllBytesValid for ");
            self.src.push_str(&name.to_camel_case());
            self.src.push_str(" {}\n");
        }
    }

    fn type_tuple(
        &mut self,
        iface: &Interface,
        id: TypeId,
        _name: &str,
        tuple: &Tuple,
        docs: &Docs,
    ) {
        self.print_typedef_tuple(iface, id, tuple, docs);
    }

    fn type_flags(
        &mut self,
        _iface: &Interface,
        _id: TypeId,
        name: &str,
        flags: &Flags,
        docs: &Docs,
    ) {
        self.src
            .push_str("wit_bindgen_wasmer::bitflags::bitflags! {\n");
        self.rustdoc(docs);
        let repr = RustFlagsRepr::new(flags);
        self.src
            .push_str(&format!("pub struct {}: {repr} {{", name.to_camel_case()));
        for (i, flag) in flags.flags.iter().enumerate() {
            self.rustdoc(&flag.docs);
            self.src.push_str(&format!(
                "const {} = 1 << {};\n",
                flag.name.to_shouty_snake_case(),
                i,
            ));
        }
        self.src.push_str("}\n");
        self.src.push_str("}\n\n");

        self.src.push_str("impl core::fmt::Display for ");
        self.src.push_str(&name.to_camel_case());
        self.src.push_str(
            "{\nfn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {\n",
        );

        self.src.push_str("f.write_str(\"");
        self.src.push_str(&name.to_camel_case());
        self.src.push_str("(\")?;\n");
        self.src.push_str("core::fmt::Debug::fmt(self, f)?;\n");
        self.src.push_str("f.write_str(\" (0x\")?;\n");
        self.src
            .push_str("core::fmt::LowerHex::fmt(&self.bits, f)?;\n");
        self.src.push_str("f.write_str(\"))\")?;\n");
        self.src.push_str("Ok(())");

        self.src.push_str("}\n");
        self.src.push_str("}\n\n");
    }

    fn type_variant(
        &mut self,
        iface: &Interface,
        id: TypeId,
        _name: &str,
        variant: &Variant,
        docs: &Docs,
    ) {
        self.print_typedef_variant(iface, id, variant, docs);
    }

    fn type_enum(&mut self, _iface: &Interface, id: TypeId, name: &str, enum_: &Enum, docs: &Docs) {
        self.print_typedef_enum(id, name, enum_, docs);
    }

    fn type_union(
        &mut self,
        iface: &Interface,
        id: TypeId,
        _name: &str,
        union: &Union,
        docs: &Docs,
    ) {
        self.print_typedef_union(iface, id, union, docs);
    }

    fn type_option(
        &mut self,
        iface: &Interface,
        id: TypeId,
        _name: &str,
        payload: &Type,
        docs: &Docs,
    ) {
        self.print_typedef_option(iface, id, payload, docs);
    }

    fn type_expected(
        &mut self,
        iface: &Interface,
        id: TypeId,
        _name: &str,
        expected: &Expected,
        docs: &Docs,
    ) {
        self.print_typedef_expected(iface, id, expected, docs);
    }

    fn type_resource(&mut self, iface: &Interface, ty: ResourceId) {
        let name = &iface.resources[ty].name;
        self.all_needed_handles.insert(name.to_string());

        // If we're binding imports then all handles are associated types so
        // there's nothing that we need to do about that.
        if self.in_import {
            return;
        }

        self.exported_resources.insert(ty);

        // ... otherwise for exports we generate a newtype wrapper around an
        // `i32` to manage the resultt.
        let tyname = name.to_camel_case();
        self.rustdoc(&iface.resources[ty].docs);
        self.src.push_str("#[derive(Debug)]\n");
        self.src.push_str(&format!(
            "pub struct {}(wit_bindgen_wasmer::rt::ResourceIndex);\n",
            tyname
        ));
    }

    fn type_alias(&mut self, iface: &Interface, id: TypeId, _name: &str, ty: &Type, docs: &Docs) {
        self.print_typedef_alias(iface, id, ty, docs);
    }

    fn type_list(&mut self, iface: &Interface, id: TypeId, _name: &str, ty: &Type, docs: &Docs) {
        self.print_type_list(iface, id, ty, docs);
    }

    fn type_builtin(&mut self, iface: &Interface, _id: TypeId, name: &str, ty: &Type, docs: &Docs) {
        self.rustdoc(docs);
        self.src
            .push_str(&format!("pub type {}", name.to_camel_case()));
        self.src.push_str(" = ");
        self.print_ty(iface, ty, TypeMode::Owned);
        self.src.push_str(";\n");
    }

    // As with `abi_variant` above, we're generating host-side bindings here
    // so a user "export" uses the "guest import" ABI variant on the inside of
    // this `Generator` implementation.
    fn export(&mut self, iface: &Interface, func: &Function) {
        assert!(!func.is_async, "async not supported yet");
        let prev = mem::take(&mut self.src);

        // Generate the closure that's passed to a `Linker`, the final piece of
        // codegen here.
        let sig = iface.wasm_signature(AbiVariant::GuestImport, func);
        let params = (0..sig.params.len())
            .map(|i| format!("arg{}", i))
            .collect::<Vec<_>>();
        let mut f = FunctionBindgen::new(self, params);
        iface.call(
            AbiVariant::GuestImport,
            LiftLower::LiftArgsLowerResults,
            func,
            &mut f,
        );
        let FunctionBindgen {
            src,
            cleanup,
            needs_borrow_checker,
            needs_memory,
            needs_buffer_transaction,
            needs_functions,
            closures,
            async_intrinsic_called,
            ..
        } = f;
        assert!(cleanup.is_none());
        assert!(!needs_buffer_transaction);

        // Generate the signature this function will have in the final trait
        let self_arg = "&mut self".to_string();
        self.in_trait = true;

        let mut fnsig = FnSig::default();
        fnsig.private = true;
        fnsig.self_arg = Some(self_arg);
        self.print_docs_and_params(iface, func, TypeMode::LeafBorrowed("'_"), &fnsig);
        // The Rust return type may differ from the wasm return type based on
        // the `custom_error` configuration of this code generator.
        match self.classify_fn_ret(iface, func) {
            FunctionRet::Normal => {
                self.push_str(" -> ");
                self.print_ty(iface, &func.result, TypeMode::Owned);
            }
            FunctionRet::CustomToTrap => {
                self.push_str(" -> Result<");
                self.print_ty(iface, &func.result, TypeMode::Owned);
                self.push_str(", Self::Error>");
            }
            FunctionRet::CustomToError { ok, .. } => {
                self.push_str(" -> Result<");
                self.print_ty(iface, &ok, TypeMode::Owned);
                self.push_str(", Self::Error>");
            }
        }
        self.in_trait = false;
        let trait_signature = mem::take(&mut self.src).into();

        // Generate the closure that's passed to a `Linker`, the final piece of
        // codegen here.
        let result_ty = match &sig.results[..] {
            &[] => format!("()"),
            &[ty] => format!("{}", wasm_type(ty)),
            tys => format!(
                "({})",
                tys.iter()
                    .map(|&ty| wasm_type(ty))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        self.src
            .push_str("move |mut store: wasmer::FunctionEnvMut<EnvWrapper<T>>");
        for (i, param) in sig.params.iter().enumerate() {
            let arg = format!("arg{}", i);
            self.src.push_str(",");
            self.src.push_str(&arg);
            self.src.push_str(":");
            self.wasm_type(*param);
        }
        self.src.push_str(&format!(
            "| -> Result<{}, wasmer::RuntimeError> {{\n",
            result_ty
        ));

        // If an intrinsic was called asynchronously, which happens if anything
        // in the module could be asynchronous, then we must wrap this host
        // import with an async block. Otherwise if the function is itself
        // explicitly async then we must also wrap it in an async block.
        //
        // If none of that happens, then this is fine to be sync because
        // everything is sync.
        let is_async = if async_intrinsic_called || self.opts.async_.includes(&func.name) {
            self.src.push_str("Box::new(async move {\n");
            true
        } else {
            false
        };

        if self.opts.tracing {
            self.src.push_str(&format!(
                "
                    let span = wit_bindgen_wasmer::tracing::span!(
                        wit_bindgen_wasmer::tracing::Level::TRACE,
                        \"wit-bindgen abi\",
                        module = \"{}\",
                        function = \"{}\",
                    );
                    let _enter = span.enter();
                ",
                iface.name, func.name,
            ));
        }
        self.src.push_str(&closures);

        for name in needs_functions.keys() {
            self.src.push_str(&format!(
                "let func_{name} = store
                    .data()
                    .lazy
                    .get()
                    .unwrap()
                    .func_{name}
                    .clone();\n"
            ));
        }
        self.needs_functions.extend(needs_functions);
        self.needs_memory |= needs_memory || needs_borrow_checker;

        if self.needs_memory {
            self.src.push_str(
                "let _memory: wasmer::Memory = store.data().lazy.get().unwrap().memory.clone();\n",
            );
        }

        if needs_borrow_checker {
            // TODO: This isn't actually sound and should be replaced with use
            // of WasmPtr/WasmCell.
            self.src.push_str(
                "let mut _bc = wit_bindgen_wasmer::BorrowChecker::new(unsafe {
                        _memory.data_unchecked_mut(&store)
                 });\n",
            );
        }

        self.src.push_str("let data_mut = store.data_mut();\n");

        if self.all_needed_handles.len() > 0 {
            self.src
                .push_str("let tables = data_mut.tables.borrow_mut();\n");
        }

        self.src.push_str(&String::from(src));

        if is_async {
            self.src.push_str("})\n");
        }
        self.src.push_str("}");
        let closure = mem::replace(&mut self.src, prev).into();

        self.guest_imports
            .entry(iface.name.to_string())
            .or_insert(Vec::new())
            .push(Import {
                is_async,
                name: func.name.to_string(),
                closure,
                trait_signature,
            });
    }

    // As with `abi_variant` above, we're generating host-side bindings here
    // so a user "import" uses the "export" ABI variant on the inside of
    // this `Generator` implementation.
    fn import(&mut self, iface: &Interface, func: &Function) {
        assert!(!func.is_async, "async not supported yet");
        let prev = mem::take(&mut self.src);

        // If anything is asynchronous on exports then everything must be
        // asynchronous, we can't intermix async and sync calls because
        // it's unknown whether the wasm module will make an async host call.
        let is_async = !self.opts.async_.is_none();
        let mut sig = FnSig::default();
        sig.async_ = is_async;

        // Adding the store to the self_arg is an ugly workaround, but the
        // FnSig and Function types don't really leave a lot of room for
        // implementing this in a better way.
        sig.self_arg = Some("&self, store: &mut wasmer::Store".to_string());
        self.print_docs_and_params(iface, func, TypeMode::AllBorrowed("'_"), &sig);
        self.push_str("-> Result<");
        self.print_ty(iface, &func.result, TypeMode::Owned);
        self.push_str(", wasmer::RuntimeError> {\n");

        let params = func
            .params
            .iter()
            .map(|(name, _)| to_rust_ident(name).to_string())
            .collect();
        let mut f = FunctionBindgen::new(self, params);
        iface.call(
            AbiVariant::GuestExport,
            LiftLower::LowerArgsLiftResults,
            func,
            &mut f,
        );
        let FunctionBindgen {
            needs_memory,
            src,
            needs_borrow_checker,
            needs_buffer_transaction,
            closures,
            needs_functions,
            ..
        } = f;

        let exports = self
            .guest_exports
            .entry(iface.name.to_string())
            .or_insert_with(Exports::default);

        for (name, func) in needs_functions {
            self.src
                .push_str(&format!("let func_{name} = &self.func_{name};\n"));
            let get = format!("_instance.exports.get_typed_function(store, \"{name}\")?",);
            exports
                .fields
                .insert(format!("func_{name}"), (func.ty(), get));
        }

        self.src.push_str(&closures);

        assert!(!needs_borrow_checker);
        if needs_memory {
            self.src.push_str("let _memory = &self.memory;\n");
            exports.fields.insert(
                "memory".to_string(),
                (
                    "wasmer::Memory".to_string(),
                    "_instance.exports.get_memory(\"memory\")?.clone()".to_string(),
                ),
            );
        }

        if needs_buffer_transaction {
            self.needs_buffer_glue = true;
            self.src
                .push_str("let mut buffer_transaction = self.buffer_glue.transaction();\n");
        }

        self.src.push_str(&String::from(src));
        self.src.push_str("}\n");
        let func_body = mem::replace(&mut self.src, prev);
        exports.funcs.push(func_body.into());

        // Create the code snippet which will define the type of this field in
        // the struct that we're exporting and additionally extracts the
        // function from an instantiated instance.
        let sig = iface.wasm_signature(AbiVariant::GuestExport, func);
        let mut cvt = String::new();
        if sig.params.len() == 1 {
            cvt.push_str(wasm_type(sig.params[0]));
        } else {
            cvt.push_str("(");
            for param in sig.params.iter() {
                cvt.push_str(wasm_type(*param));
                cvt.push_str(",");
            }
            cvt.push_str(")");
        }
        cvt.push_str(", ");
        if sig.results.len() == 1 {
            cvt.push_str(wasm_type(sig.results[0]));
        } else {
            cvt.push_str("(");
            for result in sig.results.iter() {
                cvt.push_str(wasm_type(*result));
                cvt.push_str(",");
            }
            cvt.push_str(")");
        }
        exports.fields.insert(
            format!("func_{}", to_rust_ident(&func.name)),
            (
                format!("wasmer::TypedFunction<{cvt}>"),
                format!(
                    "_instance.exports.get_typed_function(store, \"{}\")?",
                    func.name,
                ),
            ),
        );
    }

    fn finish_one(&mut self, iface: &Interface, files: &mut Files) {
        for (module, funcs) in sorted_iter(&self.guest_imports) {
            let module_camel = module.to_camel_case();
            let is_async = !self.opts.async_.is_none();
            if is_async {
                self.src.push_str("#[wit_bindgen_wasmer::async_trait]\n");
            }
            self.src.push_str("pub trait ");
            self.src.push_str(&module_camel);
            self.src.push_str(": Sized + Send + Sync + 'static");
            self.src.push_str("{\n");
            if self.all_needed_handles.len() > 0 {
                for handle in self.all_needed_handles.iter() {
                    self.src.push_str("type ");
                    self.src.push_str(&handle.to_camel_case());
                    self.src.push_str(": std::fmt::Debug");
                    if is_async {
                        self.src.push_str(" + Send + Sync");
                    }
                    self.src.push_str(";\n");
                }
            }
            if self.opts.custom_error {
                self.src.push_str("type Error;\n");
                if self.needs_custom_error_to_trap {
                    self.src.push_str(
                        "fn error_to_trap(&mut self, err: Self::Error) -> wasmer::RuntimeError;\n",
                    );
                }
                for ty in self.needs_custom_error_to_types.iter() {
                    self.src.push_str(&format!(
                        "fn error_to_{}(&mut self, err: Self::Error) -> Result<{}, wasmer::RuntimeError>;\n",
                        ty.to_snake_case(),
                        ty.to_camel_case(),
                    ));
                }
            }
            for f in funcs {
                self.src.push_str(&f.trait_signature);
                self.src.push_str(";\n\n");
            }
            for handle in self.all_needed_handles.iter() {
                self.src.push_str(&format!(
                    "fn drop_{}(&mut self, state: Self::{}) {{
                        drop(state);
                    }}\n",
                    handle.to_snake_case(),
                    handle.to_camel_case(),
                ));
            }
            self.src.push_str("}\n");

            if self.all_needed_handles.len() > 0 {
                self.src.push_str("\npub struct ");
                self.src.push_str(&module_camel);
                self.src.push_str("Tables<T: ");
                self.src.push_str(&module_camel);
                self.src.push_str("> {\n");
                for handle in self.all_needed_handles.iter() {
                    self.src.push_str("pub(crate) ");
                    self.src.push_str(&handle.to_snake_case());
                    self.src.push_str("_table: wit_bindgen_wasmer::Table<T::");
                    self.src.push_str(&handle.to_camel_case());
                    self.src.push_str(">,\n");
                }
                self.src.push_str("}\n");
                self.src.push_str("impl<T: ");
                self.src.push_str(&module_camel);
                self.src.push_str("> Default for ");
                self.src.push_str(&module_camel);
                self.src.push_str("Tables<T> {\n");
                self.src.push_str("fn default() -> Self { Self {");
                for handle in self.all_needed_handles.iter() {
                    self.src.push_str(&handle.to_snake_case());
                    self.src.push_str("_table: Default::default(),");
                }
                self.src.push_str("}}}");
                self.src.push_str("impl<T: ");
                self.src.push_str(&module_camel);
                self.src.push_str("> Clone for ");
                self.src.push_str(&module_camel);
                self.src.push_str("Tables<T> {\n");
                self.src.push_str("fn clone(&self) -> Self {\n");
                self.src.push_str("Self::default()\n");
                self.src.push_str("}}\n");
            }
        }

        self.needs_lazy_initialized |= self.needs_memory;
        self.needs_lazy_initialized |= !self.needs_functions.is_empty();
        for (module, funcs) in mem::take(&mut self.guest_imports) {
            let module_camel = module.to_camel_case();

            if self.needs_lazy_initialized {
                self.push_str("pub struct LazyInitialized {\n");
                if self.needs_memory {
                    self.push_str("memory: wasmer::Memory,\n");
                }
                for (name, func) in &self.needs_functions {
                    self.src.push_str(&format!(
                        "func_{name}: wasmer::TypedFunction<{cvt}>,\n",
                        name = name,
                        cvt = func.cvt(),
                    ));
                }
                self.push_str("}\n");
            }

            self.push_str("\n#[must_use = \"The returned initializer function must be called\n");
            self.push_str("with the instance and the store before starting the runtime\"]\n");
            self.push_str("pub fn add_to_imports<T>(store: &mut wasmer::Store, imports: &mut wasmer::Imports, data: T)\n");
            self.push_str("-> impl FnOnce(&wasmer::Instance, &dyn wasmer::AsStoreRef) -> Result<(), anyhow::Error>\n");
            self.push_str("where T: ");
            self.push_str(&module_camel);
            self.push_str("\n{\n");

            self.push_str("#[derive(Clone)]");
            self.push_str("struct EnvWrapper<T: ");
            self.push_str(&module_camel);
            self.push_str("> {\n");
            self.push_str("data: T,\n");
            if !self.all_needed_handles.is_empty() {
                self.push_str("tables: std::rc::Rc<core::cell::RefCell<");
                self.push_str(&module_camel);
                self.push_str("Tables<T>>>,\n");
            }
            if self.needs_lazy_initialized {
                self.push_str("lazy: std::rc::Rc<OnceCell<LazyInitialized>>,\n");
            }
            self.push_str("}\n");
            self.push_str("unsafe impl<T: ");
            self.push_str(&module_camel);
            self.push_str("> Send for EnvWrapper<T> {}\n");
            self.push_str("unsafe impl<T: ");
            self.push_str(&module_camel);
            self.push_str("> Sync for EnvWrapper<T> {}\n");

            if self.needs_lazy_initialized {
                self.push_str("let lazy = std::rc::Rc::new(OnceCell::new());\n");
            }

            self.push_str("let env = EnvWrapper {\n");
            self.push_str("data,\n");
            if self.all_needed_handles.len() > 0 {
                self.push_str("tables: std::rc::Rc::default(),\n");
            }
            if self.needs_lazy_initialized {
                self.push_str("lazy: std::rc::Rc::clone(&lazy),\n");
            }
            self.push_str("};\n");
            self.push_str("let env = wasmer::FunctionEnv::new(&mut *store, env);\n");
            self.push_str("let mut exports = wasmer::Exports::new();\n");
            self.push_str("let mut store = store.as_store_mut();\n");

            for f in funcs {
                if f.is_async {
                    unimplemented!();
                }
                self.push_str(&format!(
                    "exports.insert(
                        \"{}\",
                        wasmer::Function::new_native(
                            &mut store,
                            &env,
                            {}
                    ));\n",
                    f.name, f.closure,
                ));
            }
            self.push_str(&format!(
                "imports.register_namespace(\"{}\", exports);\n",
                module
            ));

            if !self.all_needed_handles.is_empty() {
                self.push_str("let mut canonical_abi = imports.get_namespace_exports(\"canonical_abi\").unwrap_or_else(wasmer::Exports::new);\n");
                for handle in self.all_needed_handles.iter() {
                    self.src.push_str(&format!(
                        "canonical_abi.insert(
                            \"resource_drop_{name}\",
                            wasmer::Function::new_native(
                                &mut store,
                                &env,
                                move |mut store: wasmer::FunctionEnvMut<EnvWrapper<T>>, handle: u32| -> Result<(), wasmer::RuntimeError> {{
                                    let data_mut = store.data_mut();
                                    let mut tables = data_mut.tables.borrow_mut();
                                    let handle = tables
                                        .{snake}_table
                                        .remove(handle)
                                        .map_err(|e| {{
                                            wasmer::RuntimeError::new(format!(\"failed to remove handle: {{}}\", e))
                                        }})?;
                                    let host = &mut data_mut.data;
                                    host.drop_{snake}(handle);
                                    Ok(())
                                }}
                            )
                        );\n",
                        name = handle,
                        snake = handle.to_snake_case(),
                    ));
                }
                self.push_str("imports.register_namespace(\"canonical_abi\", canonical_abi);\n");
            }

            self.push_str(
                "move |_instance: &wasmer::Instance, _store: &dyn wasmer::AsStoreRef| {\n",
            );
            if self.needs_lazy_initialized {
                if self.needs_memory {
                    self.push_str(
                        "let memory = _instance.exports.get_memory(\"memory\")?.clone();\n",
                    );
                }
                for name in self.needs_functions.keys() {
                    self.src.push_str(&format!(
                        "let func_{name} = _instance
                        .exports
                        .get_typed_function(
                            &_store.as_store_ref(),
                            \"{name}\",
                        )
                        .unwrap()
                        .clone();\n"
                    ));
                }
                self.push_str("lazy.set(LazyInitialized {\n");
                if self.needs_memory {
                    self.push_str("memory,\n");
                }
                for name in self.needs_functions.keys() {
                    self.src.push_str(&format!("func_{name},\n"));
                }
                self.push_str("})\n");
                self.push_str(
                    ".map_err(|_e| anyhow::anyhow!(\"Couldn't set lazy initialized data\"))?;\n",
                );
            }
            self.push_str("Ok(())\n");
            self.push_str("}\n");

            self.push_str("}\n");
        }

        for (module, exports) in sorted_iter(&mem::take(&mut self.guest_exports)) {
            let name = module.to_camel_case();

            // Generate a struct that is the "state" of this exported module
            // which is held internally.
            self.push_str(
                "
                /// Auxiliary data associated with the wasm exports.
                ",
            );
            self.push_str("#[derive(Default)]\n");
            self.push_str("pub struct ");
            self.push_str(&name);
            self.push_str("Data {\n");
            for r in self.exported_resources.iter() {
                self.src.push_str(&format!(
                    "
                        index_slab{idx}: wit_bindgen_wasmer::rt::IndexSlab,
                        resource_slab{idx}: wit_bindgen_wasmer::rt::ResourceSlab,
                        dtor{idx}: OnceCell<wasmer::TypedFunction<i32, ()>>,
                    ",
                    idx = r.index(),
                ));
            }
            self.push_str("}\n\n");

            self.push_str("pub struct ");
            self.push_str(&name);
            self.push_str(" {\n");
            self.push_str("#[allow(dead_code)]\n");
            self.push_str(&format!("env: wasmer::FunctionEnv<{}Data>,\n", name));
            for (name, (ty, _)) in exports.fields.iter() {
                self.push_str(name);
                self.push_str(": ");
                self.push_str(ty);
                self.push_str(",\n");
            }
            self.push_str("}\n");
            self.push_str(&format!("impl {} {{\n", name));

            if self.exported_resources.len() == 0 {
                self.push_str("#[allow(unused_variables)]\n");
            }
            self.push_str(&format!(
                "
                    /// Adds any intrinsics, if necessary for this exported wasm
                    /// functionality to the `ImportObject` provided.
                    ///
                    /// This function returns the `{0}Data` which needs to be
                    /// passed through to `{0}::new`.
                    fn add_to_imports(
                        store: &mut wasmer::StoreMut<'_>,
                        imports: &mut wasmer::Imports,
                    ) -> wasmer::FunctionEnv<{0}Data> {{
                ",
                name,
            ));
            self.push_str("let env = wasmer::FunctionEnv::new(store, Default::default());\n");
            if !self.all_needed_handles.is_empty() {
                self.push_str("let mut canonical_abi = imports.get_namespace_exports(\"canonical_abi\").unwrap_or_else(wasmer::Exports::new);\n");
                for r in self.exported_resources.iter() {
                    if !self.opts.async_.is_none() {
                        unimplemented!();
                    }
                    self.src.push_str(&format!(
                        "
                        canonical_abi.insert(
                            \"resource_drop_{resource}\",
                            wasmer::Function::new_native(
                                store,
                                &env,
                                move |mut store: wasmer::FunctionEnvMut<{name}Data>, idx: u32| -> Result<(), wasmer::RuntimeError> {{
                                    let resource_idx = store.data_mut().index_slab{idx}.remove(idx)?;
                                    let wasm = match store.data_mut().resource_slab{idx}.drop(resource_idx) {{
                                        Some(wasm) => wasm,
                                        None => return Ok(()),
                                    }};
                                    let dtor = store.data_mut().dtor{idx}.get().unwrap().clone();
                                    dtor.call(&mut store, wasm)?;
                                    Ok(())
                                }},
                            )
                        );
                        canonical_abi.insert(
                            \"resource_clone_{resource}\",
                            wasmer::Function::new_native(
                                store,
                                &env,
                                move |mut store: wasmer::FunctionEnvMut<{name}Data>, idx: u32| -> Result<u32, wasmer::RuntimeError>  {{
                                    let state = &mut *store.data_mut();
                                    let resource_idx = state.index_slab{idx}.get(idx)?;
                                    state.resource_slab{idx}.clone(resource_idx)?;
                                    Ok(state.index_slab{idx}.insert(resource_idx))
                                }},
                            )
                        );
                        canonical_abi.insert(
                            \"resource_get_{resource}\",
                            wasmer::Function::new_native(
                                store,
                                &env,
                                move |mut store: wasmer::FunctionEnvMut<{name}Data>, idx: u32| -> Result<i32, wasmer::RuntimeError>  {{
                                    let state = &mut *store.data_mut();
                                    let resource_idx = state.index_slab{idx}.get(idx)?;
                                    Ok(state.resource_slab{idx}.get(resource_idx))
                                }},
                            )
                        );
                        canonical_abi.insert(
                            \"resource_new_{resource}\",
                            wasmer::Function::new_native(
                                store,
                                &env,
                                move |mut store: wasmer::FunctionEnvMut<{name}Data>, val: i32| -> Result<u32, wasmer::RuntimeError>  {{
                                    let state = &mut *store.data_mut();
                                    let resource_idx = state.resource_slab{idx}.insert(val);
                                    Ok(state.index_slab{idx}.insert(resource_idx))
                                }},
                            )
                        );
                    ",
                        name = name,
                        resource = iface.resources[*r].name,
                        idx = r.index(),
                    ));
                }
                self.push_str("imports.register_namespace(\"canonical_abi\", canonical_abi);\n");
            }
            self.push_str("env\n");
            self.push_str("}\n");

            if !self.opts.async_.is_none() {
                unimplemented!();
            }
            self.push_str(&format!(
                "
                    /// Instantiates the provided `module` using the specified
                    /// parameters, wrapping up the result in a structure that
                    /// translates between wasm and the host.
                    ///
                    /// The `imports` provided will have intrinsics added to it
                    /// automatically, so it's not necessary to call
                    /// `add_to_imports` beforehand. This function will
                    /// instantiate the `module` otherwise using `imports`, and
                    /// both an instance of this structure and the underlying
                    /// `wasmer::Instance` will be returned.
                    pub fn instantiate(
                        store: &mut wasmer::StoreMut<'_>,
                        module: &wasmer::Module,
                        imports: &mut wasmer::Imports,
                    ) -> anyhow::Result<(Self, wasmer::Instance)> {{
                        let env = Self::add_to_imports(
                            &mut store.as_store_mut().as_store_mut(),
                            imports,
                        );
                        let instance = wasmer::Instance::new(
                            &mut store.as_store_mut(),
                            module,
                            &*imports,
                        )?;
                        "
            ));
            if !self.exported_resources.is_empty() {
                self.push_str("{\n");
                for r in self.exported_resources.iter() {
                    self.src.push_str(&format!(
                        "let dtor{idx} = instance
                                .exports
                                .get_typed_function(
                                    store,
                                    \"canonical_abi_drop_{name}\",
                                )?
                                .clone();
                                ",
                        name = iface.resources[*r].name,
                        idx = r.index(),
                    ));
                }
                self.push_str("\n");

                for r in self.exported_resources.iter() {
                    self.src.push_str(&format!(
                            "env
                                .as_mut(store)
                                .dtor{idx}
                                .set(dtor{idx})
                                .map_err(|_e| anyhow::anyhow!(\"Couldn't set canonical_abi_drop_{name}\"))?;
                                ",
                                name = iface.resources[*r].name,
                                idx = r.index(),
                                ));
                }
                self.push_str("}\n");
            }
            self.push_str(&format!(
                "
                        Ok((Self::new(store, &instance, env)?, instance))
                    }}
                ",
            ));

            self.push_str(&format!(
                "
                    /// Low-level creation wrapper for wrapping up the exports
                    /// of the `instance` provided in this structure of wasm
                    /// exports.
                    ///
                    /// This function will extract exports from the `instance`
                    /// and wrap them all up in the returned structure which can
                    /// be used to interact with the wasm module.
                    pub fn new(
                        store: &mut wasmer::StoreMut<'_>,
                        _instance: &wasmer::Instance,
                        env: wasmer::FunctionEnv<{}Data>,
                    ) -> Result<Self, wasmer::ExportError> {{
                ",
                name,
            ));
            //assert!(!self.needs_get_func);
            for (name, (_, get)) in exports.fields.iter() {
                self.push_str("let ");
                self.push_str(&name);
                self.push_str("= ");
                self.push_str(&get);
                self.push_str(";\n");
            }
            self.push_str("Ok(");
            self.push_str(&name);
            self.push_str("{\n");
            for (name, _) in exports.fields.iter() {
                self.push_str(name);
                self.push_str(",\n");
            }
            self.push_str("env,\n");
            self.push_str("})\n");
            self.push_str("}\n");

            for func in exports.funcs.iter() {
                self.push_str(func);
            }

            for r in self.exported_resources.iter() {
                if !self.opts.async_.is_none() {
                    unimplemented!();
                }
                self.src.push_str(&format!(
                    "
                        /// Drops the host-owned handle to the resource
                        /// specified.
                        ///
                        /// Note that this may execute the WebAssembly-defined
                        /// destructor for this type. This also may not run
                        /// the destructor if there are still other references
                        /// to this type.
                        pub fn drop_{name_snake}(
                            &self,
                            store: &mut wasmer::Store,
                            val: {name_camel},
                        ) -> Result<(), wasmer::RuntimeError> {{
                            let state = self.env.as_mut(store);
                            let wasm = match state.resource_slab{idx}.drop(val.0) {{
                                Some(val) => val,
                                None => return Ok(()),
                            }};
                            let dtor{idx} = state.dtor{idx}.get().unwrap().clone();
                            dtor{idx}.call(store, wasm)?;
                            Ok(())
                        }}
                    ",
                    name_snake = iface.resources[*r].name.to_snake_case(),
                    name_camel = iface.resources[*r].name.to_camel_case(),
                    idx = r.index(),
                ));
            }

            self.push_str("}\n");
        }
        self.print_intrinsics();

        // Close the opening `mod`.
        self.push_str("}\n");

        let mut src = mem::take(&mut self.src);
        if self.opts.rustfmt {
            let mut child = Command::new("rustfmt")
                .arg("--edition=2018")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("failed to spawn `rustfmt`");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(src.as_bytes())
                .unwrap();
            src.as_mut_string().truncate(0);
            child
                .stdout
                .take()
                .unwrap()
                .read_to_string(src.as_mut_string())
                .unwrap();
            let status = child.wait().unwrap();
            assert!(status.success());
        }

        files.push("bindings.rs", src.as_bytes());
    }
}

struct FunctionBindgen<'a> {
    gen: &'a mut Wasmer,

    // Number used to assign unique names to temporary variables.
    tmp: usize,

    // Destination where source code is pushed onto for this function
    src: Source,

    // The named parameters that are available to this function
    params: Vec<String>,

    // Management of block scopes used by `Bindgen`.
    block_storage: Vec<Source>,
    blocks: Vec<String>,

    // Whether or not the code generator is after the invocation of wasm or the
    // host, used for knowing where to acquire memory from.
    after_call: bool,
    // Whether or not the `caller_memory` variable has been defined and is
    // available for use.
    caller_memory_available: bool,
    // Whether or not a helper function was called in an async fashion. If so
    // and this is an import, then the import must be defined asynchronously as
    // well.
    async_intrinsic_called: bool,
    // Code that must be executed before a return, generated during instruction
    // lowering.
    cleanup: Option<String>,

    // Rust clousures for buffers that must be placed at the front of the
    // function.
    closures: Source,

    // Various intrinsic properties this function's codegen required, must be
    // satisfied in the function header if any are set.
    needs_buffer_transaction: bool,
    needs_borrow_checker: bool,
    needs_memory: bool,
    needs_functions: HashMap<String, NeededFunction>,
}

impl FunctionBindgen<'_> {
    fn new(gen: &mut Wasmer, params: Vec<String>) -> FunctionBindgen<'_> {
        FunctionBindgen {
            gen,
            block_storage: Vec::new(),
            blocks: Vec::new(),
            src: Source::default(),
            after_call: false,
            caller_memory_available: false,
            async_intrinsic_called: false,
            tmp: 0,
            cleanup: None,
            closures: Source::default(),
            needs_buffer_transaction: false,
            needs_borrow_checker: false,
            needs_memory: false,
            needs_functions: HashMap::new(),
            params,
        }
    }

    fn memory_src(&mut self) -> String {
        if self.gen.in_import {
            if !self.after_call {
                // Before calls we use `_bc` which is a borrow checker used for
                // getting long-lasting borrows into memory.
                self.needs_borrow_checker = true;
                return format!("_bc");
            }

            if !self.caller_memory_available {
                self.needs_memory = true;
                self.caller_memory_available = true;
                self.push_str("let caller_memory = unsafe { _memory.data_unchecked_mut(&store.as_store_ref()) };\n");
            }
            format!("caller_memory")
        } else {
            self.needs_memory = true;
            format!("unsafe {{ _memory.data_unchecked_mut(&store.as_store_ref()) }}")
        }
    }

    fn call_intrinsic(&mut self, name: &str, args: String) {
        if !self.gen.opts.async_.is_none() {
            self.async_intrinsic_called = true;
            unimplemented!();
        };
        self.push_str(&format!("func_{name}.call({args})?;\n"));
        self.caller_memory_available = false; // invalidated by call
    }

    fn load(&mut self, offset: i32, ty: &str, operands: &[String]) -> String {
        let mem = self.memory_src();
        self.gen.needs_raw_mem = true;
        let tmp = self.tmp();
        self.push_str(&format!(
            "let load{} = {}.load::<{}>({} + {})?;\n",
            tmp, mem, ty, operands[0], offset
        ));
        format!("load{}", tmp)
    }

    fn store(&mut self, offset: i32, method: &str, extra: &str, operands: &[String]) {
        let mem = self.memory_src();
        self.gen.needs_raw_mem = true;
        self.push_str(&format!(
            "{}.store({} + {}, wit_bindgen_wasmer::rt::{}({}){})?;\n",
            mem, operands[1], offset, method, operands[0], extra
        ));
    }
}

impl RustFunctionGenerator for FunctionBindgen<'_> {
    fn push_str(&mut self, s: &str) {
        self.src.push_str(s);
    }

    fn tmp(&mut self) -> usize {
        let ret = self.tmp;
        self.tmp += 1;
        ret
    }

    fn rust_gen(&self) -> &dyn RustGenerator {
        self.gen
    }

    fn lift_lower(&self) -> LiftLower {
        if self.gen.in_import {
            LiftLower::LiftArgsLowerResults
        } else {
            LiftLower::LowerArgsLiftResults
        }
    }
}

impl Bindgen for FunctionBindgen<'_> {
    type Operand = String;

    fn sizes(&self) -> &SizeAlign {
        &self.gen.sizes
    }

    fn push_block(&mut self) {
        let prev = mem::take(&mut self.src);
        self.block_storage.push(prev);
    }

    fn finish_block(&mut self, operands: &mut Vec<String>) {
        let to_restore = self.block_storage.pop().unwrap();
        let src = mem::replace(&mut self.src, to_restore);
        let expr = match operands.len() {
            0 => "()".to_string(),
            1 => operands[0].clone(),
            _ => format!("({})", operands.join(", ")),
        };
        if src.is_empty() {
            self.blocks.push(expr);
        } else if operands.is_empty() {
            self.blocks.push(format!("{{\n{}}}", &src[..]));
        } else {
            self.blocks.push(format!("{{\n{}{}\n}}", &src[..], expr));
        }
        self.caller_memory_available = false;
    }

    fn return_pointer(&mut self, _iface: &Interface, _size: usize, _align: usize) -> String {
        unimplemented!()
    }

    fn is_list_canonical(&self, iface: &Interface, ty: &Type) -> bool {
        iface.all_bits_valid(ty)
    }

    fn emit(
        &mut self,
        iface: &Interface,
        inst: &Instruction<'_>,
        operands: &mut Vec<String>,
        results: &mut Vec<String>,
    ) {
        let mut top_as = |cvt: &str| {
            let mut s = operands.pop().unwrap();
            s.push_str(" as ");
            s.push_str(cvt);
            results.push(s);
        };

        let mut try_from = |cvt: &str, operands: &[String], results: &mut Vec<String>| {
            self.gen.needs_bad_int = true;
            let result = format!("{}::try_from({}).map_err(bad_int)?", cvt, operands[0]);
            results.push(result);
        };

        match inst {
            Instruction::GetArg { nth } => results.push(self.params[*nth].clone()),
            Instruction::I32Const { val } => results.push(format!("{}i32", val)),
            Instruction::ConstZero { tys } => {
                for ty in tys.iter() {
                    match ty {
                        WasmType::I32 => results.push("0i32".to_string()),
                        WasmType::I64 => results.push("0i64".to_string()),
                        WasmType::F32 => results.push("0.0f32".to_string()),
                        WasmType::F64 => results.push("0.0f64".to_string()),
                    }
                }
            }

            Instruction::I64FromU64 | Instruction::I64FromS64 => {
                let s = operands.pop().unwrap();
                results.push(format!("wit_bindgen_wasmer::rt::as_i64({})", s));
            }
            Instruction::I32FromChar
            | Instruction::I32FromU8
            | Instruction::I32FromS8
            | Instruction::I32FromU16
            | Instruction::I32FromS16
            | Instruction::I32FromU32
            | Instruction::I32FromS32 => {
                let s = operands.pop().unwrap();
                results.push(format!("wit_bindgen_wasmer::rt::as_i32({})", s));
            }

            Instruction::F32FromFloat32
            | Instruction::F64FromFloat64
            | Instruction::Float32FromF32
            | Instruction::Float64FromF64
            | Instruction::S32FromI32
            | Instruction::S64FromI64 => {
                results.push(operands.pop().unwrap());
            }

            // Downcasts from `i32` into smaller integers are checked to ensure
            // that they fit within the valid range. While not strictly
            // necessary since we could chop bits off this should be more
            // forward-compatible with any future changes.
            Instruction::S8FromI32 => try_from("i8", operands, results),
            Instruction::U8FromI32 => try_from("u8", operands, results),
            Instruction::S16FromI32 => try_from("i16", operands, results),
            Instruction::U16FromI32 => try_from("u16", operands, results),

            // Casts of the same bit width simply use `as` since we're just
            // reinterpreting the bits already there.
            Instruction::U32FromI32 => top_as("u32"),
            Instruction::U64FromI64 => top_as("u64"),

            Instruction::CharFromI32 => {
                self.gen.needs_char_from_i32 = true;
                results.push(format!("char_from_i32({})?", operands[0]));
            }

            Instruction::Bitcasts { casts } => {
                wit_bindgen_gen_rust::bitcast(casts, operands, results)
            }

            Instruction::UnitLower => {
                self.push_str(&format!("let () = {};\n", operands[0]));
            }
            Instruction::UnitLift => {
                results.push("()".to_string());
            }

            Instruction::I32FromBool => {
                results.push(format!("match {} {{ true => 1, false => 0 }}", operands[0]));
            }
            Instruction::BoolFromI32 => {
                self.gen.needs_invalid_variant = true;
                results.push(format!(
                    "match {} {{
                        0 => false,
                        1 => true,
                        _ => return Err(invalid_variant(\"bool\")),
                    }}",
                    operands[0],
                ));
            }

            Instruction::I32FromOwnedHandle { ty } => {
                let name = &iface.resources[*ty].name;
                results.push(format!(
                    "{{
                        let data_mut = store.data_mut();
                        let mut tables = data_mut.tables.borrow_mut();
                        tables.{}_table.insert({}) as i32
                    }}",
                    name.to_snake_case(),
                    operands[0]
                ));
            }
            Instruction::HandleBorrowedFromI32 { ty } => {
                let name = &iface.resources[*ty].name;
                results.push(format!(
                    "tables
                        .{}_table
                        .get(({}) as u32)
                        .ok_or_else(|| {{
                            wasmer::RuntimeError::new(\"invalid handle index\")
                        }})?",
                    name.to_snake_case(),
                    operands[0]
                ));
            }
            Instruction::I32FromBorrowedHandle { ty } => {
                let tmp = self.tmp();
                self.push_str(&format!(
                    "
                        let obj{tmp} = {op};
                        let handle{tmp} = {{
                            let state = self.env.as_mut(store);
                            state.resource_slab{idx}.clone(obj{tmp}.0)?;
                            state.index_slab{idx}.insert(obj{tmp}.0)
                        }};
                    ",
                    tmp = tmp,
                    idx = ty.index(),
                    op = operands[0],
                ));

                results.push(format!("handle{} as i32", tmp,));
            }
            Instruction::HandleOwnedFromI32 { ty } => {
                let tmp = self.tmp();
                self.push_str(&format!(
                    "let state = self.env.as_mut(store);
                    let handle{} = state.index_slab{}.remove({} as u32)?;\n",
                    tmp,
                    ty.index(),
                    operands[0],
                ));

                let name = iface.resources[*ty].name.to_camel_case();
                results.push(format!("{}(handle{})", name, tmp));
            }

            Instruction::RecordLower { ty, record, .. } => {
                self.record_lower(iface, *ty, record, &operands[0], results);
            }
            Instruction::RecordLift { ty, record, .. } => {
                self.record_lift(iface, *ty, record, operands, results);
            }

            Instruction::TupleLower { tuple, .. } => {
                self.tuple_lower(tuple, &operands[0], results);
            }
            Instruction::TupleLift { .. } => {
                self.tuple_lift(operands, results);
            }

            Instruction::FlagsLower { flags, .. } => {
                let tmp = self.tmp();
                self.push_str(&format!("let flags{} = {};\n", tmp, operands[0]));
                for i in 0..flags.repr().count() {
                    results.push(format!("(flags{}.bits >> {}) as i32", tmp, i * 32));
                }
            }
            Instruction::FlagsLift { flags, name, .. } => {
                self.gen.needs_validate_flags = true;
                let repr = RustFlagsRepr::new(flags);
                let mut flags = String::from("0");
                for (i, op) in operands.iter().enumerate() {
                    flags.push_str(&format!("| (({} as {repr}) << {})", op, i * 32));
                }
                results.push(format!(
                    "validate_flags(
                        {},
                        {name}::all().bits(),
                        \"{name}\",
                        |bits| {name} {{ bits }}
                    )?",
                    flags,
                    name = name.to_camel_case(),
                ));
            }

            Instruction::VariantPayloadName => results.push("e".to_string()),

            Instruction::VariantLower {
                variant,
                results: result_types,
                ty,
                ..
            } => {
                let blocks = self
                    .blocks
                    .drain(self.blocks.len() - variant.cases.len()..)
                    .collect::<Vec<_>>();
                self.let_results(result_types.len(), results);
                let op0 = &operands[0];
                self.push_str(&format!("match {op0} {{\n"));
                let name = self.typename_lower(iface, *ty);
                for (case, block) in variant.cases.iter().zip(blocks) {
                    let case_name = case.name.to_camel_case();
                    self.push_str(&format!("{name}::{case_name}"));
                    if case.ty == Type::Unit {
                        self.push_str(&format!(" => {{\nlet e = ();\n{block}\n}}\n"));
                    } else {
                        self.push_str(&format!("(e) => {block},\n"));
                    }
                }
                self.push_str("};\n");
            }

            Instruction::VariantLift { variant, ty, .. } => {
                let blocks = self
                    .blocks
                    .drain(self.blocks.len() - variant.cases.len()..)
                    .collect::<Vec<_>>();
                let op0 = &operands[0];
                let mut result = format!("match {op0} {{\n");
                let name = self.typename_lift(iface, *ty);
                for (i, (case, block)) in variant.cases.iter().zip(blocks).enumerate() {
                    let block = if case.ty != Type::Unit {
                        format!("({block})")
                    } else {
                        String::new()
                    };
                    let case = case.name.to_camel_case();
                    result.push_str(&format!("{i} => {name}::{case}{block},\n"));
                }
                result.push_str(&format!("_ => return Err(invalid_variant(\"{name}\")),\n"));
                result.push_str("}");
                results.push(result);
                self.gen.needs_invalid_variant = true;
            }

            Instruction::UnionLower {
                union,
                results: result_types,
                ty,
                ..
            } => {
                let blocks = self
                    .blocks
                    .drain(self.blocks.len() - union.cases.len()..)
                    .collect::<Vec<_>>();
                self.let_results(result_types.len(), results);
                let op0 = &operands[0];
                self.push_str(&format!("match {op0} {{\n"));
                let name = self.typename_lower(iface, *ty);
                for (case_name, block) in self
                    .gen
                    .union_case_names(iface, union)
                    .into_iter()
                    .zip(blocks)
                {
                    self.push_str(&format!("{name}::{case_name}(e) => {block},\n"));
                }
                self.push_str("};\n");
            }

            Instruction::UnionLift { union, ty, .. } => {
                let blocks = self
                    .blocks
                    .drain(self.blocks.len() - union.cases.len()..)
                    .collect::<Vec<_>>();
                let op0 = &operands[0];
                let mut result = format!("match {op0} {{\n");
                let name = self.typename_lift(iface, *ty);
                for (i, (case_name, block)) in self
                    .gen
                    .union_case_names(iface, union)
                    .into_iter()
                    .zip(blocks)
                    .enumerate()
                {
                    result.push_str(&format!("{i} => {name}::{case_name}({block}),\n"));
                }
                result.push_str(&format!("_ => return Err(invalid_variant(\"{name}\")),\n"));
                result.push_str("}");
                results.push(result);
            }

            Instruction::OptionLower {
                results: result_types,
                ..
            } => {
                let some = self.blocks.pop().unwrap();
                let none = self.blocks.pop().unwrap();
                self.let_results(result_types.len(), results);
                let operand = &operands[0];
                self.push_str(&format!(
                    "match {operand} {{
                        Some(e) => {some},
                        None => {{\nlet e = ();\n{none}\n}},
                    }};"
                ));
            }

            Instruction::OptionLift { .. } => {
                let some = self.blocks.pop().unwrap();
                let none = self.blocks.pop().unwrap();
                assert_eq!(none, "()");
                let operand = &operands[0];
                results.push(format!(
                    "match {operand} {{
                        0 => None,
                        1 => Some({some}),
                        _ => return Err(invalid_variant(\"option\")),
                    }}"
                ));
                self.gen.needs_invalid_variant = true;
            }

            Instruction::ExpectedLower {
                results: result_types,
                ..
            } => {
                let err = self.blocks.pop().unwrap();
                let ok = self.blocks.pop().unwrap();
                self.let_results(result_types.len(), results);
                let operand = &operands[0];
                self.push_str(&format!(
                    "match {operand} {{
                        Ok(e) => {{ {ok} }},
                        Err(e) => {{ {err} }},
                    }};"
                ));
            }

            Instruction::ExpectedLift { .. } => {
                let err = self.blocks.pop().unwrap();
                let ok = self.blocks.pop().unwrap();
                let operand = &operands[0];
                results.push(format!(
                    "match {operand} {{
                        0 => Ok({ok}),
                        1 => Err({err}),
                        _ => return Err(invalid_variant(\"expected\")),
                    }}"
                ));
                self.gen.needs_invalid_variant = true;
            }

            Instruction::EnumLower { .. } => {
                results.push(format!("{} as i32", operands[0]));
            }

            Instruction::EnumLift { name, enum_, .. } => {
                let op0 = &operands[0];
                let mut result = format!("match {op0} {{\n");
                let name = name.to_camel_case();
                for (i, case) in enum_.cases.iter().enumerate() {
                    let case = case.name.to_camel_case();
                    result.push_str(&format!("{i} => {name}::{case},\n"));
                }
                result.push_str(&format!("_ => return Err(invalid_variant(\"{name}\")),\n"));
                result.push_str("}");
                results.push(result);
                self.gen.needs_invalid_variant = true;
            }

            Instruction::ListCanonLower { element, realloc } => {
                // Lowering only happens when we're passing lists into wasm,
                // which forces us to always allocate, so this should always be
                // `Some`.
                let realloc = realloc.unwrap();
                self.needs_functions
                    .insert(realloc.to_string(), NeededFunction::Realloc);
                let (size, align) = (self.gen.sizes.size(element), self.gen.sizes.align(element));

                // Store the operand into a temporary...
                let tmp = self.tmp();
                let val = format!("vec{}", tmp);
                self.push_str(&format!("let {} = {};\n", val, operands[0]));

                // ... and then realloc space for the result in the guest module
                let ptr = format!("ptr{}", tmp);
                self.push_str(&format!("let {} = ", ptr));
                self.call_intrinsic(
                    realloc,
                    format!(
                        "&mut store.as_store_mut(), 0, 0, {}, ({}.len() as i32) * {}",
                        align, val, size
                    ),
                );

                // ... and then copy over the result.
                let mem = self.memory_src();
                self.push_str(&format!("{}.store_many({}, &{})?;\n", mem, ptr, val));
                self.gen.needs_raw_mem = true;
                self.needs_memory = true;
                results.push(ptr);
                results.push(format!("{}.len() as i32", val));
            }

            Instruction::ListCanonLift { element, free, .. } => match free {
                Some(free) => {
                    self.needs_memory = true;
                    self.gen.needs_copy_slice = true;
                    self.needs_functions
                        .insert(free.to_string(), NeededFunction::Free);
                    let align = self.gen.sizes.align(element);
                    let tmp = self.tmp();
                    self.push_str(&format!("let ptr{} = {};\n", tmp, operands[0]));
                    self.push_str(&format!("let len{} = {};\n", tmp, operands[1]));
                    let result = format!(
                        "
                                copy_slice(
                                    store,
                                    _memory,
                                    func_{},
                                    ptr{tmp}, len{tmp}, {}
                                )?
                            ",
                        free,
                        align,
                        tmp = tmp
                    );
                    results.push(result);
                }
                None => {
                    self.needs_borrow_checker = true;
                    let tmp = self.tmp();
                    self.push_str(&format!("let ptr{} = {};\n", tmp, operands[0]));
                    self.push_str(&format!("let len{} = {};\n", tmp, operands[1]));
                    let slice = format!("_bc.slice(ptr{0}, len{0})?", tmp);
                    results.push(slice);
                }
            },

            Instruction::StringLower { realloc } => {
                // see above for this unwrap
                let realloc = realloc.unwrap();
                self.needs_functions
                    .insert(realloc.to_string(), NeededFunction::Realloc);

                // Store the operand into a temporary...
                let tmp = self.tmp();
                let val = format!("vec{}", tmp);
                self.push_str(&format!("let {} = {};\n", val, operands[0]));

                // ... and then realloc space for the result in the guest module
                let ptr = format!("ptr{}", tmp);
                self.push_str(&format!("let {} = ", ptr));
                self.call_intrinsic(
                    realloc,
                    format!("&mut store.as_store_mut(), 0, 0, 1, {}.len() as i32", val),
                );

                // ... and then copy over the result.
                let mem = self.memory_src();
                self.push_str(&format!(
                    "{}.store_many({}, {}.as_bytes())?;\n",
                    mem, ptr, val
                ));
                self.gen.needs_raw_mem = true;
                self.needs_memory = true;
                results.push(ptr);
                results.push(format!("{}.len() as i32", val));
            }

            Instruction::StringLift { free } => match free {
                Some(free) => {
                    self.needs_memory = true;
                    self.gen.needs_copy_slice = true;
                    self.needs_functions
                        .insert(free.to_string(), NeededFunction::Free);
                    let tmp = self.tmp();
                    self.push_str(&format!("let ptr{} = {};\n", tmp, operands[0]));
                    self.push_str(&format!("let len{} = {};\n", tmp, operands[1]));
                    self.push_str(&format!(
                        "
                            let data{tmp} = copy_slice(
                                store,
                                _memory,
                                func_{},
                                ptr{tmp}, len{tmp}, 1,
                            )?;
                        ",
                        free,
                        tmp = tmp,
                    ));
                    results.push(format!(
                        "String::from_utf8(data{})
                            .map_err(|_| wasmer::RuntimeError::new(\"invalid utf-8\"))?",
                        tmp,
                    ));
                }
                None => {
                    self.needs_borrow_checker = true;
                    let tmp = self.tmp();
                    self.push_str(&format!("let ptr{} = {};\n", tmp, operands[0]));
                    self.push_str(&format!("let len{} = {};\n", tmp, operands[1]));
                    let slice = format!("_bc.slice_str(ptr{0}, len{0})?", tmp);
                    results.push(slice);
                }
            },

            Instruction::ListLower { element, realloc } => {
                let realloc = realloc.unwrap();
                let body = self.blocks.pop().unwrap();
                let tmp = self.tmp();
                let vec = format!("vec{}", tmp);
                let result = format!("result{}", tmp);
                let len = format!("len{}", tmp);
                self.needs_functions
                    .insert(realloc.to_string(), NeededFunction::Realloc);
                let size = self.gen.sizes.size(element);
                let align = self.gen.sizes.align(element);

                // first store our vec-to-lower in a temporary since we'll
                // reference it multiple times.
                self.push_str(&format!("let {} = {};\n", vec, operands[0]));
                self.push_str(&format!("let {} = {}.len() as i32;\n", len, vec));

                // ... then realloc space for the result in the guest module
                self.push_str(&format!("let {} = ", result));
                self.call_intrinsic(
                    realloc,
                    format!(
                        "&mut store.as_store_mut(), 0, 0, {}, {} * {}",
                        align, len, size
                    ),
                );

                // ... then consume the vector and use the block to lower the
                // result.
                self.push_str(&format!(
                    "for (i, e) in {}.into_iter().enumerate() {{\n",
                    vec
                ));
                self.push_str(&format!("let base = {} + (i as i32) * {};\n", result, size));
                self.push_str(&body);
                self.push_str("}");

                results.push(result);
                results.push(len);
            }

            Instruction::ListLift { element, free, .. } => {
                let body = self.blocks.pop().unwrap();
                let tmp = self.tmp();
                let size = self.gen.sizes.size(element);
                let align = self.gen.sizes.align(element);
                let len = format!("len{}", tmp);
                self.push_str(&format!("let {} = {};\n", len, operands[1]));
                let base = format!("base{}", tmp);
                self.push_str(&format!("let {} = {};\n", base, operands[0]));
                let result = format!("result{}", tmp);
                self.push_str(&format!(
                    "let mut {} = Vec::with_capacity({} as usize);\n",
                    result, len,
                ));

                self.push_str("for i in 0..");
                self.push_str(&len);
                self.push_str(" {\n");
                self.push_str("let base = ");
                self.push_str(&base);
                self.push_str(" + i *");
                self.push_str(&size.to_string());
                self.push_str(";\n");
                self.push_str(&result);
                self.push_str(".push(");
                self.push_str(&body);
                self.push_str(");\n");
                self.push_str("}\n");
                results.push(result);

                if let Some(free) = free {
                    self.call_intrinsic(
                        free,
                        format!(
                            "&mut store.as_store_mut(), {}, {} * {}, {}",
                            base, len, size, align
                        ),
                    );
                    self.needs_functions
                        .insert(free.to_string(), NeededFunction::Free);
                }
            }

            Instruction::IterElem { .. } => {
                self.caller_memory_available = false; // invalidated by for loop
                results.push("e".to_string())
            }

            Instruction::IterBasePointer => results.push("base".to_string()),

            Instruction::CallWasm {
                iface: _,
                name,
                sig,
            } => {
                if sig.results.len() > 0 {
                    let tmp = self.tmp();
                    if sig.results.len() == 1 {
                        self.push_str("let ");
                        let arg = format!("result{}", tmp);
                        self.push_str(&arg);
                        results.push(arg);
                        self.push_str(" = ");
                    } else {
                        self.push_str("let (");
                        for i in 0..sig.results.len() {
                            let arg = format!("result{}_{}", tmp, i);
                            self.push_str(&arg);
                            self.push_str(",");
                            results.push(arg);
                        }
                        self.push_str(") = ");
                    }
                }
                self.push_str("self.func_");
                self.push_str(&to_rust_ident(name));
                if self.gen.opts.async_.includes(name) {
                    self.push_str(".call_async(store, ");
                } else {
                    self.push_str(".call(store, ");
                }
                for operand in operands {
                    self.push_str(operand);
                    self.push_str(", ");
                }
                self.push_str(")");
                if self.gen.opts.async_.includes(name) {
                    self.push_str(".await");
                }
                self.push_str("?;\n");
                self.after_call = true;
                self.caller_memory_available = false; // invalidated by call
            }

            Instruction::CallWasmAsyncImport { .. } => unimplemented!(),
            Instruction::CallWasmAsyncExport { .. } => unimplemented!(),

            Instruction::CallInterface { module: _, func } => {
                for (i, operand) in operands.iter().enumerate() {
                    self.push_str(&format!("let param{} = {};\n", i, operand));
                }
                if self.gen.opts.tracing && func.params.len() > 0 {
                    self.push_str("wit_bindgen_wasmer::tracing::event!(\n");
                    self.push_str("wit_bindgen_wasmer::tracing::Level::TRACE,\n");
                    for (i, (name, _ty)) in func.params.iter().enumerate() {
                        self.push_str(&format!(
                            "{} = wit_bindgen_wasmer::tracing::field::debug(&param{}),\n",
                            to_rust_ident(name),
                            i
                        ));
                    }
                    self.push_str(");\n");
                }

                let mut call = format!("host.{}(", func.name.to_snake_case());
                for i in 0..operands.len() {
                    call.push_str(&format!("param{}, ", i));
                }
                call.push_str(")");
                if self.gen.opts.async_.includes(&func.name) {
                    call.push_str(".await");
                }

                self.push_str("let host = &mut data_mut.data;\n");
                self.push_str("let result = ");
                results.push("result".to_string());
                match self.gen.classify_fn_ret(iface, func) {
                    FunctionRet::Normal => self.push_str(&call),
                    // Unwrap the result, translating errors to unconditional
                    // traps
                    FunctionRet::CustomToTrap => {
                        self.push_str("match ");
                        self.push_str(&call);
                        self.push_str("{\n");
                        self.push_str("Ok(val) => val,\n");
                        self.push_str("Err(e) => return Err(host.error_to_trap(e)),\n");
                        self.push_str("}");
                    }
                    // Keep the `Result` as a `Result`, but convert the error
                    // to either the expected destination value or a trap,
                    // propagating a trap outwards.
                    FunctionRet::CustomToError { err, .. } => {
                        self.push_str("match ");
                        self.push_str(&call);
                        self.push_str("{\n");
                        self.push_str("Ok(val) => Ok(val),\n");
                        self.push_str(&format!("Err(e) => Err(host.error_to_{}(e)?),\n", err));
                        self.push_str("}");
                    }
                }
                self.push_str(";\n");

                if self.gen.all_needed_handles.len() > 0 {
                    self.push_str("drop(tables);\n");
                }

                self.after_call = true;

                match &func.result {
                    Type::Unit => {}
                    _ if self.gen.opts.tracing => {
                        self.push_str("wit_bindgen_wasmer::tracing::event!(\n");
                        self.push_str("wit_bindgen_wasmer::tracing::Level::TRACE,\n");
                        self.push_str(&format!(
                            "{} = wit_bindgen_wasmer::tracing::field::debug(&{0}),\n",
                            results[0],
                        ));
                        self.push_str(");\n");
                    }
                    _ => {}
                }
            }

            Instruction::Return { amt, .. } => {
                let result = match amt {
                    0 => format!("Ok(())\n"),
                    1 => format!("Ok({})\n", operands[0]),
                    _ => format!("Ok(({}))\n", operands.join(", ")),
                };
                match self.cleanup.take() {
                    Some(cleanup) => {
                        self.push_str("let ret = ");
                        self.push_str(&result);
                        self.push_str(";\n");
                        self.push_str(&cleanup);
                        self.push_str("ret");
                    }
                    None => self.push_str(&result),
                }
            }

            Instruction::ReturnAsyncExport { .. } => unimplemented!(),
            Instruction::ReturnAsyncImport { .. } => unimplemented!(),

            Instruction::I32Load { offset } => results.push(self.load(*offset, "i32", operands)),
            Instruction::I32Load8U { offset } => {
                results.push(format!("i32::from({})", self.load(*offset, "u8", operands)));
            }
            Instruction::I32Load8S { offset } => {
                results.push(format!("i32::from({})", self.load(*offset, "i8", operands)));
            }
            Instruction::I32Load16U { offset } => {
                results.push(format!(
                    "i32::from({})",
                    self.load(*offset, "u16", operands)
                ));
            }
            Instruction::I32Load16S { offset } => {
                results.push(format!(
                    "i32::from({})",
                    self.load(*offset, "i16", operands)
                ));
            }
            Instruction::I64Load { offset } => results.push(self.load(*offset, "i64", operands)),
            Instruction::F32Load { offset } => results.push(self.load(*offset, "f32", operands)),
            Instruction::F64Load { offset } => results.push(self.load(*offset, "f64", operands)),

            Instruction::I32Store { offset } => self.store(*offset, "as_i32", "", operands),
            Instruction::I64Store { offset } => self.store(*offset, "as_i64", "", operands),
            Instruction::F32Store { offset } => self.store(*offset, "as_f32", "", operands),
            Instruction::F64Store { offset } => self.store(*offset, "as_f64", "", operands),
            Instruction::I32Store8 { offset } => self.store(*offset, "as_i32", " as u8", operands),
            Instruction::I32Store16 { offset } => {
                self.store(*offset, "as_i32", " as u16", operands)
            }

            Instruction::Malloc {
                realloc,
                size,
                align,
            } => {
                self.needs_functions
                    .insert(realloc.to_string(), NeededFunction::Realloc);
                let tmp = self.tmp();
                let ptr = format!("ptr{}", tmp);
                self.push_str(&format!("let {} = ", ptr));
                self.call_intrinsic(realloc, format!("store, 0, 0, {}, {}", align, size));
                results.push(ptr);
            }

            Instruction::Free { .. } => unimplemented!(),
        }
    }
}

impl NeededFunction {
    fn cvt(&self) -> &'static str {
        match self {
            NeededFunction::Realloc => "(i32, i32, i32, i32), i32",
            NeededFunction::Free => "(i32, i32, i32), ()",
        }
    }

    fn ty(&self) -> String {
        format!("wasmer::TypedFunction<{}>", self.cvt())
    }
}

fn sorted_iter<K: Ord, V>(map: &HashMap<K, V>) -> impl Iterator<Item = (&K, &V)> {
    let mut list = map.into_iter().collect::<Vec<_>>();
    list.sort_by_key(|p| p.0);
    list.into_iter()
}
