//! Code copied from Incisive at 54433fc: the writer's prompt, schema and checks.
//! Nothing here calls a model or names a Claude type.

pub mod answer;
pub mod numbers;
pub mod tokens;
pub mod wrap;
pub mod writer;

#[cfg(test)]
mod writer_tests;
