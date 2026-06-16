use anyhow::Result;
use clap::{Parser, Subcommand};
use linux_conductor_core::doctor;
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::{AddRepository, RepositoryStore};
use linux_conductor_core::workspace::{CreateWorkspace, WorkspaceStore};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "linux-conductor")]
#[command(about = "Linux-native Git worktree workflow for parallel coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "origin")]
        remote: String,
        #[arg(long)]
        default_branch: Option<String>,
        #[arg(long)]
        workspace_parent: Option<PathBuf>,
    },
    List,
    Doctor {
        name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    Create {
        repository: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        branch: String,
        #[arg(long)]
        base: Option<String>,
    },
    List,
    Archive {
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::from_env();

    match cli.command {
        Command::Doctor => print_doctor(doctor::report_from_host()),
        Command::Repo { command } => {
            let store = RepositoryStore::open(paths.database_path)?;
            match command {
                RepoCommand::Add {
                    path,
                    name,
                    remote,
                    default_branch,
                    workspace_parent,
                } => {
                    let repo = store.add(AddRepository {
                        name,
                        root_path: path,
                        default_branch,
                        remote_name: remote,
                        workspace_parent_path: workspace_parent,
                    })?;
                    println!(
                        "Added {} at {} (default branch: {}, workspace parent: {})",
                        repo.name,
                        repo.root_path.display(),
                        repo.default_branch,
                        repo.workspace_parent_path.display()
                    );
                }
                RepoCommand::List => {
                    for repo in store.list()? {
                        println!(
                            "{}\t{}\t{}\t{}",
                            repo.name,
                            repo.root_path.display(),
                            repo.remote_name,
                            repo.default_branch
                        );
                    }
                }
                RepoCommand::Doctor { name: _ } => {
                    print_doctor(doctor::report_from_host());
                }
            }
        }
        Command::Workspace { command } => {
            let store = WorkspaceStore::open(paths.database_path)?;
            match command {
                WorkspaceCommand::Create {
                    repository,
                    name,
                    branch,
                    base,
                } => {
                    let workspace = store.create(CreateWorkspace {
                        repository_name: repository,
                        name,
                        branch,
                        base_ref: base,
                    })?;
                    println!(
                        "Created {} at {} (branch: {}, base: {}, port: {})",
                        workspace.name,
                        workspace.path.display(),
                        workspace.branch,
                        workspace.base_ref,
                        workspace.port_base
                    );
                }
                WorkspaceCommand::List => {
                    for workspace in store.list()? {
                        println!(
                            "{}\t{}\t{}\t{}\t{}\t{}",
                            workspace.name,
                            workspace.path.display(),
                            workspace.branch,
                            workspace.base_ref,
                            workspace.port_base,
                            workspace.status
                        );
                    }
                }
                WorkspaceCommand::Archive { name } => {
                    let workspace = store.archive(&name)?;
                    println!(
                        "Archived {} at {}",
                        workspace.name,
                        workspace.path.display()
                    );
                }
            }
        }
    }

    Ok(())
}

fn print_doctor(report: doctor::DoctorReport) {
    let distro = report.distro_id.as_deref().unwrap_or("unknown");
    println!("Distro: {distro}");

    if let Some(command) = report.install_command {
        println!("Install required tools: {command}");
    } else {
        println!(
            "Install required tools: see your distro packages for git, gh, sqlite, and openssh"
        );
    }

    for dependency in report.dependencies {
        let required = if dependency.required {
            "required"
        } else {
            "optional"
        };
        let status = if dependency.installed {
            "ok"
        } else {
            "missing"
        };
        println!("{:<8} {:<8} {}", dependency.name, required, status);
    }
}
