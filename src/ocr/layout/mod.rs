//! Turning a bag of detected boxes into something a human would read in order.

pub mod reading_order;

pub use reading_order::{assemble, group_into_lines};
