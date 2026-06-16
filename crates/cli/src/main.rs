use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use linux_conductor_core::doctor;
use linux_conductor_core::paths::AppPaths;
use linux_conductor_core::repository::{AddRepository, RepositoryStore};
use linux_conductor_core::workspace::{
    CreateWorkspace, SessionKind, WorkspaceStatusLine, WorkspaceStore,
};
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
    Run {
        workspace: String,
    },
    Stop {
        workspace: String,
    },
    Logs {
        workspace: String,
        #[arg(long)]
        run: bool,
        #[arg(long)]
        session: bool,
    },
    Runs {
        workspace: String,
    },
    Diff {
        workspace: String,
        #[arg(long)]
        name_only: bool,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Pr {
        #[command(subcommand)]
        command: PrCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Todo {
        #[command(subcommand)]
        command: TodoCommand,
    },
    Checks {
        workspace: String,
    },
    Open {
        workspace: String,
        #[arg(long, default_value = "code")]
        editor: String,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    Archive {
        name: String,
        #[arg(long)]
        remove_worktree: bool,
    },
    Status,
    Checkpoint {
        #[command(subcommand)]
        command: CheckpointCommand,
    },
    Conflicts {
        workspace: String,
    },
    Discard {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    Status { workspace: String },
}

#[derive(Debug, Subcommand)]
enum ReviewCommand {
    Add {
        workspace: String,
        file: String,
        #[arg(long)]
        line: Option<i64>,
        body: Vec<String>,
    },
    List {
        workspace: String,
    },
    Resolve {
        id: i64,
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
    Update {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    Create {
        repository: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        from_issue: Option<u64>,
        #[arg(long)]
        branch_prefix: Option<String>,
    },
    List {
        #[arg(long)]
        active: bool,
    },
    Archive {
        name: String,
        #[arg(long)]
        remove_worktree: bool,
    },
    Restore {
        name: String,
    },
    Discard {
        name: String,
    },
    Rename {
        name: String,
        new_name: String,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    Start {
        workspace: String,
        #[arg(long, value_enum, default_value_t = CliSessionKind::Shell)]
        kind: CliSessionKind,
    },
    Stop {
        workspace: String,
    },
    List {
        workspace: String,
    },
}

#[derive(Debug, Subcommand)]
enum PrCommand {
    Create {
        workspace: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        from_context: bool,
    },
    Checks {
        workspace: String,
    },
    View {
        workspace: String,
    },
    Merge {
        workspace: String,
        #[arg(long, default_value = "squash")]
        method: String,
    },
}

#[derive(Debug, Subcommand)]
enum TodoCommand {
    Add {
        workspace: String,
        text: Vec<String>,
    },
    List {
        workspace: String,
    },
    Done {
        id: i64,
    },
    Sync {
        workspace: String,
    },
}

#[derive(Debug, Subcommand)]
enum CheckpointCommand {
    Create {
        workspace: String,
        #[arg(long)]
        session: Option<i64>,
        message: Vec<String>,
    },
    List {
        workspace: String,
    },
    Restore {
        workspace: String,
        id: i64,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSessionKind {
    Shell,
    Codex,
    Claude,
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
                    for (repo, active, total) in store.list_with_workspace_counts()? {
                        println!(
                            "{:<20} {:<10} {:<6} {:>2} active / {:>2} total  {}",
                            repo.name,
                            repo.default_branch,
                            repo.remote_name,
                            active,
                            total,
                            repo.root_path.display(),
                        );
                    }
                }
                RepoCommand::Doctor { name: _ } => {
                    print_doctor(doctor::report_from_host());
                }
                RepoCommand::Update { name } => {
                    let repo = store.update(&name)?;
                    println!(
                        "Updated {} (default branch: {})",
                        repo.name, repo.default_branch
                    );
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
                    from_issue,
                    branch_prefix,
                } => {
                    let workspace = if let Some(issue) = from_issue {
                        store.create_from_issue(&repository, issue, branch_prefix.as_deref())?
                    } else {
                        let name =
                            name.with_context(|| "--name is required when not using --from-issue")?;
                        let branch = branch
                            .with_context(|| "--branch is required when not using --from-issue")?;
                        store.create(CreateWorkspace {
                            repository_name: repository,
                            name,
                            branch,
                            base_ref: base,
                        })?
                    };
                    println!(
                        "Created {} at {} (branch: {}, base: {}, port: {})",
                        workspace.name,
                        workspace.path.display(),
                        workspace.branch,
                        workspace.base_ref,
                        workspace.port_base
                    );
                }
                WorkspaceCommand::List { active } => {
                    for workspace in store.list()? {
                        if active && workspace.status != "active" {
                            continue;
                        }
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
                WorkspaceCommand::Archive {
                    name,
                    remove_worktree,
                } => {
                    let workspace = store.archive(&name, remove_worktree)?;
                    println!(
                        "Archived {} at {}",
                        workspace.name,
                        workspace.path.display()
                    );
                }
                WorkspaceCommand::Restore { name } => {
                    let workspace = store.restore(&name)?;
                    println!(
                        "Restored {} at {} (branch: {})",
                        workspace.name,
                        workspace.path.display(),
                        workspace.branch
                    );
                }
                WorkspaceCommand::Discard { name } => {
                    let workspace = store.discard(&name)?;
                    println!(
                        "Discarded {} — worktree removed and branch deleted",
                        workspace.name
                    );
                }
                WorkspaceCommand::Rename { name, new_name } => {
                    let workspace = store.rename(&name, &new_name)?;
                    println!(
                        "Renamed {} to {} at {}",
                        name,
                        workspace.name,
                        workspace.path.display()
                    );
                }
            }
        }
        Command::Run { workspace } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            let process = store.run_workspace(&workspace)?;
            println!(
                "Started run for {} as pid {} (log: {})",
                workspace,
                process.pid,
                process.log_path.display()
            );
        }
        Command::Stop { workspace } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            let process = store.stop_workspace(&workspace)?;
            println!("Stopped run for {} (pid {})", workspace, process.pid);
        }
        Command::Logs {
            workspace,
            run,
            session,
        } => {
            if run == session {
                anyhow::bail!(
                    "choose exactly one log stream, for example: linux-conductor logs {workspace} --run"
                );
            }
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            if run {
                print!("{}", store.read_latest_run_log(&workspace)?);
            } else {
                print!("{}", store.read_latest_session_log(&workspace)?);
            }
        }
        Command::Runs { workspace } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            for run in store.list_runs(&workspace)? {
                println!(
                    "#{}\t{}\t{}\t{}\t{}",
                    run.id,
                    run.status.as_str(),
                    run.started_at,
                    run.ended_at.as_deref().unwrap_or("-"),
                    run.log_path.display(),
                );
            }
        }
        Command::Diff {
            workspace,
            name_only,
            file,
        } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            if name_only {
                for path in store.changed_files(&workspace)? {
                    println!("{path}");
                }
            } else {
                print!("{}", store.unified_diff(&workspace, file.as_deref())?);
            }
        }
        Command::Pr { command } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                PrCommand::Create {
                    workspace,
                    title,
                    body,
                    draft,
                    from_context,
                } => {
                    let body = if from_context && body.is_none() {
                        store.read_context_brief(&workspace)?
                    } else {
                        body
                    };
                    store.push_branch(&workspace)?;
                    print!(
                        "{}",
                        store.create_pull_request(
                            &workspace,
                            title.as_deref(),
                            body.as_deref(),
                            draft
                        )?
                    );
                }
                PrCommand::Checks { workspace } => {
                    print!("{}", store.pull_request_checks(&workspace)?);
                }
                PrCommand::View { workspace } => {
                    match store.refresh_pull_request_state(&workspace)? {
                        Some(pr) => println!("#{} {} (state: {})", pr.number, pr.url, pr.state),
                        None => println!("No pull request recorded for {workspace}"),
                    }
                }
                PrCommand::Merge { workspace, method } => {
                    print!("{}", store.merge_pull_request(&workspace, &method)?);
                    println!("Merged pull request for {workspace}");
                }
            }
        }
        Command::Session { command } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                SessionCommand::Start { workspace, kind } => {
                    let process = store.start_session(&workspace, kind.into())?;
                    println!(
                        "Started session for {} as pid {} (log: {})",
                        workspace,
                        process.pid,
                        process.log_path.display()
                    );
                }
                SessionCommand::Stop { workspace } => {
                    let process = store.stop_session(&workspace)?;
                    println!("Stopped session for {} (pid {})", workspace, process.pid);
                }
                SessionCommand::List { workspace } => {
                    for session in store.list_sessions(&workspace)? {
                        println!(
                            "#{}\t{}\t{}\t{}\t{}",
                            session.id,
                            session.status.as_str(),
                            session.started_at,
                            session.ended_at.as_deref().unwrap_or("-"),
                            session.command,
                        );
                    }
                }
            }
        }
        Command::Todo { command } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                TodoCommand::Add { workspace, text } => {
                    let todo = store.add_todo(&workspace, &text.join(" "))?;
                    println!("Added todo #{} to {}: {}", todo.id, workspace, todo.text);
                }
                TodoCommand::List { workspace } => {
                    for todo in store.list_todos(&workspace)? {
                        println!("#{}\t{}\t{}", todo.id, todo.status, todo.text);
                    }
                }
                TodoCommand::Done { id } => {
                    let todo = store.complete_todo(id)?;
                    println!("Completed todo #{}: {}", todo.id, todo.text);
                }
                TodoCommand::Sync { workspace } => {
                    let n = store.sync_todos_from_context(&workspace)?;
                    println!("Imported {n} todo(s) from .context/todos.md into {workspace}");
                }
            }
        }
        Command::Checks { workspace } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            print_checks_summary(store.checks_summary(&workspace)?);
        }
        Command::Open { workspace, editor } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            let launch = store.editor_launch(&workspace, &editor)?;
            let mut cmd = std::process::Command::new(&launch.program);
            cmd.args(&launch.args)
                .current_dir(&launch.cwd)
                .envs(launch.env);
            cmd.spawn()
                .with_context(|| format!("launch editor {editor} for workspace {workspace}"))?;
            println!("Opened {} in {editor}", launch.cwd.display());
        }
        Command::Mcp { command } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                McpCommand::Status { workspace } => {
                    print_mcp_status(store.mcp_status(&workspace)?);
                }
            }
        }
        Command::Review { command } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                ReviewCommand::Add {
                    workspace,
                    file,
                    line,
                    body,
                } => {
                    let comment =
                        store.add_review_comment(&workspace, &file, line, &body.join(" "))?;
                    println!(
                        "Added review comment #{} on {}{}",
                        comment.id,
                        file,
                        line.map(|l| format!(":{l}")).unwrap_or_default()
                    );
                }
                ReviewCommand::List { workspace } => {
                    for comment in store.list_review_comments(&workspace)? {
                        let line = comment
                            .line_number
                            .map(|l| format!(":{l}"))
                            .unwrap_or_default();
                        println!(
                            "#{}\t{}\t{}{}\t{}",
                            comment.id, comment.status, comment.file_path, line, comment.body
                        );
                    }
                }
                ReviewCommand::Resolve { id } => {
                    let comment = store.resolve_review_comment(id)?;
                    println!(
                        "Resolved review comment #{} on {}",
                        comment.id, comment.file_path
                    );
                }
            }
        }
        Command::Archive {
            name,
            remove_worktree,
        } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            let workspace = store.archive(&name, remove_worktree)?;
            println!(
                "Archived {} at {}",
                workspace.name,
                workspace.path.display()
            );
        }
        Command::Status => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            print_status(store.list_status()?);
        }
        Command::Checkpoint { command } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                CheckpointCommand::Create {
                    workspace,
                    session,
                    message,
                } => {
                    let cp = store.checkpoint_create(&workspace, &message.join(" "), session)?;
                    println!(
                        "Created checkpoint #{} for {} (ref: {})",
                        cp.id, workspace, cp.git_ref
                    );
                }
                CheckpointCommand::List { workspace } => {
                    for cp in store.checkpoint_list(&workspace)? {
                        println!("#{}\t{}\t{}", cp.id, cp.created_at, cp.message);
                    }
                }
                CheckpointCommand::Restore { workspace, id } => {
                    let cp = store.checkpoint_restore(&workspace, id)?;
                    println!(
                        "Restored {} to checkpoint #{} ({})",
                        workspace, cp.id, cp.git_ref
                    );
                    println!("Warning: untracked files removed. Re-run setup if needed.");
                }
            }
        }
        Command::Conflicts { workspace } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            let conflicts = store.find_conflicting_workspaces(&workspace)?;
            if conflicts.is_empty() {
                println!("No file conflicts with other active workspaces.");
            } else {
                for (other, files) in &conflicts {
                    println!("Conflicts with {other}:");
                    for f in files {
                        println!("  {f}");
                    }
                }
            }
        }
        Command::Discard { name } => {
            let store = WorkspaceStore::open_with_logs(paths.database_path, paths.logs_dir)?;
            let workspace = store.discard(&name)?;
            println!(
                "Discarded {} — worktree removed and branch deleted",
                workspace.name
            );
        }
    }

    Ok(())
}

