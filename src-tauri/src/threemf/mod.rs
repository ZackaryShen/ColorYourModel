//! Read side of the 3MF format: OPC package access, model XML, plate grouping,
//! palette resolution, `paint_color` decoding and triangle subdivision rebuild.

pub mod assemble;
pub mod materials;
pub mod model;
pub mod opc;
pub mod paint;
pub mod plates;
pub mod subdivide;

#[cfg(test)]
pub mod test_support;
