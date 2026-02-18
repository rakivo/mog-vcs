#![warn(clippy::all, clippy::pedantic, clippy::cargo, dead_code)]
#![allow(
    clippy::inline_always,
    clippy::uninlined_format_args, // ?...
    clippy::borrow_as_ptr,
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

mod hash;
mod object;
mod store;
mod wire;
mod storage;
mod repository;
mod hash_object;
mod cat_file;
mod write_tree;
mod commit;
mod log;
mod checkout;
mod add;
mod index;
mod branch;
mod cache;
mod ignore;
mod status;
mod remove;
mod util;
mod tracy;
mod tree;

use repository::Repository;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(name = "vx")]
#[command(about = "A fast version control system")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        path: Option<PathBuf>,
    },
    HashObject {
        #[arg(short = 'w')]
        write: bool,
        file: PathBuf,
    },
    CatFile {
        hash: String,
    },
    WriteTree,
    Log,
    Add {
        files: Vec<PathBuf>,
    },
    /// Remove paths from the index (unstage)
    Remove {
        files: Vec<PathBuf>,
    },
    Checkout {
        branch: String,

        #[arg(short = 'p', long)]
        path: Option<String>,

        /// Create and switch to a new branch
        #[arg(short = 'b', long)]
        new_branch: bool,
    },
    Commit {
        #[arg(short = 'm')]
        message: String,

        #[arg(long, default_value = "Your Name")]
        author: String,
    },
    Branch {
        /// Name of branch to create (omit to list branches)
        name: Option<String>,

        /// Create at specific commit or branch instead of HEAD
        #[arg(long)]
        at: Option<String>,

        /// Delete branch (safe)
        #[arg(short = 'd', long, conflicts_with_all = ["force_delete", "rename_to", "name"])]
        delete: Option<String>,

        /// Force delete branch
        #[arg(short = 'D', long = "force-delete", conflicts_with_all = ["delete", "rename_to", "name"])]
        force_delete: Option<String>,

        /// Rename: vx branch -m old new
        #[arg(short = 'm', long = "rename", num_args = 2, conflicts_with_all = ["delete", "force_delete"])]
        rename_to: Vec<String>,
    },
    /// Show working tree status (staged, modified, deleted, untracked)
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracy_client::Client::start();

    match cli.command {
        Commands::Init { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from("."));
            Repository::init(&path)?;
            println!("Initialized empty vx repository in {}/.vx", path.display());
        }

        Commands::HashObject { write, file } => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            hash_object::hash_object(&mut repo, &file, write)?;
        }

        Commands::CatFile { hash } => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            cat_file::cat_file(&mut repo, &hash)?;
        }

        Commands::WriteTree => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            let hash = write_tree::write_tree(&mut repo, &PathBuf::from("."))?;
            println!("{}", hash::hash_to_hex(&hash));
        }

        Commands::Log => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            let mut buf = String::new();
            log::log(&mut repo, &mut buf)?;
            print!("{buf}");
        }

        Commands::Checkout { branch, path, new_branch } => {
            let mut repo = repository::Repository::open(&PathBuf::from("."))?;
            if new_branch {
                branch::create(&mut repo, &branch, None)?;
                checkout::checkout(&mut repo, &branch)?;
            } else {
                match path {
                    Some(p) => checkout::checkout_path(&mut repo, &branch, &p)?,
                    None => checkout::checkout(&mut repo, &branch)?,
                }
            }
        }

        Commands::Branch { name, at, delete, force_delete, rename_to } => {
            let mut repo = Repository::open(&PathBuf::from("."))?;

            if let Some(branch) = delete {
                branch::delete(&mut repo, &branch)?;
            } else if let Some(branch) = force_delete {
                branch::force_delete(&mut repo, &branch)?;
            } else if rename_to.len() == 2 {
                branch::rename(&repo, &rename_to[0], &rename_to[1])?;
            } else if let Some(name) = name {
                branch::create(&mut repo, &name, at.as_deref())?;
            } else {
                branch::list(&repo)?;
            }
        }

        Commands::Add { files } => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            add::add(&mut repo, &files)?;
        }

        Commands::Remove { files } => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            remove::remove(&mut repo, &files)?;
        }

        Commands::Status => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            status::status(&mut repo)?;
        }

        Commands::Commit { message, author } => {
            let mut repo = Repository::open(&PathBuf::from("."))?;
            let index = index::Index::load(&repo.root)?;
            if index.count == 0 {
                eprintln!("nothing staged to commit (use 'vx add <file>'...)");
                return Ok(());
            }
            let tree = index.write_tree(&mut repo)?;
            let parent = repo.read_head_commit().ok();
            commit::commit(&mut repo, tree, parent, &author, &message)?;
        }
    }

    Ok(())
}
