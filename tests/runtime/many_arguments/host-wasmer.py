from exports.bindings import Exports
from imports.bindings import add_imports_to_imports, Imports
from typing import Any
import math;
import sys
import wasmer # type: ignore

class MyImports:
    def many_arguments(self,
            a1: int,
            a2: int,
            a3: int,
            a4: int,
            a5: int,
            a6: int,
            a7: int,
            a8: int,
            a9: int,
            a10: int,
            a11: int,
            a12: int,
            a13: int,
            a14: int,
            a15: int,
            a16: int,
            a17: int,
            a18: int,
            a19: int,
            a20: int) -> None:
        assert(a1 == 1)
        assert(a2 == 2)
        assert(a3 == 3)
        assert(a4 == 4)
        assert(a5 == 5)
        assert(a6 == 6)
        assert(a7 == 7)
        assert(a8 == 8)
        assert(a9 == 9)
        assert(a10 == 10)
        assert(a11 == 11)
        assert(a12 == 12)
        assert(a13 == 13)
        assert(a14 == 14)
        assert(a15 == 15)
        assert(a16 == 16)
        assert(a17 == 17)
        assert(a18 == 18)
        assert(a19 == 19)
        assert(a20 == 20)


def run(wasm_file: str) -> None:
    store = wasmer.Store()
    module = wasmer.Module(store, open(wasm_file, 'rb').read())
    wasi_version = wasmer.wasi.get_version(module, strict=False)
    if wasi_version is None:
        import_object = {}
    else:
        wasi_env = wasmer.wasi.StateBuilder('test').finalize()
        import_object = wasi_env.generate_imports(store, wasi_version)

    wasm: Exports
    def get_export(name: str) -> Any:
        return wasm.instance.exports.__getattribute__(name)

    imports = MyImports()
    add_imports_to_imports(store, import_object, imports, get_export)
    wasm = Exports(store, import_object, module)

    wasm.many_arguments(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,14, 15, 16, 17, 18, 19, 20)

if __name__ == '__main__':
    run(sys.argv[1])
