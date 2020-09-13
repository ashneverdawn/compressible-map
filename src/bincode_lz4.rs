use crate::{Compressible, Decompressible};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// A fast and portable compression scheme for use with the `CompressibleMap`.
#[derive(Clone, Copy)]
pub struct BincodeLz4 {
    pub level: u32,
}

#[derive(Deserialize, Serialize)]
pub struct BincodeLz4Compressed<T> {
    pub compressed_bytes: Vec<u8>,
    marker: std::marker::PhantomData<T>,
}

impl<T> Compressible<BincodeLz4> for T
where
    T: DeserializeOwned + Serialize,
{
    type Compressed = BincodeLz4Compressed<T>;

    fn compress(&self, params: BincodeLz4) -> Self::Compressed {
        let serialized_bytes = bincode::serialize(&self).unwrap();

        let mut compressed_bytes = Vec::new();
        let mut encoder = lz4::EncoderBuilder::new()
            .level(params.level)
            .build(&mut compressed_bytes)
            .unwrap();

        std::io::copy(&mut std::io::Cursor::new(serialized_bytes), &mut encoder).unwrap();
        let (_output, _result) = encoder.finish();

        BincodeLz4Compressed {
            compressed_bytes,
            marker: Default::default(),
        }
    }
}

impl<T> Decompressible<BincodeLz4> for BincodeLz4Compressed<T>
where
    T: DeserializeOwned + Serialize,
{
    type Decompressed = T;

    fn decompress(&self) -> Self::Decompressed {
        let mut decoder = lz4::Decoder::new(self.compressed_bytes.as_slice()).unwrap();
        let mut decompressed_bytes = Vec::new();
        std::io::copy(&mut decoder, &mut decompressed_bytes).unwrap();

        bincode::deserialize(decompressed_bytes.as_slice()).unwrap()
    }
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

    use serde::Deserialize;

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Foo(Vec<i32>);

    #[test]
    fn compress_and_decompress_serializable_type() {
        let foo = Foo((0..100).collect());

        let compressed_foo = foo.compress(BincodeLz4 { level: 10 });

        let decompressed_foo = compressed_foo.decompress();

        assert_eq!(foo, decompressed_foo);
    }
}
