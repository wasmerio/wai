use crate::Source;
use std::collections::{BTreeMap, BTreeSet};

/// Tracks all of the import and intrinsics that a given codegen
/// requires and how to generate them when needed.
#[derive(Default)]
pub struct Dependencies {
    pub needs_clamp: bool,
    pub needs_store: bool,
    pub needs_load: bool,
    pub needs_validate_guest_char: bool,
    pub needs_expected: bool,
    pub needs_i32_to_f32: bool,
    pub needs_f32_to_i32: bool,
    pub needs_i64_to_f64: bool,
    pub needs_f64_to_i64: bool,
    pub needs_decode_utf8: bool,
    pub needs_encode_utf8: bool,
    pub needs_list_canon_lift: bool,
    pub needs_list_canon_lower: bool,
    pub needs_t_typevar: bool,
    pub needs_resources: bool,
    pub pyimports: BTreeMap<String, Option<BTreeSet<String>>>,
}

impl Dependencies {
    /// Record that a Python import is required
    ///
    /// Examples
    /// ```
    /// # use wasmer_wit_bindgen_gen_wasmer_py::dependencies::Dependencies;
    /// # let mut deps = Dependencies::default();
    /// // Import a specific item from a module
    /// deps.pyimport("typing", "NamedTuple");
    /// // Import an entire module
    /// deps.pyimport("collections", None);
    /// ```
    pub fn pyimport<'a>(&mut self, module: &str, name: impl Into<Option<&'a str>>) {
        let name = name.into();
        let list = self
            .pyimports
            .entry(module.to_string())
            .or_insert(match name {
                Some(_) => Some(BTreeSet::new()),
                None => None,
            });
        match name {
            Some(name) => {
                assert!(list.is_some());
                list.as_mut().unwrap().insert(name.to_string());
            }
            None => assert!(list.is_none()),
        }
    }

    /// Create a `Source` containing all of the intrinsics
    /// required according to this `Dependencies` struct.
    pub fn intrinsics(&mut self) -> Source {
        let mut src = Source::default();

        if self.needs_clamp {
            src.push_str(
                "
                    def _clamp(i: int, min: int, max: int) -> int:
                        if i < min or i > max:
                            raise OverflowError(f'must be between {min} and {max}')
                        return i
                ",
            );
        }
        if self.needs_store {
            self.pyimport("typing", "Callable");
            // TODO: this uses native endianness
            src.push_str(
                "
                    def _store(make_view: Callable[[], Any], mem: wasmer.Memory, base: int, offset: int, val: Any) -> None:
                        ptr = (base & 0xffffffff) + offset
                        view = make_view()
                        if ptr + view.bytes_per_element > mem.data_size:
                            raise IndexError('out-of-bounds store')
                        view_ptr = ptr // view.bytes_per_element
                        view[view_ptr] = val
                ",
            );
        }
        if self.needs_load {
            self.pyimport("typing", "Callable");
            // TODO: this uses native endianness
            src.push_str(
                "
                    def _load(make_view: Callable[[], Any], mem: wasmer.Memory, base: int, offset: int) -> Any:
                        ptr = (base & 0xffffffff) + offset
                        view = make_view()
                        if ptr + view.bytes_per_element > mem.data_size:
                            raise IndexError('out-of-bounds load')
                        view_ptr = ptr // view.bytes_per_element
                        return view[view_ptr]
                ",
            );
        }
        if self.needs_validate_guest_char {
            src.push_str(
                "
                    def _validate_guest_char(i: int) -> str:
                        if i > 0x10ffff or (i >= 0xd800 and i <= 0xdfff):
                            raise TypeError('not a valid char');
                        return chr(i)
                ",
            );
        }
        if self.needs_expected {
            self.pyimport("dataclasses", "dataclass");
            self.pyimport("typing", "TypeVar");
            self.pyimport("typing", "Generic");
            self.pyimport("typing", "Union");
            self.needs_t_typevar = true;
            src.push_str(
                "
                    @dataclass
                    class Ok(Generic[T]):
                        value: T
                    E = TypeVar('E')
                    @dataclass
                    class Err(Generic[E]):
                        value: E

                    Expected = Union[Ok[T], Err[E]]
                ",
            );
        }
        if self.needs_i32_to_f32 || self.needs_f32_to_i32 {
            self.pyimport("ctypes", None);
            src.push_str("_i32_to_f32_i32 = ctypes.pointer(ctypes.c_int32(0))\n");
            src.push_str(
                "_i32_to_f32_f32 = ctypes.cast(_i32_to_f32_i32, ctypes.POINTER(ctypes.c_float))\n",
            );
            if self.needs_i32_to_f32 {
                src.push_str(
                    "
                        def _i32_to_f32(i: int) -> float:
                            _i32_to_f32_i32[0] = i     # type: ignore
                            return _i32_to_f32_f32[0]  # type: ignore
                    ",
                );
            }
            if self.needs_f32_to_i32 {
                src.push_str(
                    "
                        def _f32_to_i32(i: float) -> int:
                            _i32_to_f32_f32[0] = i    # type: ignore
                            return _i32_to_f32_i32[0] # type: ignore
                    ",
                );
            }
        }
        if self.needs_i64_to_f64 || self.needs_f64_to_i64 {
            self.pyimport("ctypes", None);
            src.push_str("_i64_to_f64_i64 = ctypes.pointer(ctypes.c_int64(0))\n");
            src.push_str(
                "_i64_to_f64_f64 = ctypes.cast(_i64_to_f64_i64, ctypes.POINTER(ctypes.c_double))\n",
            );
            if self.needs_i64_to_f64 {
                src.push_str(
                    "
                        def _i64_to_f64(i: int) -> float:
                            _i64_to_f64_i64[0] = i    # type: ignore
                            return _i64_to_f64_f64[0] # type: ignore
                    ",
                );
            }
            if self.needs_f64_to_i64 {
                src.push_str(
                    "
                        def _f64_to_i64(i: float) -> int:
                            _i64_to_f64_f64[0] = i    # type: ignore
                            return _i64_to_f64_i64[0] # type: ignore
                    ",
                );
            }
        }
        if self.needs_decode_utf8 {
            src.push_str(
                "
                    def _decode_utf8(mem: wasmer.Memory, ptr: int, len: int) -> str:
                        ptr = ptr & 0xffffffff
                        len = len & 0xffffffff
                        if ptr + len > mem.data_size:
                            raise IndexError('string out of bounds')
                        view = mem.uint8_view()
                        bytes = bytearray(view[ptr:ptr+len])
                        x = bytes.decode('utf8')
                        return x
                ",
            );
        }
        if self.needs_encode_utf8 {
            self.pyimport("typing", "Tuple");
            src.push_str(
                "
                    def _encode_utf8(val: str, realloc: wasmer.Function, mem: wasmer.Memory) -> Tuple[int, int]:
                        bytes = val.encode('utf8')
                        ptr = realloc(0, 0, 1, len(bytes))
                        assert(isinstance(ptr, int))
                        ptr = ptr & 0xffffffff
                        if ptr + len(bytes) > mem.data_size:
                            raise IndexError('string out of bounds')
                        view = mem.uint8_view()
                        view[ptr:ptr+len(bytes)] = bytes
                        return (ptr, len(bytes))
                ",
            );
        }
        if self.needs_list_canon_lift {
            self.pyimport("ctypes", None);
            self.pyimport("typing", "List");
            self.pyimport("typing", "Callable");
            // TODO: this is doing a native-endian read, not a little-endian
            // read
            src.push_str(
                "
                    def _list_canon_lift(ptr: int, len: int, size: int, make_view: Callable[[], Any], mem: wasmer.Memory) -> Any:
                        ptr = ptr & 0xffffffff
                        len = len & 0xffffffff
                        if ptr + len * size > mem.data_size:
                            raise IndexError('list out of bounds')
                        view = make_view()
                        assert(size == view.bytes_per_element)
                        view_ptr = ptr // view.bytes_per_element
                        if isinstance(view, wasmer.Uint8Array):
                            return bytearray(view[view_ptr:view_ptr+len])
                        return view[view_ptr:view_ptr + len]
                ",
            );
        }
        if self.needs_list_canon_lower {
            self.pyimport("typing", "List");
            self.pyimport("typing", "Tuple");
            self.pyimport("typing", "Callable");
            // TODO: this is doing a native-endian write, not a little-endian
            // write
            src.push_str(
                "
                    def _list_canon_lower(list: Any, make_view: Callable[[], Any], size: int, align: int, realloc: wasmer.Function, mem: wasmer.Memory) -> Tuple[int, int]:
                        total_size = size * len(list)
                        ptr = realloc(0, 0, align, total_size)
                        assert(isinstance(ptr, int))
                        ptr = ptr & 0xffffffff
                        if ptr + total_size > mem.data_size:
                            raise IndexError('list realloc return of bounds')
                        view = make_view()
                        assert(size == view.bytes_per_element)
                        view_ptr = ptr // view.bytes_per_element
                        view[view_ptr:view_ptr + len(list)] = list
                        return (ptr, len(list))
                ",
            );
        }

        if self.needs_resources {
            self.pyimport("typing", "TypeVar");
            self.pyimport("typing", "Generic");
            self.pyimport("typing", "List");
            self.pyimport("typing", "Optional");
            self.pyimport("dataclasses", "dataclass");
            self.needs_t_typevar = true;
            src.push_str(
                "
                    @dataclass
                    class SlabEntry(Generic[T]):
                        next: int
                        val: Optional[T]

                    class Slab(Generic[T]):
                        head: int
                        list: List[SlabEntry[T]]

                        def __init__(self) -> None:
                            self.list = []
                            self.head = 0

                        def insert(self, val: T) -> int:
                            if self.head >= len(self.list):
                                self.list.append(SlabEntry(next = len(self.list) + 1, val = None))
                            ret = self.head
                            slot = self.list[ret]
                            self.head = slot.next
                            slot.next = -1
                            slot.val = val
                            return ret

                        def get(self, idx: int) -> T:
                            if idx >= len(self.list):
                                raise IndexError('handle index not valid')
                            slot = self.list[idx]
                            if slot.next == -1:
                                assert(slot.val is not None)
                                return slot.val
                            raise IndexError('handle index not valid')

                        def remove(self, idx: int) -> T:
                            ret = self.get(idx)
                            slot = self.list[idx]
                            slot.val = None
                            slot.next = self.head
                            self.head = idx
                            return ret
                ",
            );
        }

        src
    }
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, BTreeSet};

    use super::Dependencies;

    #[test]
    fn test_pyimport_only_contents() {
        let mut deps = Dependencies::default();
        deps.pyimport("typing", None);
        deps.pyimport("typing", None);
        assert_eq!(deps.pyimports, BTreeMap::from([("typing".into(), None)]));
    }

    #[test]
    fn test_pyimport_only_module() {
        let mut deps = Dependencies::default();
        deps.pyimport("typing", "Union");
        deps.pyimport("typing", "List");
        deps.pyimport("typing", "NamedTuple");
        assert_eq!(
            deps.pyimports,
            BTreeMap::from([(
                "typing".into(),
                Some(BTreeSet::from([
                    "Union".into(),
                    "List".into(),
                    "NamedTuple".into()
                ]))
            )])
        );
    }

    #[test]
    #[should_panic]
    fn test_pyimport_conflicting() {
        let mut deps = Dependencies::default();
        deps.pyimport("typing", "NamedTuple");
        deps.pyimport("typing", None);
    }
}
