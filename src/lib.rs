#![warn(
    clippy::all,
    clippy::pedantic,
    // clippy::cargo,
    dead_code
)]
#![allow(
    clippy::inline_always,
    clippy::uninlined_format_args, // ?...
    clippy::borrow_as_ptr,
    clippy::single_match_else,
    clippy::collapsible_if,
    clippy::new_without_default,
    clippy::redundant_field_names,
    clippy::struct_field_names,
    clippy::ptr_as_ptr,
    clippy::missing_transmute_annotations,
    clippy::multiple_crate_versions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::similar_names,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::used_underscore_binding,
    clippy::nonstandard_macro_braces,
    clippy::used_underscore_items,
    clippy::enum_glob_use,
    clippy::cast_lossless,
    clippy::match_same_arms,
    clippy::too_many_lines,
    clippy::unnested_or_patterns,
    clippy::blocks_in_conditions,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
)]

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod hash;
pub mod object;
pub mod store;
pub mod wire;
pub mod storage;
pub mod repository;
pub mod hash_object;
pub mod cat_file;
pub mod write_tree;
pub mod commit;
pub mod log;
pub mod checkout;
pub mod stage;
pub mod index;
pub mod branch;
pub mod cache;
pub mod ignore;
pub mod status;
pub mod unstage;
pub mod util;
pub mod tracy;
pub mod tree;
pub mod stash;
pub mod discard;
pub mod storage_mock;
