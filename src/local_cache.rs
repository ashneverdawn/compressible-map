use core::hash::{BuildHasher, Hash};
use std::cell::UnsafeCell;
use std::collections::{hash_map, HashMap};
use std::pin::Pin;

/// When immutable cache access is required, use this `LocalCache` to store evicted values. Then
/// when you get mutable cache access, call `into_iter` to update the cache manually.
///
/// This cache comes with a price: the values will be boxed in order to provide stable borrows.
/// Ideally, it should be cheap to box your values, i.e. most of their data should already be on the
/// heap.
#[derive(Default)]
pub struct LocalCache<K, V, H> {
    accesses: UnsafeCell<HashMap<K, LocalAccess<Pin<Box<V>>>, H>>,
}

pub enum LocalAccess<V> {
    /// Represents a global cache hit that we want to remember so we can update the LRU order later.
    Cached,
    /// Represents a miss of the global cache that required us to cache the value locally.
    /// `Pin<Box>` is used to maintain a stable address for the value, even if the the map it lives
    /// in is mutated.
    Missed(V),
}

impl<V> LocalAccess<V> {
    fn unwrap_ref(&self) -> &V {
        match self {
            LocalAccess::Cached => panic!("Tried to unwrap access without value"),
            LocalAccess::Missed(value) => &value,
        }
    }

    fn map<T>(self, f: impl FnOnce(V) -> T) -> LocalAccess<T> {
        match self {
            LocalAccess::Cached => LocalAccess::Cached,
            LocalAccess::Missed(value) => LocalAccess::Missed(f(value)),
        }
    }
}

impl<K, V, H> LocalCache<K, V, H>
where
    K: Eq + Hash,
    H: Default + BuildHasher,
{
    pub fn new() -> Self {
        LocalCache {
            accesses: UnsafeCell::new(HashMap::with_hasher(Default::default())),
        }
    }

    // SAFE: We guarantee in these APIs that all references returned are valid for the lifetime of
    // the `LocalCache`, even as new values are added to the map. The invariants are:
    //   1. Once a value is placed here, it will never get dropped or moved until calling
    //      `into_iter`.
    //   2. The values are placed into `Pin<Box<V>>` so the memory address is guaranteed stable.
    //   3. Returned references must be dropped before calling `into_iter`.

    pub fn remember_cached_access(&self, key: K) {
        let mut_accesses = unsafe { &mut *self.accesses.get() };
        mut_accesses.entry(key).or_insert(LocalAccess::Cached);
    }

    pub fn get_or_insert_with(&self, key: K, f: impl FnOnce() -> V) -> &V {
        let mut_accesses = unsafe { &mut *self.accesses.get() };
        match mut_accesses.entry(key) {
            hash_map::Entry::Occupied(occupied) => {
                let access_ref = occupied.into_mut();
                match access_ref {
                    LocalAccess::Cached => {
                        *access_ref = LocalAccess::Missed(Box::pin(f()));

                        access_ref.unwrap_ref()
                    }
                    LocalAccess::Missed(value) => value,
                }
            }
            hash_map::Entry::Vacant(vacant) => {
                let access_ref = vacant.insert(LocalAccess::Missed(Box::pin(f())));

                access_ref.unwrap_ref()
            }
        }
    }

    pub fn into_iter(self) -> impl Iterator<Item = (K, LocalAccess<V>)> {
        self.accesses.into_inner().into_iter().map(|(k, access)| {
            (
                k,
                access.map(|value| unsafe { *Pin::into_inner_unchecked(value) }),
            )
        })
    }
}
