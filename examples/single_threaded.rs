use compressible_map::{BincodeCompression, CompressibleMap, Lz4};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct BigValue(Vec<u8>);

fn main() {
    // Using the "bincode_lz4" feature to compress any serializable types.
    let compression = BincodeCompression::new(Lz4 { level: 10 });
    let mut map = CompressibleMap::<_, _, _>::new(compression);

    for i in 0..100 {
        map.insert(i, BigValue(vec![0; 1024]));
    }

    // Save some memory by compressing half of the values.
    for _ in 0..50 {
        map.compress_lru();
    }

    // Read some values, some are already cached and some will be decompressed
    // into the cache.
    for i in 25..75 {
        map.get(i);
    }
}
