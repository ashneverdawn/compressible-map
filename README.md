# Compressible Map

A hash map that allows compressing the least recently used values. Useful when you need to store a
lot of large values in memory.

Two compression backends are provided:

- Lz4
- Snappy

These can be used on any serializable values by setting:

```toml
features = ["compressed-bincode", "lz4"]
```

or

```toml
features = ["compressed-bincode", "snap"]
```

Or you can implement the `Compression` trait in your own way.
