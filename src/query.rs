//! Adds utility queries
//!
//! These are common queries and resources grouped up, so everything
//! done here can be replicated with a normal query. You may not
//! need or want to use them but for quick prototyping they are useful to
//! have and not pollute your systems with many and/or complex types.
//!
//! [PixelBuffers] is a [WorldQuery] intented for more than one pixel buffer.
//!
//! [QueryPixelBuffer] is a [SystemParam] that groups the [PixelBuffers] query and
//! the [image](Image) [assets](Assets) resource. It has some convenience methods
//! when working with a single pixel buffer.
//!
//! # Examples
//!
//! For many pixel buffers
//! ```
//! # use bevy::prelude::*;
//! # use bevy_pixel_buffer::prelude::*;
//! fn example_system(mut images: ResMut<Assets<Image>>, pixel_buffers: Query<PixelBuffers>) {
//!     for item in pixel_buffers.iter() {
//!         images.frame(item).per_pixel(|_, _| Pixel::random())
//!     }
//! }
//! # bevy::ecs::system::assert_is_system(example_system);
//! ```
//! Is equivalent to
//! ```
//! # use bevy::prelude::*;
//! # use bevy_pixel_buffer::prelude::*;
//! fn example_system(pixel_buffers: QueryPixelBuffer) {
//!     let (query, mut images) = pixel_buffers.split();
//!     for item in query.iter() {
//!         images.frame(item).per_pixel(|_, _| Pixel::random())
//!     }
//! }
//! # bevy::ecs::system::assert_is_system(example_system);
//! ```
//! ---
//! For a single pixel buffer
//!
//! ```
//! # use bevy::prelude::*;
//! # use bevy_pixel_buffer::prelude::*;
//! fn example_system(mut pb: QueryPixelBuffer) {
//!     pb.frame().per_pixel(|_, _| Pixel::random());
//! }
//! # bevy::ecs::system::assert_is_system(example_system);
//! ```

use std::ops::{Deref, DerefMut};

use bevy::{ecs::system::SystemParam, prelude::*};

use crate::{
    frame::{AsImageHandle, Frame, GetFrame},
    pixel_buffer::PixelBuffer,
};

// #[derive(WorldQuery)] generates structs without documentation, put them inside
// here to allow that
mod queries {
    use bevy::ecs::query::QueryData;

    use super::*;
    // cannot use #[cfg(feature = "egui")] inside the derive

    #[cfg(not(feature = "egui"))]
    /// Query to get the pixel buffers
    ///
    /// See [module documentation](crate::query).
    #[derive(QueryData)]
    #[query_data(mutable, derive(Debug))]
    pub struct PixelBuffers {
        /// [Entity] of the pixel buffer
        pub entity: Entity,
        /// [PixelBuffer] component
        pub pixel_buffer: &'static mut PixelBuffer,
        /// Image handle
        pub sprite: &'static Sprite,
    }

    #[cfg(feature = "egui")]
    /// Query to get the pixel buffers.
    ///
    /// See [module documentation](crate::query).
    #[derive(QueryData)]
    #[query_data(mutable, derive(Debug))]
    pub struct PixelBuffers {
        /// [Entity] of the pixel buffer
        pub entity: Entity,
        /// [PixelBuffer] component
        pub pixel_buffer: &'static mut PixelBuffer,
        /// Image handle via Sprte
        pub sprite: &'static Sprite,
        /// [EguiTexture](crate::egui::EguiTexture) component.
        ///
        /// Only available with the `egui` feature.
        ///
        /// If the [PixelBufferEguiPlugin](crate::egui::PixelBufferEguiPlugin) is added
        /// it will always be [Some].
        pub egui_texture: Option<&'static crate::egui::EguiTexture>,
    }
}

pub use queries::*;

impl AsImageHandle for crate::query::PixelBuffersReadOnlyItem<'_> {
    fn as_image_handle(&self) -> &Handle<Image> {
        &self.sprite.image
    }
}

impl AsImageHandle for crate::query::PixelBuffersItem<'_> {
    fn as_image_handle(&self) -> &Handle<Image> {
        &self.sprite.image
    }
}

/// System parameter to use in systems
#[derive(SystemParam)]
pub struct QueryPixelBuffer<'w, 's> {
    pub(crate) query: Query<'w, 's, PixelBuffers>,
    pub(crate) images: ResMut<'w, Assets<Image>>,
}

impl<'w, 's> Deref for QueryPixelBuffer<'w, 's> {
    type Target = Query<'w, 's, PixelBuffers>;

    fn deref(&self) -> &Self::Target {
        &self.query
    }
}

impl<'w, 's> DerefMut for QueryPixelBuffer<'w, 's> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.query
    }
}

// Zheoni: Help, I can't make a way to iterate over Frame s... lifetimes
//   and so many other problems :(

impl<'w, 's> QueryPixelBuffer<'w, 's> {
    /// Get the image assets resource.
    pub fn images(&mut self) -> &mut Assets<Image> {
        &mut self.images
    }

    /// Gets the query and images resource
    pub fn split(self) -> (Query<'w, 's, PixelBuffers>, ResMut<'w, Assets<Image>>) {
        (self.query, self.images)
    }
}

impl<'w, 's> GetFrame for QueryPixelBuffer<'w, 's> {
    fn frame(&mut self) -> Frame<'_> {
        let image_handle = &self.query.single().sprite.image;
        Frame::extract(&mut self.images, &image_handle)
    }
}
