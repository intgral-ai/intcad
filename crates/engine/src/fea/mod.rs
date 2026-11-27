//! A stress estimate of a part (PIL-294). This first part holds the files the estimate trades with
//! the external mesher (gmsh) and solver (CalculiX), and the step from gmsh's mesh to a CalculiX
//! job. Running them, and the command that does, come next.

pub mod formats;
pub mod model;
