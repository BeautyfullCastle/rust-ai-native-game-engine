#![forbid(clippy::float_arithmetic)]

use orr_fp::{float_interop as _, FP};

pub fn view_conversion(value: FP) -> f32 {
    FP::to_f32(value)
}

pub fn authored_value(value: f64) -> FP {
    FP::from_f64(value)
}
