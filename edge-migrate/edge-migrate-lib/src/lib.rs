//! edge-migrate-lib: Core analysis + transformation library for edge-migrate.
//!
//! Provides C AST analysis via tree-sitter, POSIX pattern detection,
//! POSIX → WASI transformation, and structured migration reports.

pub mod analyzer;
pub mod patterns;
pub mod preprocessor;
pub mod report;
pub mod transformer;
pub mod tree;

pub use analyzer::CAnalyzer;
pub use patterns::{is_valid_deployment_app_name, PatternMatch, PosixPattern, Transformability};
pub use preprocessor::{ExpandedSource, PreprocessError, Preprocessor, PreprocessorInfo};
pub use report::{FileReport, MigrationReport, TreeMigrationReport};
pub use transformer::{TransformResult, Transformer};
pub use tree::{transform_tree, transform_tree_with_app_name, walk_tree, FileEntry, TreeTransformResult, WalkError};
