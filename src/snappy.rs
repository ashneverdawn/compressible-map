use crate::{Compressible, Decompressible};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// A fast and portable compression scheme for use with the `CompressibleMap`.
#[derive(Clone, Copy, Debug)]
pub struct BincodeSnappy;

#[derive(Deserialize, Serialize)]
pub struct BincodeSnappyCompressed<T> {
    pub compressed_bytes: Vec<u8>,
    marker: std::marker::PhantomData<T>,
}

impl<T> Compressible<BincodeSnappy> for T
where
    T: DeserializeOwned + Serialize,
{
    type Compressed = BincodeSnappyCompressed<T>;

    fn compress(&self, _params: BincodeSnappy) -> Self::Compressed {
        let serialized_bytes = bincode::serialize(&self).unwrap();

        let mut encoder = snap::write::FrameEncoder::new(Vec::new());

        std::io::copy(&mut std::io::Cursor::new(serialized_bytes), &mut encoder).unwrap();
        let compressed_bytes = encoder.into_inner().expect("failed to flush the writer");

        BincodeSnappyCompressed {
            compressed_bytes,
            marker: Default::default(),
        }
    }
}

impl<T> Decompressible<BincodeSnappy> for BincodeSnappyCompressed<T>
where
    T: DeserializeOwned + Serialize,
{
    type Decompressed = T;

    fn decompress(&self) -> Self::Decompressed {
        let mut decoder = snap::read::FrameDecoder::new(self.compressed_bytes.as_slice());
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

        let compressed_foo = foo.compress(BincodeSnappy);

        let decompressed_foo = compressed_foo.decompress();

        assert_eq!(foo, decompressed_foo);
    }
}
