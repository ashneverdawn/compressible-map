#[cfg(feature = "compressed-bincode")]
mod compressed_bincode;
#[cfg(feature = "lz4")]
mod lz4_compression;
#[cfg(feature = "snap")]
mod snappy_compression;

#[cfg(feature = "compressed-bincode")]
pub use compressed_bincode::BincodeCompression;
#[cfg(feature = "lz4")]
pub use lz4_compression::Lz4;
#[cfg(feature = "snap")]
pub use snappy_compression::Snappy;

use serde::{Deserialize, Serialize};

/// An algorithm for compressing a specific type `Data` into type `Compressed`.
pub trait Compression: Sized {
    type Data;
    type CompressedData;

    fn compress(&self, data: &Self::Data) -> Compressed<Self>;
    fn decompress(compressed: &Self::CompressedData) -> Self::Data;
}

#[derive(Deserialize, Serialize)]
pub struct Compressed<A>
where
    A: Compression,
{
    pub compressed_data: A::CompressedData,
    marker: std::marker::PhantomData<A>,
}

impl<T, A> Compressed<A>
where
    A: Compression<CompressedData = T>,
{
    pub fn new(compressed_data: A::CompressedData) -> Self {
        Self {
            compressed_data,
            marker: Default::default(),
        }
    }

    pub fn decompress(&self) -> A::Data {
        A::decompress(&self.compressed_data)
    }

    pub fn take(self) -> A::CompressedData {
        self.compressed_data
    }
}

/// A compression algorithm that acts directly on a slice of bytes.
pub trait BytesCompression {
    fn compress_bytes(&self, bytes: &[u8], compressed_bytes: impl std::io::Write);
    fn decompress_bytes(compressed_bytes: &[u8], bytes: &mut impl std::io::Write);
}
