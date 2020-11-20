mod compressible_map;
mod local_cache;
mod lru_cache;

#[cfg(feature = "bincode_lz4")]
mod bincode_lz4;
#[cfg(feature = "bincode_snappy")]
mod bincode_snappy;

#[cfg(feature = "bincode_lz4")]
pub use bincode_lz4::{BincodeLz4, BincodeLz4Compressed};
#[cfg(feature = "bincode_snappy")]
pub use bincode_snappy::{BincodeSnappy, BincodeSnappyCompressed};

pub use crate::compressible_map::{CompressibleMap, MaybeCompressed};
pub use local_cache::LocalCache;

/// A type that's compressible using algorithm `A`.
pub trait Compressible<A> {
    type Compressed: Decompressible<A>;

    fn compress(&self, params: A) -> Self::Compressed;
}

/// A type that's decompressible using the inverse of algorithm `A`.
pub trait Decompressible<A> {
    type Decompressed: Compressible<A>;

    fn decompress(&self) -> Self::Decompressed;
}
