use compressible_map::{BincodeCompression, CompressibleMap, LocalCache, Lz4};

use crossbeam::{channel, thread};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct BigValue(Vec<u8>);

fn main() {
    // Populate the map.
    let compression = BincodeCompression::new(Lz4 { level: 10 });
    let mut map = CompressibleMap::<_, _, _>::new(compression);
    for i in 0..100 {
        map.insert(i, BigValue(vec![0; 1024]));
    }

    // Compress half of the values.
    for _ in 0..50 {
        map.compress_lru();
    }

    // Note that we can't share a local cache among threads, but we **can** share the map!
    let map_ref = &map;
    let (tx, rx) = channel::unbounded();
    {
        // Set up channels to send the thread-local caches back to main thread so we can update the
        // global cache.
        let mut txs = Vec::new();
        for _ in 0..99 {
            txs.push(tx.clone());
        }
        txs.push(tx);
        let txs_ref = &txs;

        thread::scope(|s| {
            for i in 0..100 {
                s.spawn(move |_| {
                    // Borrow a big value. It can either live in the global cache, the local cache,
                    // or neither, requiring decompression inline. After decompression, we can't
                    // modify the global cache, so we modify the local one.
                    let local_cache = LocalCache::new();
                    let _big_value = map_ref.get_const(i, &local_cache);

                    // Do something with big value...

                    // Send the local cache back to the main thread.
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
