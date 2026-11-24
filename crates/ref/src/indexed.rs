// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent CPU reference for indexed triangle-list draws.
//!
//! A draw reads `count` indices starting at index `start` and fetches vertex
//! `index + base_vertex` for each, as `DrawIndexedInstanced` and Metal's
//! `drawIndexedPrimitives` describe. Every fetched vertex must lie inside the
//! vertex buffer, and no index may be the primitive-restart value (all bits
//! set); the reference rejects the draw instead of defining either behaviour. The triangles are then rasterised by
//! [`crate::depth::render_depth`] without a depth attachment.

use std::fmt;

use crate::depth::{CompareFunction, RefDraw, RefFill, RefTexture, render_depth};

/// An index buffer of 16- or 32-bit unsigned indices.
#[derive(Clone, Copy, Debug)]
pub enum RefIndices<'a> {
    U16(&'a [u16]),
    U32(&'a [u32]),
}

impl RefIndices<'_> {
    fn len(self) -> usize {
        match self {
            Self::U16(indices) => indices.len(),
            Self::U32(indices) => indices.len(),
        }
    }

    fn restart(self) -> u32 {
        match self {
            Self::U16(_) => u32::from(u16::MAX),
            Self::U32(_) => u32::MAX,
        }
    }

    fn get(self, position: usize) -> Option<u32> {
        match self {
            Self::U16(indices) => indices.get(position).map(|&index| u32::from(index)),
            Self::U32(indices) => indices.get(position).copied(),
        }
    }
}

/// One indexed draw: a range of the index buffer, a signed base vertex and a fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefIndexedDraw {
    pub start: usize,
    pub count: usize,
    pub base_vertex: i32,
    pub fill: RefFill,
}

#[derive(Debug, Eq, PartialEq)]
pub enum IndexedRefError {
    /// The index count of the draw is zero or not a multiple of 3: (draw).
    Count(usize),
    /// The index range of the draw ends past the index buffer: (draw).
    IndexRange(usize),
    /// `index + base_vertex` lies outside the vertex buffer: (draw, index position).
    VertexRange(usize, usize),
    /// The index is the primitive-restart value: (draw, index position).
    Restart(usize, usize),
    /// The resolved triangles could not be rasterised.
    Raster(crate::depth::DepthRefError),
}

impl fmt::Display for IndexedRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for IndexedRefError {}

/// The vertex numbers draw `draw` of a pass fetches, in index order.
///
/// # Errors
///
/// Returns an error for a count that is zero or not a multiple of 3, an index
/// range past the buffer, a primitive-restart index, or a fetched vertex
/// outside `0..vertex_count`.
pub fn fetch(
    draw_number: usize,
    draw: &RefIndexedDraw,
    indices: RefIndices<'_>,
    vertex_count: usize,
) -> Result<Vec<usize>, IndexedRefError> {
    if draw.count == 0 || !draw.count.is_multiple_of(3) {
        return Err(IndexedRefError::Count(draw_number));
    }
    let end = draw
        .start
        .checked_add(draw.count)
        .filter(|&end| end <= indices.len())
        .ok_or(IndexedRefError::IndexRange(draw_number))?;
    (draw.start..end)
        .map(|position| {
            let index = indices.get(position).expect("position checked above");
            if index == indices.restart() {
                return Err(IndexedRefError::Restart(draw_number, position));
            }
            usize::try_from(i64::from(index) + i64::from(draw.base_vertex))
                .ok()
                .filter(|&vertex| vertex < vertex_count)
                .ok_or(IndexedRefError::VertexRange(draw_number, position))
        })
        .collect()
}

/// Render indexed draws in order over a target cleared to `clear_color`.
///
/// # Errors
///
/// Returns an error when [`fetch`] rejects a draw or the resolved triangles
/// cannot be rasterised (see [`render_depth`]).
pub fn render_indexed(
    extent: [usize; 2],
    clear_color: [u8; 4],
    vertices: &[[f32; 4]],
    indices: RefIndices<'_>,
    draws: &[RefIndexedDraw],
    texture: Option<RefTexture<'_>>,
) -> Result<Vec<[u8; 4]>, IndexedRefError> {
    let mut triangles = Vec::new();
    for (number, draw) in draws.iter().enumerate() {
        let fetched = fetch(number, draw, indices, vertices.len())?;
        triangles.extend(fetched.as_chunks::<3>().0.iter().map(|triangle| RefDraw {
            positions: triangle.map(|vertex| vertices[vertex]),
            fill: draw.fill,
            compare: CompareFunction::Always,
            write: false,
        }));
    }
    render_depth(extent, clear_color, None, &triangles, texture)
        .map(|(color, _)| color)
        .map_err(IndexedRefError::Raster)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(start: usize, count: usize, base_vertex: i32) -> RefIndexedDraw {
        RefIndexedDraw {
            start,
            count,
            base_vertex,
            fill: RefFill::Flat([255; 4]),
        }
    }

    #[test]
    fn fetch_applies_start_and_signed_base_vertex() {
        let indices = RefIndices::U16(&[7, 0, 1, 2, 5]);
        assert_eq!(fetch(0, &draw(1, 3, 2), indices, 5), Ok(vec![2, 3, 4]));
        assert_eq!(fetch(0, &draw(2, 3, -1), indices, 5), Ok(vec![0, 1, 4]));
    }

    #[test]
    fn fetch_rejects_bad_counts_ranges_and_vertices() {
        let indices = RefIndices::U32(&[0, 1, 2, u32::MAX]);
        assert_eq!(
            fetch(3, &draw(0, 0, 0), indices, 3),
            Err(IndexedRefError::Count(3))
        );
        assert_eq!(
            fetch(0, &draw(0, 2, 0), indices, 3),
            Err(IndexedRefError::Count(0))
        );
        assert_eq!(
            fetch(0, &draw(2, 3, 0), indices, 3),
            Err(IndexedRefError::IndexRange(0))
        );
        assert_eq!(
            fetch(0, &draw(usize::MAX, 3, 0), indices, 3),
            Err(IndexedRefError::IndexRange(0))
        );
        assert_eq!(
            fetch(0, &draw(0, 3, -1), indices, 3),
            Err(IndexedRefError::VertexRange(0, 0))
        );
        assert_eq!(
            fetch(0, &draw(0, 3, 1), indices, 3),
            Err(IndexedRefError::VertexRange(0, 2))
        );
        assert_eq!(
            fetch(0, &draw(1, 3, 0), indices, 3),
            Err(IndexedRefError::Restart(0, 3))
        );
        // The raw restart value is rejected even when the base vertex would
        // bring it into range.
        let high = RefIndices::U16(&[65533, 65534, 65535]);
        assert_eq!(
            fetch(0, &draw(0, 3, -65533), high, 3),
            Err(IndexedRefError::Restart(0, 2))
        );
        let high = RefIndices::U16(&[65532, 65533, 65534]);
        assert_eq!(fetch(0, &draw(0, 3, -65532), high, 3), Ok(vec![0, 1, 2]));
    }
}
