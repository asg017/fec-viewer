//! A filing's bytes, all addressable at once: memory-mapped from disk
//! natively (so a multi-gigabyte filing costs address space, not memory), or
//! the buffer the browser handed us.

use std::sync::Arc;

use crate::filing::Source;

#[derive(Clone)]
pub enum Data {
    Owned(Arc<[u8]>),
    #[cfg(not(target_family = "wasm"))]
    Mapped(Arc<memmap2::Mmap>),
}

impl Data {
    pub fn open(source: &Source) -> std::io::Result<Self> {
        match source {
            #[cfg(not(target_family = "wasm"))]
            Source::Path(path) => {
                let file = std::fs::File::open(path)?;
                if file.metadata()?.len() == 0 {
                    // Mapping an empty file fails on some platforms.
                    return Ok(Self::Owned(Arc::from(Vec::new())));
                }
                // SAFETY: the map is read-only. If another process truncates
                // the file while it's open, reads past the new end fault;
                // that's the usual trade for not reading gigabytes up front.
                let map = unsafe { memmap2::Mmap::map(&file)? };
                Ok(Self::Mapped(Arc::new(map)))
            }
            #[cfg(target_family = "wasm")]
            Source::Path(_) => Err(std::io::Error::other("no filesystem in the browser")),
            Source::Bytes { bytes, .. } => Ok(Self::Owned(bytes.clone())),
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            Data::Owned(b) => b,
            #[cfg(not(target_family = "wasm"))]
            Data::Mapped(m) => m,
        }
    }

    pub fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }
}

impl From<Vec<u8>> for Data {
    fn from(v: Vec<u8>) -> Self {
        Self::Owned(v.into())
    }
}
