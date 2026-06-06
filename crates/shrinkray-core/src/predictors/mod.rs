//! v0.7.4 — Real predictors for the Δ-Codec.
//!
//! The delta-codec crate intentionally has no ONNX runtime dependency so it
//! can publish standalone. This module lives in `shrinkray-core` (which
//! already pulls in `ort` behind the `inference` feature) and provides
//! concrete `Predictor` impls that bridge to ORT.
//!
//! v0.7.4 ships one production predictor:
//!   - [`esrgan::EsrganX4Predictor`] — Real-ESRGAN x4 (dynamic-input ONNX
//!     export). The load-bearing piece for the thesis "AI predictors shrink
//!     residuals enough to beat byte-exact backups."

#[cfg(feature = "inference")]
pub mod esrgan;

#[cfg(feature = "inference")]
pub use esrgan::EsrganX4Predictor;
