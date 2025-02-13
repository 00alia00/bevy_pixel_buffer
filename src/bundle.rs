//! [Bundle]s that can be used to manually create a pixel buffer.
//!
//! As the image of a pixel buffer has to be created with specific parameters,
//! [create_image](crate::pixel_buffer::create_image) should be used to obtain it. Because of
//! this, the usage of [PixelBufferBuilder](crate::builder::PixelBufferBuilder) is recommended but
//! not required.

use crate::pixel_buffer::PixelBuffer;
use bevy::prelude::*;

/// [Bundle] to create a pixel buffer manually.
///
/// Gives the possibility to insert the [Bundle] instead of
/// abstracting it with [PixelBufferBuilder](crate::builder::PixelBufferBuilder).
///
/// The [PixelBufferBundle::image] underlying image handle should be obtained from
/// [create_image](crate::pixel_buffer::create_image) and added to the [Assets] to get a handle.
#[derive(Bundle)]
pub struct PixelBufferBundle {
    /// Pixel buffer component.
    pub pixel_buffer: PixelBuffer,
    /// Image handle obtained with [create_image](crate::pixel_buffer::create_image).
    pub sprite: Sprite,
}

