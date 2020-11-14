use crate::{
    local_cache::{LocalAccess, LocalCache},
    lru_cache::{EntryState, LruCache},
    Compressible, Decompressible,
};

#[cfg(feature = "bincode_lz4")]
use crate::BincodeLz4;

use std::collections::{hash_map::RandomState, HashMap};
use std::hash::{BuildHasher, Hash};

/// A hash map that allows compressing the least recently used values. Useful when you need to store
/// a lot of large values in memory. You must define your own compression method for the value type
/// using the `Compressible` and `Decompressible` traits.
///
/// Call the `compress_lru` method to compress the least recently used value. The most recently used
/// values will stay uncompressed in a cache.
///
/// Any **mutable** access (`&mut self`) that misses the cache will decompress and cache the value
/// inline. You can call `get` to prefetch into the cache and avoid extra latency on further
/// accesses.
///
/// Any **immutable** access (`&self`, e.g. from multiple threads), like `get_const`, cannot update
/// the cache. Instead, it will record accesses and store decompressed values in a `LocalCache` that
/// can be used later to update the cache with `flush_local_cache`.
pub struct CompressibleMap<K, V, A, H = RandomState>
where
    V: Compressible<A>,
{
    cache: LruCache<K, V, H>,
    compressed: HashMap<K, V::Compressed, H>,
    compression_params: A,
}

#[cfg(feature = "bincode_lz4")]
impl<K, V, H> CompressibleMap<K, V, BincodeLz4, H>
where
    K: Clone + Eq + Hash,
    V: Compressible<BincodeLz4>,
    H: BuildHasher + Default,
{
    pub fn new_bincode_lz4(level: u32) -> Self {
        Self {
            cache: LruCache::default(),
            compressed: HashMap::default(),
            compression_params: BincodeLz4 { level },
        }
    }
}

