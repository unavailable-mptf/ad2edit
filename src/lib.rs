//! AD2 dupe reading, editing and writing.
//!
//! The CLI (`ad2read`) and the GUI (`ad2edit`) are both thin shells over this.

pub mod arcs;
pub mod build;
pub mod buildfile;
pub mod doctor;
pub mod driveshaft;
pub mod gen;
pub mod gizmo;
pub mod graph;
pub mod pipeline;
pub mod ports;
pub mod primitive;
pub mod ports_table;
pub mod roles;
pub mod rules;
pub mod scale;
pub mod json;
pub mod look;
pub mod bulk;
pub mod catalog;
pub mod codec;
pub mod config;
pub mod containers;
pub mod dupe;
pub mod dupefile;
pub mod duplicate;
pub mod extras;
#[cfg(test)]
pub mod fixture;
pub mod gallery;
pub mod history;
pub mod hull;
pub mod merge;
pub mod map;
pub mod mass;
pub mod models;
pub mod gpu;
pub mod raster;
pub mod refs;
pub mod render;
pub mod sprops;
pub mod sprops_table;
pub mod syntax;
pub mod theme;
pub mod transform;
pub mod tutorial;
pub mod verbs;
pub mod wire_items;
pub mod value;
