# Title

This file is like import-me.wai, but it's a Markdown file with embedded wai
code blocks.

## `foo`
```wai
/// This is foo.
type foo = u32
```

## `x`
```wai
/// This is x.
resource x
```

## `handle`
```wai
/// This is handle.
type %handle = handle x
```

## `some-record`
```wai
/// This is some-record.
type some-record = tuple<u32, u64, float32>
```
