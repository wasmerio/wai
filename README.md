<div align="center">
  <h1><code>WebAssembly Interfaces</code></h1>

  <p>
    <strong>A language bindings generator for <code>wai</code></strong>
  </p>

  <strong>
    A <a href="https://wasmer.io/">Wasmer</a> project building on
    <a href="https://github.com/wasmerio/wai">wai</a>
  </strong>

  <p>
    <a href="https://github.com/wasmerio/wai/actions?query=workflow%3ACI"><img src="https://github.com/wasmerio/wai/workflows/CI/badge.svg" alt="build status" /></a>
    <img src="https://img.shields.io/badge/rustc-stable+-green.svg" alt="supported rustc stable" />
  </p>
</div>

## About

> **Note**: Unfortunately, the maintainers behind [`wit-bindgen`] didn’t want to
> [add support for Wasmer upstream], so we had to do a hard-fork in order to
> make things work with [Wasmer].

[`wit-bindgen`]: https://github.com/bytecodealliance/wit-bindgen
[add support for Wasmer upstream]: https://github.com/bytecodealliance/wit-bindgen/issues/306
[Wasmer]: https://wasmer.io

This project is a bindings generator framework for WebAssembly programs and
embeddings of WebAssembly. This works with `*.wai` files which describe the
interface of a module, either imported or exported. For example this project can
be used in cases such as:

* Your language (say, Rust) is compiled to WebAssembly and you'd like to import
  WASI. This project will generate Rust bindings to import WASI APIs that are
  described with `*.wai`.

* Your runtime (say, Wasmer) wants to then provide WASI functionality to guest
  programs. This project will generate a Rust `trait` for you to implement for
  the WASI interface.

* You're consuming a WebAssembly module (say, in a browser) and you don't want
  to deal with funky ABI details. You'd use this project to generate JS bindings
  which give you a TypeScript interface dealing with native JS types for the
  WebAssembly module described by `*.wai`.

This project is based on the [interface types
proposal](https://github.com/webassembly/interface-types). This repository will be
following upstream changes. The purpose of `wai` is to provide a
forwards-compatible toolchain and story for interface types and a canonical ABI.
Generated language bindings all use the canonical ABI for communication,
enabling WebAssembly modules to be written in any language with support and for
WebAssembly modules to be consumed in any environment with language support.

## Demo

[View generated bindings
online!](https://wasmerio.github.io/wai/)

If you're curious to poke around and see what generated bindings look like for a
given input `*.wai`, you can explore the generated code online to get an idea
of what's being generated and what the glue code looks like.

## Usage

At this time a CLI tool is provided mostly for debugging and exploratory
purposes. It can be used easily with the `wasmer` CLI.

```wai
// browser.wai

record person {
  name: string,
  age: u32,
}

// Say hello to either the specified person or the current user
hello: func(who: option<person>) -> string
```

```console
$ wasmer run wasmer/wai-bindgen-cli --dir=. -- js --import browser.wai
Generating "browser.d.ts"
Generating "browser.js"
Generating "intrinsics.js"
```

This tool is not necessarily intended to be integrated into toolchains. For
example usage in Rust would more likely be done through procedural macros and
Cargo dependencies. Usage in a Web application would probably use a version of
`wai-bindgen` compiled to WebAssembly and published to NPM.

For now, though, you can explore what bindings look like in each language
through the CLI. Again if you'd like to depend on this if you wouldn't mind
please reach out on [Slack] so we can figure out a better story than relying on
the CLI tool for your use case.

## Supported Languages

First here's a list of supported languages for generating a WebAssembly binary
which uses interface types. This means that these languages support
`*.wai`-defined imports and exports.

* `rust-wasm` - this is for Rust compiled to WebAssembly, typically using either
  the `wasm32-wasi` or `wasm32-unknown-unknown` targets depending on your use
  case. In this mode you'd probably depend on the `wai-bindgen-rust` crate
  (located at `crates/rust-wasm`) and use the `import!` and `export!` macros to
  generate code.

* `c` - this is for C compiled to WebAssembly, using either of the targets above
  for Rust as well. With C the `wai-bindgen` CLI tool will emit a `*.h` and a
  `*.c` file to be compiled into the wasm module.

This repository also supports a number of host languages/runtimes which can be
used to consume WebAssembly modules that use interface types. These modules need
to follow the canonical ABI for their exports/imports:

* `wasmer` - this is for Rust users using the `wasmer` crate. This generator
  is used through the `wai-bindgen-wasmer` crate (located at
  `crates/wasmer`) and, like the compiled-to-wasm Rust support, has an
  `import!` and an `export!` macro for generating code.

* `js` - this is for JavaScript users executing WebAssembly modules. This could
  be in a browser, Node.js, or Deno. In theory this covers browser use cases
  like web workers and such as well. In this mode the `wai-bindgen` CLI tool
  will emit a `*.js` and a `*.d.ts` file describing the interface and providing
  necessary runtime support in JS to implement the canonical ABI. Note that the
  intended long-term integration of this language is to compile `wai-bindgen`
  itself to WebAssembly and publish NPM packages for popular JS build systems to
  integrate `wai-bindgen` into JS build processes.

* `wasmer-py` - this is for Python users using the `wasmer` PyPI package.
  This uses Wasmer under the hood but you get to write Python in providing
  imports to WebAssembly modules or consume modules using interface types. This
  generates a `*.py` file which is annotated with types for usage in `mypy` or
  other type-checkers.

All generators support the `--import` and `--export` flags in the `wai-bindgen`
CLI tool:

```console
$ wasmer run wasmer/wai-bindgen-cli --dir=. -- js --import browser.wai
$ wasmer run wasmer/wai-bindgen-cli --dir=. -- rust-wasm --export my-interface.wai
$ wasmer run wasmer/wai-bindgen-cli --dir=. -- wasmer --import host-functions.wai
```

Here "import" means "I want to import and call the functions in this interface"
and "export" means "I want to define the functions in this interface for others
to call".

Finally in a sort of "miscellaneous" category the `wai-bindgen` CLI also
supports:

* `markdown` - generates a `*.md` and a `*.html` file with readable
  documentation rendered from the comments in the source `*.wai` file.

Note that the list of supported languages here is a snapshot in time and is not
final. The purpose of the interface-types proposal is to be language agnostic
both in how WebAssembly modules are written as well as how they are consumed. If
you have a runtime that isn't listed here or you're compiling to WebAssembly and
your language isn't listed here, it doesn't mean that it will never be
supported! A language binding generator is intended to be not the hardest thing
in the world (but unfortunately also not the easiest) to write, and the crates
and support in this repository mostly exist to make writing generators as easy
as possible.

Some other languages and runtimes, for example, that don't have support in
`wai-bindgen` today but are possible in the future (and may get written here
too) are:

* `wasmer-go` - same as for `wasmer-py` but for Go. Basically for Go users
  using the [`wasmer-go`
  package](https://github.com/wasmerio/wasmer-go) who want to work
  with interface types rather than raw pointers/memories/etc.

* `wasmer-ruby` - same as for `wasmer-py` but for Ruby. Basically for Go users
  using the [`wasmer-ruby`
  package](https://github.com/wasmerio/wasmer-ruby) who want to work
  with interface types rather than raw pointers/memories/etc.

Note that this is not an exclusive list, only intended to give you an idea of
what other bindings could look like. There's a plethora of runtimes and
languages that compile to WebAssembly, and interface types should be able to
work with all of them and it's theoretically just some work-hours away from
having support in `wai-bindgen`.

[Slack]: https://slack.wasmer.io/
