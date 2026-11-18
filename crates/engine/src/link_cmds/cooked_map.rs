use std::sync::Arc;

use texturecraft_compose::ImageRes;
use texturecraft_core::Channel;

use crate::{EngineError, Result};

/// A validated bake ready to install in the document and decoded-image cache.
#[derive(Clone, Debug)]
pub struct CookedMap {
    pub channel: Channel,
    pub bytes: Arc<Vec<u8>>,
    pub image: Arc<ImageRes>,
}

impl CookedMap {
    /// Allow nine 4K RGBA maps across a single material import.
    pub const MAX_TOTAL_TEXELS: u64 = 9 * 4096 * 4096;

    pub fn decode(channel: Channel, bytes: Vec<u8>, remaining_texels: &mut u64) -> Result<Self> {
        let limit = (*remaining_texels).min(texturecraft_formats::MAX_RESOURCE_TEXELS);
        let image = texturecraft_formats::decode_image_within(&bytes, limit).map_err(|error| EngineError::Other(format!("{} map: {error}", channel.id())))?;
        *remaining_texels -= u64::from(image.width) * u64::from(image.height);

        Ok(Self { channel, bytes: Arc::new(bytes), image: crate::resource_cmds::decoded(image) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_retained_map_reduces_the_next_decoders_budget() {
        let bytes = texturecraft_formats::gray16_png(4, 4, [0.5; 16].into_iter()).unwrap();
        let mut remaining = 31;

        let first = CookedMap::decode(Channel::Roughness, bytes.clone(), &mut remaining).unwrap();
        let second = CookedMap::decode(Channel::Metallic, bytes, &mut remaining);

        assert_eq!(first.image.rgba.len(), 16);
        assert_eq!(remaining, 15);
        assert!(second.unwrap_err().to_string().contains("15 texels"));
    }
}
