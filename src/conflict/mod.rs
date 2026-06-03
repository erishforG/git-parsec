mod detector;
mod simulator;

pub use detector::{detect, FileConflict};
pub use simulator::{simulate, MergeSimulation};
