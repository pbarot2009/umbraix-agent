pub mod client;
pub mod error;
pub mod prompt;
pub mod types;

pub use client::{GeminiClient, GenerateParams};
pub use error::GeminiError;
