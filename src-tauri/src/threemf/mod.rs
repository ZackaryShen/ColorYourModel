//! Read side of the 3MF format: OPC package access, model XML, plate grouping,
//! palette resolution, `paint_color` decoding and triangle subdivision rebuild.

pub mod paint;
pub mod subdivide;

#[cfg(test)]
pub mod test_support;
