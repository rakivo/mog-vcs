use mog::repository::Repository;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(name = "mog")]
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
    /// Add paths to the index (stage)
    Stage {
        files: Vec<PathBuf>,
    },
    /// Remove paths from the index (unstage)
    Unstage {
        files: Vec<PathBuf>,
    },
    /// Discard working directory changes, restoring to index state.
    Discard {
        /// Paths to discard (omit to discard everything).
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

        /// Rename: mog branch -m old new
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
            println!("Initialized empty mog repository in {}/.mog", path.display());
        }

        Commands::HashObject { write, file } => {
            let mut repo = Repository::open(".")?;
            mog::hash_object::hash_object(&mut repo, &file, write)?;
        }

        Commands::CatFile { hash } => {
            let mut repo = Repository::open(".")?;
            let mut buf = String::new();
            mog::cat_file::cat_file(&mut repo, &hash, &mut buf)?;
            println!("{buf}");
        }

        Commands::WriteTree => {
            let mut repo = Repository::open(".")?;
            let hash = mog::write_tree::write_tree(&mut repo, ".")?;
            println!("{}", mog::hash::hash_to_hex(&hash));
        }

        Commands::Log => {
            let mut repo = Repository::open(".")?;
            let mut buf = String::new();
            mog::log::log(&mut repo, &mut buf)?;
            print!("{buf}");
        }

        Commands::Checkout { branch, path, new_branch } => {
            let mut repo = Repository::open(".")?;
            if new_branch {
                mog::branch::create(&mut repo, &branch, None)?;
                mog::checkout::checkout(&mut repo, &branch)?;
            } else {
                match path {
                    Some(p) => mog::checkout::checkout_path(&mut repo, &branch, &p)?,
                    None => mog::checkout::checkout(&mut repo, &branch)?,
                }
            }
        }

        Commands::Discard { files } => {
            let mut repo = Repository::open(".")?;
            mog::discard::discard(&mut repo, &files)?;
        }

        Commands::Branch { name, at, delete, force_delete, rename_to } => {
            let mut repo = Repository::open(".")?;

            if let Some(branch) = delete {
                mog::branch::delete(&mut repo, &branch)?;
            } else if let Some(branch) = force_delete {
                mog::branch::force_delete(&mut repo, &branch)?;
            } else if rename_to.len() == 2 {
                mog::branch::rename(&repo, &rename_to[0], &rename_to[1])?;
            } else if let Some(name) = name {
                mog::branch::create(&mut repo, &name, at.as_deref())?;
            } else {
                mog::branch::list(&repo)?;
            }
        }

        Commands::Stage { files } => {
            let mut repo = Repository::open(".")?;
            mog::stage::stage(&mut repo, &files)?;
        }

        Commands::Unstage { files } => {
            let mut repo = Repository::open(".")?;
            mog::unstage::unstage(&mut repo, &files)?;
        }

        Commands::Status => {
            let mut repo = Repository::open(".")?;
            mog::status::status(&mut repo)?;
        }

        Commands::Commit { message, author } => {
            let mut repo = Repository::open(".")?;
            let index = mog::index::Index::load(&repo.root)?;
            if index.count == 0 {
                eprintln!("nothing staged to commit (use 'mog add <file>'...)");
                return Ok(());
            }
            let tree = index.write_tree(&mut repo)?;
            let parent = repo.read_head_commit().ok();
            mog::commit::commit(&mut repo, tree, parent, &author, &message)?;
        }
    }

    Ok(())
}
