#![allow(unused, dead_code)]

use clap::{Parser, Subcommand};
use anyhow::Result;
use std::path::PathBuf;

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
    Checkout {
        branch: String,
    },
    Commit {
        #[arg(short = 'm')]
        message: String,

        #[arg(long, default_value = "Your Name")]
        author: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from("."));
            repository::Repository::init(&path)?;
            println!("Initialized empty vx repository in {}/.vx", path.display());
        }

        Commands::HashObject { write, file } => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;
            hash_object::hash_object(&repo, &file, write)?;
        }

        Commands::CatFile { hash } => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;
            cat_file::cat_file(&repo, &hash)?;
        }

        Commands::WriteTree => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;
            let hash = write_tree::write_tree(&repo, &PathBuf::from("."))?;
            println!("{}", hash::hash_to_hex(&hash));
        }

        Commands::Log => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;
            let mut buf = String::new();
            log::log(&repo, "refs/heads/main", &mut buf)?;
            println!("{buf}");
        }

        Commands::Checkout { branch } => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;
            checkout::checkout(&repo, &branch)?;
        }

        Commands::Commit { message, author } => {
            let repo = repository::Repository::open(&PathBuf::from("."))?;

            // Write tree from current directory
            let tree = write_tree::write_tree(&repo, &PathBuf::from("."))?;

            // Get parent (if exists)
            let parent = repo.read_ref("refs/heads/main").ok();

            // Create commit
            commit::commit(&repo, tree, parent, &author, &message)?;
        }
    }

    Ok(())
}