impl<K, V, Vc, H, A> CompressibleMap<K, V, A, H>
where
    K: Clone + Eq + Hash,
    V: Compressible<A, Compressed = Vc>,
    Vc: Decompressible<A, Decompressed = V>,
    H: BuildHasher + Default,
    A: Clone,
{
    pub fn new(compression_params: A) -> Self {
        Self {
            cache: LruCache::default(),
            compressed: HashMap::default(),
            compression_params,
        }
    }

    pub fn compression_params(&self) -> &A {
        &self.compression_params
    }

    pub fn from_all_compressed(
        compression_params: A,
        compressed: HashMap<K, V::Compressed, H>,
    ) -> Self {
        let mut cache = LruCache::<K, V, H>::default();
        for key in compressed.keys() {
            cache.evict(key.clone());
        }

        Self {
            cache,
            compressed,
            compression_params,
        }
    }

    /// Insert a new value and drop the old one.
    pub fn insert(&mut self, key: K, value: V) {
        self.cache.insert(key.clone(), value);

        // PERF: this might not be necessary, but we need to confirm that the compressed value won't
        // pop up again somewhere and cause inconsistencies
        self.compressed.remove(&key);
    }

    /// Insert a compressed value, returning any pre-existing entry.
    pub fn insert_compressed(
        &mut self,
        key: K,
        value: V::Compressed,
    ) -> Option<MaybeCompressed<V, V::Compressed>> {
        let old_cached_value = self
            .cache
            .evict(key.clone())
            .map(|e| e.some_if_cached())
            .flatten();

        self.compressed
            .insert(key, value)
            .map(|v| MaybeCompressed::Compressed(v))
            .or(old_cached_value.map(|v| MaybeCompressed::Decompressed(v)))
    }

    /// Insert a new value and return the old one if it exists, which requires decompressing it.
    pub fn replace(&mut self, key: K, value: V) -> Option<V> {
        self.cache
            .insert(key.clone(), value)
            .map(|old_cache_entry| match old_cache_entry {
                EntryState::Cached(v) => v,
                EntryState::Evicted => {
                    let compressed_value = self.compressed.remove(&key).unwrap();

                    compressed_value.decompress()
                }
            })
    }

    pub fn compress_lru(&mut self) {
        if let Some((lru_key, lru_value)) = self.cache.evict_lru() {
            self.compressed
                .insert(lru_key, lru_value.compress(self.compression_params.clone()));
        }
    }

    pub fn remove_lru(&mut self) -> Option<(K, V)> {
        self.cache.remove_lru()
    }

    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        let CompressibleMap {
            cache, compressed, ..
        } = self;

        cache.get_or_repopulate_with(key.clone(), || {
            compressed.remove(&key).map(|v| v.decompress()).unwrap()
        })
    }

    pub fn get(&mut self, key: K) -> Option<&V> {
        // Hopefully downgrading the reference is a NOOP.
        self.get_mut(key).map(|v| &*v)
    }

    pub fn get_or_insert_with(&mut self, key: K, on_missing: impl FnOnce() -> V) -> &mut V {
        let CompressibleMap {
            cache, compressed, ..
        } = self;

        let on_evicted = || compressed.remove(&key).unwrap().decompress();

        cache.get_or_insert_with(key.clone(), on_evicted, on_missing)
    }

    /// Used for thread-safe access or to borrow multiple values at once. The cache will not be
    /// updated, but accesses will be recorded in the provided `LocalCache`. The interior
    /// mutability of the local cache has a cost (more heap indirection), but it allows us to borrow
    /// multiple values at once. Call `flush_local_cache` to update the "global" cache with
    /// the local cache.
    pub fn get_const<'a>(&'a self, key: K, local_cache: &'a LocalCache<K, V, H>) -> Option<&'a V> {
        self.cache.get_const(&key).map(|entry| {
            match entry {
                EntryState::Cached(v) => {
                    // For the sake of updating LRU order when we flush this local cache.
                    local_cache.remember_cached_access(key.clone());

                    v
                }
                EntryState::Evicted => {
                    // Check the local cache before trying to decompress.
                    local_cache.get_or_insert_with(key.clone(), || {
                        self.compressed.get(&key).unwrap().decompress()
                    })
                }
            }
        })
    }

    /// Returns a copy of the value at `key`, which may involve decompression.
    /// WARNING: the cache will not be updated. This is useful for read-modify-write scenarios where
    /// you would just insert the modified value back into the map, which defeats the purpose of
    /// caching it on read.
    pub fn get_copy_without_caching(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        self.cache.get_const(key).map(|entry| match entry {
            EntryState::Cached(v) => v.clone(),
            EntryState::Evicted => self.compressed.get(key).unwrap().decompress(),
        })
    }

    /// Updates the cache and it's approximate LRU order after calling `get_const` some number of
    /// times. WARNING/TODO: There is currently no mechanism to prevent overwriting newer compressed
    /// data with old data from a local cache.
    pub fn flush_local_cache(&mut self, local_cache: LocalCache<K, V, H>) {
        let CompressibleMap {
            cache, compressed, ..
        } = self;
        for (key, access) in local_cache.into_iter() {
            match access {
                LocalAccess::Cached => {
                    // We accessed this key and it was cached, so let's reflect that in the cache's
                    // LRU order.
                    cache.get(&key);
                }
                LocalAccess::Missed(value) => {
                    // We accessed this key and it was missed, so let's repopulate the cache. Don't
                    // replace a value that's already in the cache, since it might be newer than
                    // what we're trying to flush (which must have come from a read).
                    cache.get_or_repopulate_with(key.clone(), || {
                        compressed.remove(&key);

                        value
                    });
                }
            }
        }
    }

    pub fn drop(&mut self, key: &K) {
        self.cache.remove(key);
        self.compressed.remove(key);
    }

    /// Removes the value and returns it if it exists, decompressing it first.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.cache.remove(key).map(|entry| match entry {
            EntryState::Cached(v) => v,
            EntryState::Evicted => self.compressed.remove(key).unwrap().decompress(),
        })
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.compressed.clear();
    }

    pub fn len(&self) -> usize {
        self.len_cached() + self.len_compressed()
    }

    pub fn len_cached(&self) -> usize {
        self.cache.len_cached()
    }

    pub fn len_compressed(&self) -> usize {
        self.compressed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn keys<'a>(&'a self) -> impl Iterator<Item = &K>
    where
        Vc: 'a,
    {
        self.cache.keys()
    }

    /// Iterate over all (key, value) pairs, but compressed values will not be decompressed inline.
    /// Does not affect the cache.
    pub fn iter_maybe_compressed<'a>(
        &'a self,
    ) -> impl Iterator<Item = (&K, MaybeCompressed<&V, &V::Compressed>)>
    where
        Vc: 'a,
    {
        self.cache
            .iter()
            .map(|(k, v)| (k, MaybeCompressed::Decompressed(v)))
            .chain(
                self.compressed
                    .iter()
                    .map(|(k, v)| (k, MaybeCompressed::Compressed(v))),
            )
    }

    pub fn into_iter(self) -> impl Iterator<Item = (K, MaybeCompressed<V, V::Compressed>)> {
        self.cache
            .into_iter()
            .map(|(k, v)| (k, MaybeCompressed::Decompressed(v)))
            .chain(
                self.compressed
                    .into_iter()
                    .map(|(k, v)| (k, MaybeCompressed::Compressed(v))),
            )
    }
}

pub enum MaybeCompressed<D, C> {
    Decompressed(D),
    Compressed(C),
}