fn print_checks_summary(summary: linux_conductor_core::workspace::ChecksSummary) {
    println!(
        "Workspace: {} ({})",
        summary.workspace.name, summary.workspace.status
    );
    println!("Branch:    {}", summary.workspace.branch);
    match &summary.branch_push_state {
        Some(state) if !state.has_upstream => {
            println!("Push:      no upstream set (push with: linux-conductor pr create)");
        }
        Some(state) => println!(
            "Push:      {} ahead, {} behind upstream",
            state.ahead, state.behind
        ),
        None => {}
    }
    println!("Changed:   {} file(s)", summary.changed_files);
    println!(
        "Run:       {}",
        summary
            .run_status
            .map(|s| s.as_str())
            .unwrap_or("not started")
    );
    println!(
        "Session:   {} ({} active)",
        summary
            .session_status
            .map(|s| s.as_str())
            .unwrap_or("not started"),
        summary.active_sessions
    );
    match summary.pull_request {
        Some(pr) => println!("PR:        #{} {} ({})", pr.number, pr.url, pr.state),
        None => println!("PR:        none"),
    }
    println!(
        "Todos:     {} open / {} total",
        summary.open_todos, summary.total_todos
    );
    println!(
        "Review:    {} open comment(s)",
        summary.open_review_comments
    );
    if !summary.conflicting_workspaces.is_empty() {
        println!("Conflicts:");
        for (other, files) in &summary.conflicting_workspaces {
            println!("  {other}: {}", files.join(", "));
        }
    }
}

