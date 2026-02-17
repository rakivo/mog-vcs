#![allow(unused, dead_code)]

mod hash;
mod object;
mod tree_builder;
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
mod util;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from("."));
            Repository::init(&path)?;
            println!("Initialized empty vx repository in {}/.vx", path.display());
        }

        Commands::HashObject { write, file } => {
            let repo = Repository::open(&PathBuf::from("."))?;
            hash_object::hash_object(&repo, &file, write)?;
        }

        Commands::CatFile { hash } => {
            let repo = Repository::open(&PathBuf::from("."))?;
            cat_file::cat_file(&repo, &hash)?;
        }

        Commands::WriteTree => {
            let repo = Repository::open(&PathBuf::from("."))?;
            let hash = write_tree::write_tree(&repo, &PathBuf::from("."))?;
            println!("{}", hash::hash_to_hex(&hash));
        }

        Commands::Log => {
            let repo = Repository::open(&PathBuf::from("."))?;
            let mut buf = String::new();
            log::log(&repo, &mut buf)?;
            println!("{buf}");
        }

        Commands::Checkout { branch, path, new_branch } => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;
            if new_branch {
                //
                // Create branch at HEAD then switch to it
                //
                branch::create(&repo, &branch, None)?;
                checkout::checkout(&repo, &branch)?;
            } else {
                match path {
                    Some(p) => checkout::checkout_path(&repo, &branch, &p)?,
                    None    => checkout::checkout(&repo, &branch)?,
                }
            }
        }

        Commands::Branch { name, at, delete, force_delete, rename_to } => {
            let repo = Repository::open(&PathBuf::from("."))?;

            if let Some(branch) = delete {
                branch::delete(&repo, &branch)?;
            } else if let Some(branch) = force_delete {
                branch::force_delete(&repo, &branch)?;
            } else if rename_to.len() == 2 {
                branch::rename(&repo, &rename_to[0], &rename_to[1])?;
            } else if let Some(name) = name {
                branch::create(&repo, &name, at.as_deref())?;
            } else {
                branch::list(&repo)?;
            }
        }

        Commands::Add { files } => {
            let repo = Repository::open(&PathBuf::from("."))?;
            add::add(&repo, &files)?;
        }

        Commands::Commit { message, author } => {
            let repo = Repository::open(&PathBuf::from("."))?;
            let index = index::Index::load(&repo.root)?;
            if index.count == 0 {
                eprintln!("nothing staged to commit (use 'vx add <file>'...)");
                return Ok(());
            }
            let tree = index.write_tree_recursive(&repo)?;
            let parent = repo.read_head_commit().ok();
            commit::commit(&repo, tree, parent, &author, &message)?;
        }
    }

    Ok(())
}