// ████████╗███████╗███████╗████████╗███████╗
// ╚══██╔══╝██╔════╝██╔════╝╚══██╔══╝██╔════╝
//    ██║   █████╗  ███████╗   ██║   ███████╗
//    ██║   ██╔══╝  ╚════██║   ██║   ╚════██║
//    ██║   ███████╗███████║   ██║   ███████║
//    ╚═╝   ╚══════╝╚══════╝   ╚═╝   ╚══════╝

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, Eq, PartialEq)]
    struct Foo(u32);

    struct FooCompressed(u32);

    impl Compressible<()> for Foo {
        type Compressed = FooCompressed;

        fn compress(&self, _: ()) -> Self::Compressed {
            FooCompressed(self.0 + 1)
        }
    }

    impl Decompressible<()> for FooCompressed {
        type Decompressed = Foo;

        fn decompress(&self) -> Self::Decompressed {
            Foo(self.0 + 1)
        }
    }

    #[test]
    fn get_after_compress() {
        let mut map = CompressibleMap::<_, _, _>::new(());

        map.insert(1, Foo(0));

        map.compress_lru();

        assert_eq!(map.len_cached(), 0);
        assert_eq!(map.len_compressed(), 1);

        assert_eq!(Some(&Foo(2)), map.get(1));

        assert_eq!(map.len_cached(), 1);
        assert_eq!(map.len_compressed(), 0);
    }

    #[test]
    fn keys_iterator_has_both_cached_and_compressed() {
        let mut map = CompressibleMap::<_, _, _>::new(());

        map.insert(1, Foo(0));
        map.insert(2, Foo(0));

        map.compress_lru();

        let mut keys: Vec<i32> = map.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec![1, 2]);
    }

    #[test]
    fn flush_after_get_const_populates_cache() {
        // Use a function just to mimic the "global" lifetime of the map.
        fn do_test_with_global_cache(map: &mut CompressibleMap<i32, Foo, ()>) {
            map.insert(1, Foo(0));
            map.insert(2, Foo(1));

            // Compress everything, forcing cache misses to populate the local cache.
            map.compress_lru();
            map.compress_lru();

            let local_cache = LocalCache::default();
            let mut values = Vec::new();
            values.push(map.get_const(1, &local_cache));
            values.push(map.get_const(2, &local_cache));

            // This would fail to compile, because we have living borrows!
            // map.flush_local_cache(local_cache);

            // The values were decompressed into the local cache.
            assert_eq!(Some(&Foo(2)), values[0]);
            assert_eq!(Some(&Foo(3)), values[1]);

            // The "global" cache couldn't be modified.
            assert_eq!(map.len_cached(), 0);
            assert_eq!(map.len_compressed(), 2);

            map.flush_local_cache(local_cache);

            assert_eq!(map.len_cached(), 2);
            assert_eq!(map.len_compressed(), 0);

            assert_eq!(Some(&Foo(2)), map.get(1));
            assert_eq!(Some(&Foo(3)), map.get(2));
        }

        let mut map = CompressibleMap::new(());
        do_test_with_global_cache(&mut map);
    }

    #[test]
    fn multithreaded_borrows() {
        use crossbeam::thread;

        // Populate the map.
        let mut map = CompressibleMap::<_, _, _>::new(());
        for i in 0..100 {
            map.insert(i, Foo(i));
        }

        // Compress half of the values.
        for _ in 0..50 {
            map.compress_lru();
        }

        // Gathering a batch of references.
        let local_cache = LocalCache::new();
        let mut batch = Vec::new();
        for i in 0..100 {
            batch.push(map.get_const(i, &local_cache));
        }

        thread::scope(|s| {
            for (i, value) in batch.into_iter().enumerate() {
                s.spawn(move |_| {
                    if i < 50 {
                        // These got compressed and decompressed.
                        assert_eq!(value, Some(&Foo((i + 2) as u32)))
                    } else {
                        // These stayed cached.
                        assert_eq!(value, Some(&Foo(i as u32)))
                    }
                });
            }
        })
        .unwrap();

        map.flush_local_cache(local_cache);

        assert_eq!(map.len_cached(), 100);
    }

    #[test]
    fn multithreaded_decompression() {
        use crossbeam::{channel, thread};

        // Populate the map.
        let mut map = CompressibleMap::<_, _, _>::new(());
        for i in 0..100 {
            map.insert(i, Foo(i));
        }

        // Compress half of the values.
        for _ in 0..50 {
            map.compress_lru();
        }

        // Note that we can't share a local cache among threads, but we **can** share the map!
        let map_ref = &map;
        let (tx, rx) = channel::unbounded();
        {
            let mut txs = Vec::new();
            for _ in 0..99 {
                txs.push(tx.clone());
            }
            txs.push(tx);
            let txs_ref = &txs;

            thread::scope(|s| {
                for i in 0..100 {
                    s.spawn(move |_| {
                        let local_cache = LocalCache::new();
                        if i < 50 {
                            // These got compressed and decompressed.
                            assert_eq!(
                                map_ref.get_const(i, &local_cache),
                                Some(&Foo((i + 2) as u32))
                            )
                        } else {
                            // These stayed cached.
                            assert_eq!(map_ref.get_const(i, &local_cache), Some(&Foo(i as u32)))
                        }

                        txs_ref[i as usize].send(local_cache).unwrap();
                    });
                }
            })
            .unwrap();
        }

        loop {
            match rx.recv() {
                Ok(cache) => map.flush_local_cache(cache),
                Err(_) => {
                    break;
                }
            }
        }

        assert_eq!(map.len_cached(), 100);
    }
}