fn print_mcp_status(status: linux_conductor_core::mcp::McpStatus) {
    println!("MCP status for {}", status.workspace_path.display());
    let groups = [
        ("Claude user (~/.claude.json)", &status.claude_user),
        ("Claude project (.mcp.json)", &status.claude_project),
        ("Codex user (~/.codex/config.toml)", &status.codex_user),
        ("Codex project (.codex/config.toml)", &status.codex_project),
        ("Cursor user (~/.cursor/mcp.json)", &status.cursor_user),
        ("Cursor project (.cursor/mcp.json)", &status.cursor_project),
    ];
    for (label, servers) in groups {
        if servers.is_empty() {
            println!("  {label}: none");
        } else {
            let names: Vec<_> = servers.iter().map(|s| s.name.as_str()).collect();
            println!("  {label}: {}", names.join(", "));
        }
    }
}

fn print_status(lines: Vec<WorkspaceStatusLine>) {
    if lines.is_empty() {
        println!("No workspaces found. Run: linux-conductor workspace create <repo> --name <name> --branch <branch>");
        return;
    }
    for line in lines {
        let ws = &line.workspace;
        let pr = line
            .pull_request
            .as_ref()
            .map(|pr| format!("PR #{} ({})", pr.number, pr.state))
            .unwrap_or_else(|| "no PR".to_owned());
        let push = match &line.branch_push_state {
            Some(state) if !state.has_upstream => "no upstream".to_owned(),
            Some(state) => format!("↑{} ↓{}", state.ahead, state.behind),
            None => String::new(),
        };
        let run = if line.run_running {
            "running"
        } else {
            "stopped"
        };
        let sessions = match line.active_sessions {
            0 => "no session".to_owned(),
            n => format!("{n} session(s)"),
        };
        println!(
            "{:<16} {:<10} {:<28} {:<14} {:<10} {:<12} {} todo(s)  {}",
            ws.name, ws.status, ws.branch, push, run, sessions, line.open_todos, pr,
        );
    }
}

impl From<CliSessionKind> for SessionKind {
    fn from(value: CliSessionKind) -> Self {
        match value {
            CliSessionKind::Shell => Self::Shell,
            CliSessionKind::Codex => Self::Codex,
            CliSessionKind::Claude => Self::Claude,
        }
    }
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
