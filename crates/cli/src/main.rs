use anyhow::{Context, Result};
use archductor_core::archcar::client::ArchcarClient;
use archductor_core::archcar::harness_contract::ProviderInteractionResolution;
use archductor_core::archcar::protocol::{
    ArchcarInputDelivery, ArchcarInputKind, ArchcarMessage, ArchcarRequest, ArchcarResponse,
};
use archductor_core::archcar::server::{reconcile_managed_sessions_on_startup, ArchcarServer};
use archductor_core::doctor;
use archductor_core::import::{default_conductor_app_database, import_conductor_app_database};
use archductor_core::paths::AppPaths;
use archductor_core::provider_adapters::claude_hooks::handle_claude_hook_json;
use archductor_core::provider_interactions::ProviderInteractionRecord;
use archductor_core::repository::{AddRepository, RepositoryStore};
use archductor_core::settings::{
    app_shared_settings_to_toml, save_app_shared_settings_from_toml,
    save_repository_settings_from_toml, SettingsLayer,
};
use archductor_core::workspace::{
    CreateWorkspace, LinkedDirectory, LocalChatHistoryMessage, LocalChatHistorySummary,
    ProcessRecord, ProcessStatus, SessionHarnessOptions, SessionKind, SessionLaunch,
    WorkspaceStatusLine, WorkspaceStore, WorkspaceTimelineEvent,
};
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashSet;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(name = "archductor")]
#[command(about = "Archductor Git worktree workflow for parallel coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Gtk {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Settings {
        #[command(subcommand)]
        command: AppSettingsCommand,
    },
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
    Import {
        #[command(subcommand)]
        command: ImportCommand,
    },
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
    Archcar {
        #[command(subcommand)]
        command: ArchcarCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AppSettingsCommand {
    Export {
        #[arg(long)]
        output: PathBuf,
    },
    Import {
        input: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ImportCommand {
    Conductor {
        #[arg(long)]
        source: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum HistoryCommand {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Show {
        process_id: i64,
    },
}

#[derive(Debug, Subcommand)]
enum ArchcarCommand {
    Ensure {
        workspace: String,
        #[arg(long, value_enum, default_value_t = CliSessionKind::Codex)]
        kind: CliSessionKind,
    },
    Spawn {
        workspace: String,
        #[arg(long, value_enum, default_value_t = CliSessionKind::Shell)]
        kind: CliSessionKind,
    },
    Status {
        session_id: i64,
    },
    Screen {
        session_id: i64,
    },
    Messages {
        thread_id: i64,
    },
    Interactions {
        #[command(subcommand)]
        command: ArchcarInteractionsCommand,
    },
    Send {
        session_id: i64,
        #[arg(long, value_enum, default_value_t = CliArchcarInputKind::User)]
        kind: CliArchcarInputKind,
        #[arg(long)]
        visible_input: Option<String>,
        #[arg(
            long,
            help = "Deliver now: steer an active agent turn or start a new turn"
        )]
        immediate: bool,
        input: Vec<String>,
    },
    Model {
        session_id: i64,
        model: String,
    },
    Effort {
        session_id: i64,
        level: String,
    },
    PermissionMode {
        session_id: i64,
        mode: String,
    },
    Interrupt {
        session_id: i64,
    },
    Resize {
        session_id: i64,
        rows: u16,
        cols: u16,
    },
    Kill {
        session_id: i64,
    },
}

#[derive(Debug, Subcommand)]
enum ArchcarInteractionsCommand {
    List {
        #[arg(long)]
        thread_id: Option<i64>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        detail: bool,
    },
    Show {
        interaction_id: String,
    },
    Allow {
        interaction_id: String,
        #[arg(long)]
        always: bool,
    },
    Deny {
        interaction_id: String,
        #[arg(long)]
        message: Option<String>,
    },
    Answer {
        interaction_id: String,
        #[arg(long)]
        answers_json: String,
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
    Settings {
        name: String,
        #[command(subcommand)]
        command: RepoSettingsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RepoSettingsCommand {
    Export {
        #[arg(long)]
        local: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Import {
        input: PathBuf,
        #[arg(long)]
        local: bool,
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
        from_pr: Option<u64>,
        #[arg(long)]
        from_linear: Option<String>,
        #[arg(long)]
        prompt: Option<String>,
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
    Delete {
        name: String,
        #[arg(long)]
        remove_worktree: bool,
        #[arg(long)]
        delete_branch: bool,
    },
    Rename {
        name: String,
        new_name: String,
    },
    Duplicate {
        name: String,
        new_name: String,
        #[arg(long)]
        branch: Option<String>,
    },
    LinkDir {
        workspace: String,
        target: String,
    },
    UnlinkDir {
        workspace: String,
        target: String,
    },
    LinkedDirs {
        workspace: String,
    },
    Branch {
        workspace: String,
        #[command(subcommand)]
        command: WorkspaceBranchCommand,
    },
    Timeline {
        workspace: String,
        #[arg(long)]
        kind: Option<String>,
    },
    SourcePreflight,
}

#[derive(Debug, Subcommand)]
enum WorkspaceBranchCommand {
    Create { branch: String },
    Checkout { branch: String },
    Rename { branch: String },
    Delete { branch: String },
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    Start {
        workspace: String,
        #[arg(long, value_enum, default_value_t = CliSessionKind::Shell)]
        kind: CliSessionKind,
        #[arg(long)]
        plan_mode: bool,
        #[arg(long)]
        fast_mode: bool,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        approval_mode: Option<String>,
        #[arg(long)]
        reasoning_mode: Option<String>,
        #[arg(long)]
        effort_mode: Option<String>,
        #[arg(long)]
        codex_personality: Option<String>,
        #[arg(long)]
        codex_goals: Option<String>,
        #[arg(long)]
        codex_skills: Option<String>,
    },
    Open {
        workspace: String,
        #[arg(long, value_enum, default_value_t = CliSessionKind::Shell)]
        kind: CliSessionKind,
        #[arg(long)]
        terminal: Option<String>,
        #[arg(long)]
        print_command: bool,
        #[arg(long)]
        plan_mode: bool,
        #[arg(long)]
        fast_mode: bool,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        approval_mode: Option<String>,
        #[arg(long)]
        reasoning_mode: Option<String>,
        #[arg(long)]
        effort_mode: Option<String>,
        #[arg(long)]
        codex_personality: Option<String>,
        #[arg(long)]
        codex_goals: Option<String>,
        #[arg(long)]
        codex_skills: Option<String>,
    },
    Stop {
        workspace: String,
    },
    Attach {
        workspace: String,
        #[arg(long)]
        process_id: Option<i64>,
        #[arg(long)]
        print_pty_path: bool,
    },
    Send {
        workspace: String,
        #[arg(long, value_enum, default_value_t = CliSessionKind::Codex)]
        kind: CliSessionKind,
        #[arg(long)]
        thread_id: Option<i64>,
        #[arg(long, value_enum, default_value_t = CliArchcarInputKind::User)]
        input_kind: CliArchcarInputKind,
        #[arg(long)]
        visible_input: Option<String>,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(
            long,
            help = "Deliver now: steer an active agent turn or start a new turn"
        )]
        immediate: bool,
        message: Vec<String>,
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
    Summary {
        workspace: String,
        #[arg(long)]
        agent_prompt: bool,
    },
    ResolveThread {
        workspace: String,
        thread_id: String,
    },
    ReopenThread {
        workspace: String,
        thread_id: String,
    },
    View {
        workspace: String,
    },
    Merge {
        workspace: String,
        #[arg(long)]
        method: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliSessionKind {
    Shell,
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliArchcarInputKind {
    User,
    ReviewPrompt,
    ControlCommand,
}

fn main() -> Result<()> {
    if handle_archcar_claude_hook()? {
        return Ok(());
    }
    if should_run_archcar_server_mode(std::env::args()) {
        let paths = AppPaths::from_env();
        reconcile_managed_sessions_on_startup(&paths)?;
        return ArchcarServer::bind(paths)?.serve();
    }
    let cli = Cli::parse();
    let paths = AppPaths::from_env();

    match cli.command {
        Command::Doctor => print_doctor(doctor::report_from_host()),
        Command::Gtk { args } => launch_gtk(&args)?,
        Command::Settings { command } => match command {
            AppSettingsCommand::Export { output } => {
                let contents = app_shared_settings_to_toml(&paths.shared_settings_path())?;
                fs::write(&output, contents)
                    .with_context(|| format!("write {}", output.display()))?;
                println!("Exported Shared settings to {}", output.display());
            }
            AppSettingsCommand::Import { input } => {
                let contents = fs::read_to_string(&input)
                    .with_context(|| format!("read {}", input.display()))?;
                save_app_shared_settings_from_toml(&paths.shared_settings_path(), &contents)?;
                let refreshed = refresh_all_repository_prompt_snapshots(&paths)?;
                println!(
                    "Imported Shared settings from {} and refreshed {refreshed} prompt snapshot(s)",
                    input.display()
                );
            }
        },
        Command::Import { command } => match command {
            ImportCommand::Conductor { source } => {
                let source = source.unwrap_or_else(default_conductor_app_database);
                let summary = import_conductor_app_database(&source, &paths.database_path)?;
                println!(
                    "Imported {} repositories and {} workspaces from {}",
                    summary.repositories_imported,
                    summary.workspaces_imported,
                    source.display()
                );
                if summary.renamed_duplicate_workspaces > 0 {
                    println!(
                        "Renamed {} duplicate workspace(s) with repository prefixes for CLI safety.",
                        summary.renamed_duplicate_workspaces
                    );
                }
                if summary.skipped_workspaces > 0 {
                    println!(
                        "Skipped {} workspace(s) with missing repository or name data.",
                        summary.skipped_workspaces
                    );
                }
            }
        },
        Command::History { command } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                HistoryCommand::List { workspace } => {
                    let workspace_path = workspace
                        .as_deref()
                        .map(|name| store.workspace_path(name))
                        .transpose()?;
                    let sessions = store.list_local_chat_history(workspace_path.as_deref())?;
                    print!("{}", render_history_list(&sessions));
                }
                HistoryCommand::Show { process_id } => {
                    let messages = store.local_chat_history_messages(process_id)?;
                    print!("{}", render_history_messages(&messages));
                }
            }
        }
        Command::Archcar { command } => {
            let client = ArchcarClient::from_paths(&paths);
            match command {
                ArchcarCommand::Ensure { workspace, kind } => {
                    print_archcar_response(client.send(
                        ArchcarRequest::EnsureWorkspaceDefaultSession {
                            workspace,
                            kind: kind.into(),
                            harness: None,
                        },
                    )?);
                }
                ArchcarCommand::Spawn { workspace, kind } => {
                    print_archcar_response(client.send(ArchcarRequest::SpawnSession {
                        workspace,
                        kind: kind.into(),
                        harness: None,
                    })?);
                }
                ArchcarCommand::Status { session_id } => {
                    print_archcar_response(
                        client.send(ArchcarRequest::GetSessionStatus { session_id })?,
                    );
                }
                ArchcarCommand::Screen { session_id } => {
                    print_archcar_response(
                        client.send(ArchcarRequest::GetSessionScreen { session_id })?,
                    );
                }
                ArchcarCommand::Messages { thread_id } => {
                    match client.send(ArchcarRequest::GetSessionMessages { thread_id })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    }
                }
                ArchcarCommand::Interactions { command } => match command {
                    ArchcarInteractionsCommand::List {
                        thread_id,
                        all,
                        detail,
                    } => match client.send(ArchcarRequest::ListProviderInteractions {
                        thread_id,
                        pending_only: !all,
                    })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        ArchcarResponse::ProviderInteractions { interactions } => {
                            print!("{}", render_provider_interactions(&interactions, detail));
                        }
                        response => print_archcar_response(response),
                    },
                    ArchcarInteractionsCommand::Show { interaction_id } => {
                        match client
                            .send(ArchcarRequest::GetProviderInteraction { interaction_id })?
                        {
                            ArchcarResponse::Error { message } => anyhow::bail!(message),
                            ArchcarResponse::ProviderInteraction { interaction } => {
                                print!("{}", render_provider_interaction_detail(&interaction));
                            }
                            response => print_archcar_response(response),
                        }
                    }
                    ArchcarInteractionsCommand::Allow {
                        interaction_id,
                        always,
                    } => match client.send(ArchcarRequest::ResolveProviderInteraction {
                        interaction_id,
                        resolution: archcar_allow_resolution(always)?,
                    })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    },
                    ArchcarInteractionsCommand::Deny {
                        interaction_id,
                        message,
                    } => match client.send(ArchcarRequest::ResolveProviderInteraction {
                        interaction_id,
                        resolution: ProviderInteractionResolution::Deny { reason: message },
                    })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    },
                    ArchcarInteractionsCommand::Answer {
                        interaction_id,
                        answers_json,
                    } => {
                        let answers = parse_answers_json(&answers_json)?;
                        match client.send(ArchcarRequest::ResolveProviderInteraction {
                            interaction_id,
                            resolution: ProviderInteractionResolution::Answer { answers },
                        })? {
                            ArchcarResponse::Error { message } => anyhow::bail!(message),
                            response => print_archcar_response(response),
                        }
                    }
                },
                ArchcarCommand::Send {
                    session_id,
                    kind,
                    visible_input,
                    immediate,
                    input,
                } => {
                    print_archcar_response(client.send(ArchcarRequest::SendInput {
                        session_id,
                        input: input.join(" "),
                        visible_input,
                        kind: kind.into(),
                        delivery: cli_input_delivery(immediate),
                    })?);
                }
                ArchcarCommand::Model { session_id, model } => {
                    match client.send(ArchcarRequest::SetSessionModel {
                        session_id,
                        model: Some(model),
                    })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    }
                }
                ArchcarCommand::Effort { session_id, level } => {
                    match client.send(ArchcarRequest::SetSessionEffort {
                        session_id,
                        effort: Some(level),
                    })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    }
                }
                ArchcarCommand::PermissionMode { session_id, mode } => {
                    match client
                        .send(ArchcarRequest::SetSessionPermissionMode { session_id, mode })?
                    {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    }
                }
                ArchcarCommand::Interrupt { session_id } => {
                    match client.send(ArchcarRequest::InterruptTurn { session_id })? {
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        response => print_archcar_response(response),
                    }
                }
                ArchcarCommand::Resize {
                    session_id,
                    rows,
                    cols,
                } => {
                    print_archcar_response(client.send(ArchcarRequest::ResizeSession {
                        session_id,
                        rows,
                        cols,
                    })?);
                }
                ArchcarCommand::Kill { session_id } => {
                    print_archcar_response(
                        client.send(ArchcarRequest::KillSession { session_id })?,
                    );
                }
            }
        }
        Command::Repo { command } => {
            let store = RepositoryStore::open(&paths.database_path)?;
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
                RepoCommand::Settings { name, command } => {
                    let repo = store.get_by_name(&name)?;
                    match command {
                        RepoSettingsCommand::Export { local, output } => {
                            let layer = repo_settings_layer(local);
                            let path = repo_settings_path(&repo.root_path, layer);
                            let contents = fs::read_to_string(&path)
                                .with_context(|| format!("read {}", path.display()))?;
                            if let Some(output) = output {
                                fs::write(&output, contents)
                                    .with_context(|| format!("write {}", output.display()))?;
                                println!(
                                    "Exported {} settings for {} to {}",
                                    repo_settings_layer_label(layer),
                                    repo.name,
                                    output.display()
                                );
                            } else {
                                print!("{contents}");
                            }
                        }
                        RepoSettingsCommand::Import { input, local } => {
                            let contents = fs::read_to_string(&input)
                                .with_context(|| format!("read {}", input.display()))?;
                            let layer = repo_settings_layer(local);
                            save_repository_settings_from_toml(&repo.root_path, layer, &contents)?;
                            let refreshed = WorkspaceStore::open_app_with_logs(
                                &paths.database_path,
                                &paths.logs_dir,
                            )?
                            .refresh_repository_prompt_snapshots(repo.id)?;
                            println!(
                                "Imported {} settings for {} from {} and refreshed {refreshed} prompt snapshot(s)",
                                repo_settings_layer_label(layer),
                                repo.name,
                                input.display()
                            );
                        }
                    }
                }
            }
        }
        Command::Workspace { command } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            store.recover_workspace_lifecycle_jobs()?;
            match command {
                WorkspaceCommand::Create {
                    repository,
                    name,
                    branch,
                    base,
                    from_issue,
                    from_pr,
                    from_linear,
                    prompt,
                    branch_prefix,
                } => {
                    let selected_sources = [
                        from_issue.is_some(),
                        from_pr.is_some(),
                        from_linear.is_some(),
                        prompt.is_some(),
                    ]
                    .into_iter()
                    .filter(|selected| *selected)
                    .count();
                    anyhow::ensure!(
                        selected_sources <= 1,
                        "choose only one source: --from-issue, --from-pr, --from-linear, or --prompt"
                    );
                    let workspace = if let Some(issue) = from_issue {
                        store.create_from_issue(&repository, issue, branch_prefix.as_deref())?
                    } else if let Some(pr) = from_pr {
                        store.create_from_pull_request(
                            &repository,
                            pr,
                            name.as_deref(),
                            branch.as_deref(),
                        )?
                    } else if let Some(linear) = from_linear {
                        store.create_from_linear_issue(
                            &repository,
                            &linear,
                            name.as_deref(),
                            branch.as_deref(),
                            base.as_deref(),
                        )?
                    } else if let Some(prompt) = prompt {
                        store.create_from_prompt(
                            &repository,
                            &prompt,
                            name.as_deref(),
                            branch.as_deref(),
                            base.as_deref(),
                        )?
                    } else {
                        let name = name
                            .with_context(|| "--name is required when not using a source option")?;
                        let branch = branch.with_context(|| {
                            "--branch is required when not using a source option"
                        })?;
                        store.create_lifecycle_job(CreateWorkspace {
                            repository_name: repository,
                            name,
                            branch,
                            base_ref: base,
                        })?
                    };
                    println!(
                        "Created {} at {} (branch: {}, base: {})",
                        workspace.name,
                        workspace.path.display(),
                        workspace.branch,
                        workspace.base_ref
                    );
                }
                WorkspaceCommand::List { active } => {
                    for workspace in store.list()? {
                        if active && workspace.status != "active" {
                            continue;
                        }
                        println!(
                            "{}\t{}\t{}\t{}\t{}",
                            workspace.name,
                            workspace.path.display(),
                            workspace.branch,
                            workspace.base_ref,
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
                WorkspaceCommand::Delete {
                    name,
                    remove_worktree,
                    delete_branch,
                } => {
                    let result =
                        store.delete_lifecycle_job(&name, remove_worktree, delete_branch)?;
                    println!("Deleted workspace {}", result.workspace.name);
                    if remove_worktree || delete_branch {
                        if let Some(err) = result.cleanup_error {
                            eprintln!(
                                "Artifact cleanup failed after metadata delete for {}: {err}",
                                result.workspace.name
                            );
                            anyhow::bail!(
                                "workspace metadata deleted but artifact cleanup failed: {err}"
                            );
                        }
                        println!("Cleaned workspace artifacts for {}", result.workspace.name);
                    }
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
                WorkspaceCommand::Duplicate {
                    name,
                    new_name,
                    branch,
                } => {
                    let workspace = store.duplicate(&name, &new_name, branch.as_deref())?;
                    println!(
                        "Duplicated {} to {} at {} (branch: {})",
                        name,
                        workspace.name,
                        workspace.path.display(),
                        workspace.branch
                    );
                }
                WorkspaceCommand::LinkDir { workspace, target } => {
                    let link = store.link_workspace_directory(&workspace, &target)?;
                    println!(
                        "Linked {} into {} at {}",
                        link.target_workspace_name,
                        link.workspace_name,
                        link.link_path.display()
                    );
                }
                WorkspaceCommand::UnlinkDir { workspace, target } => {
                    let link = store.unlink_workspace_directory(&workspace, &target)?;
                    println!(
                        "Unlinked {} from {}",
                        link.target_workspace_name, link.workspace_name
                    );
                }
                WorkspaceCommand::LinkedDirs { workspace } => {
                    print!(
                        "{}",
                        render_linked_directories(&store.list_linked_directories(&workspace)?)
                    );
                }
                WorkspaceCommand::Branch { workspace, command } => match command {
                    WorkspaceBranchCommand::Create { branch } => {
                        store.create_branch(&workspace, &branch)?;
                        println!("Created branch {branch} for {workspace}");
                    }
                    WorkspaceBranchCommand::Checkout { branch } => {
                        let updated = store.checkout_branch(&workspace, &branch)?;
                        println!("Checked out {} in {}", updated.branch, updated.name);
                    }
                    WorkspaceBranchCommand::Rename { branch } => {
                        let updated = store.rename_branch(&workspace, &branch)?;
                        println!("Renamed workspace branch to {}", updated.branch);
                    }
                    WorkspaceBranchCommand::Delete { branch } => {
                        store.delete_branch(&workspace, &branch)?;
                        println!("Deleted branch {branch} for {workspace}");
                    }
                },
                WorkspaceCommand::Timeline { workspace, kind } => {
                    print!(
                        "{}",
                        render_workspace_timeline(
                            &store.workspace_timeline(&workspace, kind.as_deref())?
                        )
                    );
                }
                WorkspaceCommand::SourcePreflight => {
                    print_source_preflight(store.source_preflight());
                }
            }
        }
        Command::Run { workspace } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            let process = store.run_workspace(&workspace)?;
            println!(
                "Started run for {} as pid {} (log: {})",
                workspace,
                process.pid,
                process.log_path.display()
            );
        }
        Command::Stop { workspace } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
                    "choose exactly one log stream, for example: archductor logs {workspace} --run"
                );
            }
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            if run {
                print!("{}", store.read_latest_run_log(&workspace)?);
            } else {
                print!("{}", store.read_latest_session_log(&workspace)?);
            }
        }
        Command::Runs { workspace } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            if name_only {
                for path in store.changed_files(&workspace)? {
                    println!("{path}");
                }
            } else {
                print!("{}", store.unified_diff(&workspace, file.as_deref())?);
            }
        }
        Command::Pr { command } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
                PrCommand::Summary {
                    workspace,
                    agent_prompt,
                } => {
                    if agent_prompt {
                        print!("{}", store.pull_request_readiness_agent_prompt(&workspace)?);
                    } else {
                        print!("{}", store.pull_request_readiness_text(&workspace)?);
                    }
                }
                PrCommand::ResolveThread {
                    workspace,
                    thread_id,
                } => {
                    let thread = store
                        .set_pull_request_review_thread_resolution(&workspace, &thread_id, true)?;
                    println!(
                        "Resolved review thread {} for {}",
                        thread.id.as_deref().unwrap_or(thread_id.as_str()),
                        workspace
                    );
                }
                PrCommand::ReopenThread {
                    workspace,
                    thread_id,
                } => {
                    let thread = store
                        .set_pull_request_review_thread_resolution(&workspace, &thread_id, false)?;
                    println!(
                        "Reopened review thread {} for {}",
                        thread.id.as_deref().unwrap_or(thread_id.as_str()),
                        workspace
                    );
                }
                PrCommand::View { workspace } => {
                    match store.refresh_pull_request_state(&workspace)? {
                        Some(pr) => println!("#{} {} (state: {})", pr.number, pr.url, pr.state),
                        None => println!("No pull request recorded for {workspace}"),
                    }
                }
                PrCommand::Merge { workspace, method } => {
                    print!(
                        "{}",
                        store.merge_pull_request(&workspace, method.as_deref())?
                    );
                    println!("Merged pull request for {workspace}");
                }
            }
        }
        Command::Session { command } => {
            let store = WorkspaceStore::open_app_with_logs(
                paths.database_path.clone(),
                paths.logs_dir.clone(),
            )?;
            match command {
                SessionCommand::Start {
                    workspace,
                    kind,
                    plan_mode,
                    fast_mode,
                    model,
                    approval_mode,
                    reasoning_mode,
                    effort_mode,
                    codex_personality,
                    codex_goals,
                    codex_skills,
                } => {
                    let harness = SessionHarnessOptions {
                        plan_mode,
                        fast_mode,
                        model,
                        approval_mode,
                        reasoning_mode,
                        effort_mode,
                        codex_personality,
                        codex_goals,
                        codex_skills,
                    };
                    let process = if cli_session_start_uses_archcar(kind) {
                        let existing_ids = running_session_ids(&store, &workspace)?;
                        let client = ArchcarClient::from_paths(&paths);
                        let kind: SessionKind = kind.into();
                        print_archcar_response(client.send(ArchcarRequest::SpawnSession {
                            workspace: workspace.clone(),
                            kind,
                            harness: Some(harness.clone()),
                        })?);
                        wait_for_new_session_process(
                            &store,
                            &workspace,
                            kind,
                            &existing_ids,
                            Duration::from_secs(5),
                        )?
                    } else {
                        store.start_session_with_options(&workspace, kind.into(), harness)?
                    };
                    println!(
                        "Started session for {} as pid {} (log: {})",
                        workspace,
                        process.pid,
                        process.log_path.display()
                    );
                }
                SessionCommand::Open {
                    workspace,
                    kind,
                    terminal,
                    print_command,
                    plan_mode,
                    fast_mode,
                    model,
                    approval_mode,
                    reasoning_mode,
                    effort_mode,
                    codex_personality,
                    codex_goals,
                    codex_skills,
                } => {
                    let launch = store.session_launch_with_options(
                        &workspace,
                        kind.into(),
                        SessionHarnessOptions {
                            plan_mode,
                            fast_mode,
                            model,
                            approval_mode,
                            reasoning_mode,
                            effort_mode,
                            codex_personality,
                            codex_goals,
                            codex_skills,
                        },
                    )?;
                    if print_command {
                        println!("{}", render_manual_session_command(&launch));
                    } else {
                        open_interactive_session(&launch, terminal.as_deref())?;
                    }
                }
                SessionCommand::Stop { workspace } => {
                    let sessions = store.list_sessions(&workspace)?;
                    let record = latest_running_session(&sessions).with_context(|| {
                        format!("no running session found for workspace {workspace}")
                    })?;
                    if cli_session_stop_uses_archcar(session_kind_from_process_record(
                        &store, record,
                    )?) {
                        let client = ArchcarClient::from_paths(&paths);
                        let _ = client.send(ArchcarRequest::KillSession {
                            session_id: record.id,
                        });
                    }
                    let process = store.stop_session_process(&workspace, record.id)?;
                    println!("Stopped session for {} (pid {})", workspace, process.pid);
                }
                SessionCommand::Attach {
                    workspace,
                    process_id,
                    print_pty_path,
                } => {
                    let process = resolve_attachable_session(&store, &workspace, process_id)?;
                    let pty_path = terminal_device_path_for_pid(process.pid)?;
                    if print_pty_path {
                        println!("{}", pty_path.display());
                    } else {
                        attach_to_session_pty(&pty_path)?;
                    }
                }
                SessionCommand::Send {
                    workspace,
                    kind,
                    thread_id,
                    input_kind,
                    visible_input,
                    timeout_ms,
                    immediate,
                    message,
                } => {
                    let kind: SessionKind = kind.into();
                    anyhow::ensure!(
                        matches!(kind, SessionKind::Codex | SessionKind::Claude),
                        "session send supports codex and claude"
                    );
                    let input = message_text_or_stdin(message)?;
                    let client = ArchcarClient::from_paths(&paths);
                    let (session_id, resolved_thread_id) = ensure_session_send_target(
                        &client,
                        &store,
                        &workspace,
                        kind,
                        thread_id,
                        Duration::from_millis(timeout_ms),
                    )?;
                    match client.send(ArchcarRequest::SendInput {
                        session_id,
                        input,
                        visible_input,
                        kind: input_kind.into(),
                        delivery: cli_input_delivery(immediate),
                    })? {
                        ArchcarResponse::Ack => {
                            println!(
                                "sent {}{} message to session {} thread {}",
                                session_kind_label(kind),
                                if immediate { " immediate" } else { "" },
                                session_id,
                                resolved_thread_id
                            );
                        }
                        ArchcarResponse::Error { message } => anyhow::bail!(message),
                        other => print_archcar_response(other),
                    }
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            print_checks_summary(store.checks_summary(&workspace)?);
        }
        Command::Open { workspace, editor } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            match command {
                McpCommand::Status { workspace } => {
                    print_mcp_status(store.mcp_status(&workspace)?);
                }
            }
        }
        Command::Review { command } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            let workspace = store.archive(&name, remove_worktree)?;
            println!(
                "Archived {} at {}",
                workspace.name,
                workspace.path.display()
            );
        }
        Command::Status => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            print_status(store.list_status()?);
        }
        Command::Checkpoint { command } => {
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
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
            let store = WorkspaceStore::open_app_with_logs(paths.database_path, paths.logs_dir)?;
            let workspace = store.discard(&name)?;
            println!(
                "Discarded {} — worktree removed and branch deleted",
                workspace.name
            );
        }
    }

    Ok(())
}

fn should_run_archcar_server_mode<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    matches!(
        args.next().as_ref().map(|arg| arg.as_ref()),
        Some("--archcar-serve")
    )
}

fn handle_archcar_claude_hook() -> Result<bool> {
    let args = std::env::args().collect::<Vec<_>>();
    let Some(index) = args.iter().position(|arg| arg == "--archcar-claude-hook") else {
        return Ok(false);
    };
    let thread_id = args
        .get(index + 1)
        .context("--archcar-claude-hook requires a thread id")?
        .parse::<i64>()
        .context("parse Claude hook thread id")?;
    let mut stdin = String::new();
    io::stdin()
        .read_to_string(&mut stdin)
        .context("read Claude hook stdin")?;
    let output = handle_claude_hook_json(thread_id, &stdin);
    println!("{output}");
    Ok(true)
}

fn refresh_all_repository_prompt_snapshots(paths: &AppPaths) -> Result<usize> {
    let repositories = RepositoryStore::open(&paths.database_path)?.list()?;
    let store = WorkspaceStore::open_app_with_logs(&paths.database_path, &paths.logs_dir)?;
    repositories.into_iter().try_fold(0, |total, repository| {
        Ok(total + store.refresh_repository_prompt_snapshots(repository.id)?)
    })
}

fn print_archcar_response(response: ArchcarResponse) {
    match response {
        ArchcarResponse::Ack => println!("ok"),
        ArchcarResponse::SessionSpawnQueued { workspace, kind } => {
            println!("queued {:?} session for {}", kind, workspace);
        }
        ArchcarResponse::SessionSpawned {
            session_id,
            thread_id,
            workspace,
            kind,
            pid,
        } => {
            println!(
                "spawned {:?} session {} thread {} for {} pid {}",
                kind, session_id, thread_id, workspace, pid
            );
        }
        ArchcarResponse::SessionStatus {
            session_id,
            status,
            runtime_state,
            ready,
            capabilities,
        } => {
            println!(
                "session {} status={} state={} ready={}",
                session_id,
                status,
                runtime_state.as_str(),
                ready
            );
            if let Some(capabilities) = capabilities {
                println!(
                    "capabilities contract={} required={} optional={} observed_native={}",
                    capabilities.contract_version,
                    capabilities.required.len(),
                    capabilities.optional.len(),
                    capabilities.observed_native.len()
                );
            }
        }
        ArchcarResponse::SessionScreen { screen, .. } => print!("{screen}"),
        ArchcarResponse::SessionMessages { messages, .. } => {
            print!("{}", render_archcar_protocol_messages(&messages));
        }
        ArchcarResponse::ProviderInteraction { interaction } => {
            print!("{}", render_provider_interactions(&[interaction], false));
        }
        ArchcarResponse::ProviderInteractions { interactions } => {
            print!("{}", render_provider_interactions(&interactions, false));
        }
        ArchcarResponse::Error { message } => {
            eprintln!("{message}");
        }
    }
}

fn render_provider_interactions(
    interactions: &[ProviderInteractionRecord],
    detail: bool,
) -> String {
    let mut output = String::new();
    if interactions.is_empty() {
        output.push_str("provider interactions 0\n");
        return output;
    }
    for interaction in interactions {
        output.push_str(&render_provider_interaction_line(interaction));
        output.push('\n');
        if detail {
            output.push_str(&render_provider_interaction_detail(interaction));
        }
    }
    output
}

fn render_provider_interaction_line(interaction: &ProviderInteractionRecord) -> String {
    let summary = interaction_summary(interaction);
    format!(
        "{} provider={} kind={} status={} thread={} {}",
        interaction.id,
        interaction.provider_key,
        provider_interaction_kind_label(interaction),
        provider_interaction_status_label(interaction),
        interaction.thread_id,
        summary
    )
}

fn render_provider_interaction_detail(interaction: &ProviderInteractionRecord) -> String {
    format!(
        "{}\nrequest={}\n",
        render_provider_interaction_line(interaction),
        serde_json::to_string_pretty(&interaction.native_request)
            .unwrap_or_else(|_| "{}".to_owned())
    )
}

fn interaction_summary(interaction: &ProviderInteractionRecord) -> String {
    let title = interaction.title.trim();
    let detail = interaction.detail.trim();
    match (title.is_empty(), detail.is_empty()) {
        (true, true) => interaction.native_id.clone(),
        (false, true) => title.to_owned(),
        (true, false) => detail.to_owned(),
        (false, false) => format!("{title}: {detail}"),
    }
}

fn provider_interaction_kind_label(interaction: &ProviderInteractionRecord) -> &'static str {
    match interaction.kind {
        archductor_core::archcar::harness_contract::ProviderInteractionKind::Permission => {
            "permission"
        }
        archductor_core::archcar::harness_contract::ProviderInteractionKind::UserQuestion => {
            "question"
        }
        archductor_core::archcar::harness_contract::ProviderInteractionKind::PlanApproval => "plan",
    }
}

fn provider_interaction_status_label(interaction: &ProviderInteractionRecord) -> &'static str {
    match interaction.status {
        archductor_core::provider_interactions::ProviderInteractionStatus::Pending => "pending",
        archductor_core::provider_interactions::ProviderInteractionStatus::Allowed => "allowed",
        archductor_core::provider_interactions::ProviderInteractionStatus::Denied => "denied",
        archductor_core::provider_interactions::ProviderInteractionStatus::Answered => "answered",
        archductor_core::provider_interactions::ProviderInteractionStatus::Expired => "expired",
        archductor_core::provider_interactions::ProviderInteractionStatus::Failed => "failed",
    }
}

fn parse_answers_json(value: &str) -> Result<Vec<(String, String)>> {
    let json = serde_json::from_str::<serde_json::Value>(value).context("parse --answers-json")?;
    let object = json
        .as_object()
        .context("--answers-json must be a JSON object")?;
    Ok(object
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            )
        })
        .collect())
}

fn archcar_allow_resolution(always: bool) -> Result<ProviderInteractionResolution> {
    if always {
        anyhow::bail!(
            "--always is not supported yet; run allow without --always for a one-time approval"
        );
    }
    Ok(ProviderInteractionResolution::Approve)
}

fn cli_session_start_uses_archcar(kind: CliSessionKind) -> bool {
    matches!(kind, CliSessionKind::Codex | CliSessionKind::Claude)
}

fn cli_session_stop_uses_archcar(kind: SessionKind) -> bool {
    matches!(kind, SessionKind::Codex | SessionKind::Claude)
}

fn render_archcar_protocol_messages(messages: &[ArchcarMessage]) -> String {
    let mut out = String::new();
    for message in messages {
        let label = archcar_role_label(&message.role);
        let content = if message.source == "provider_event"
            && !matches!(message.role.as_str(), "user" | "agent" | "assistant")
        {
            archcar_message_content_without_duplicate_title(&label, &message.content)
        } else {
            &message.content
        };
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        out.push_str(&format!("{label}\n{content}\n\n"));
    }
    out
}

fn archcar_role_label(role: &str) -> String {
    match role {
        "user" => "You".to_owned(),
        "agent" | "assistant" => "Assistant".to_owned(),
        other => sentence_case_label(other),
    }
}

fn archcar_message_content_without_duplicate_title<'a>(label: &str, content: &'a str) -> &'a str {
    content
        .strip_prefix(label)
        .and_then(|rest| rest.strip_prefix('\n'))
        .unwrap_or(content)
}

fn sentence_case_label(value: &str) -> String {
    let label = value.replace(['_', '-'], " ");
    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_uppercase(), chars.as_str())
}

fn print_checks_summary(summary: archductor_core::workspace::ChecksSummary) {
    println!(
        "Workspace: {} ({})",
        summary.workspace.name, summary.workspace.status
    );
    println!("Branch:    {}", summary.workspace.branch);
    match &summary.branch_push_state {
        Some(state) if !state.has_upstream => {
            println!("Push:      no upstream set (push with: archductor pr create)");
        }
        Some(state) => println!(
            "Push:      {} ahead, {} behind upstream",
            state.ahead, state.behind
        ),
        None => {}
    }
    if summary.source_branch_ahead > 0 {
        println!(
            "Source:    {} commit(s) ahead; merge before creating PR",
            summary.source_branch_ahead
        );
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

fn print_source_preflight(preflight: archductor_core::workspace::WorkspaceSourcePreflight) {
    println!("Workspace source preflight");
    println!("GitHub: {}", preflight.github_status());
    println!("Linear: {}", preflight.linear_status());
}

fn render_linked_directories(links: &[LinkedDirectory]) -> String {
    if links.is_empty() {
        return "No linked directories.\n".to_owned();
    }
    let mut out = String::new();
    for link in links {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            link.target_workspace_name,
            link.target_workspace_path.display(),
            link.link_path.display()
        ));
    }
    out
}

fn render_workspace_timeline(events: &[WorkspaceTimelineEvent]) -> String {
    let mut out = String::new();
    for event in events {
        out.push_str(&format!(
            "#{}\t{}\t{}\t{}\n",
            event.id, event.created_at, event.kind, event.summary
        ));
    }
    out
}

fn render_history_list(sessions: &[LocalChatHistorySummary]) -> String {
    if sessions.is_empty() {
        return "No local chat history found.\n".to_owned();
    }
    let mut out = String::new();
    for session in sessions {
        out.push_str(&format!(
            "#{}\t{}\t{}\t{}\t{}\t{} message(s)\t{}\n",
            session.process_id,
            session.status,
            session.updated_at,
            session.repository_name,
            session.workspace_name,
            session.message_count,
            session.preview.replace('\n', " ")
        ));
    }
    out
}

fn render_history_messages(messages: &[LocalChatHistoryMessage]) -> String {
    if messages.is_empty() {
        return "No messages in this chat.\n".to_owned();
    }
    let mut out = String::new();
    for message in messages {
        out.push_str(history_role_label(&message.role));
        out.push('\n');
        out.push_str(&message.content);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn history_role_label(role: &str) -> &'static str {
    match role {
        "user" => "You",
        "review" => "Review Prompt",
        "system" => "System",
        _ => "Agent",
    }
}

fn repo_settings_layer(local: bool) -> SettingsLayer {
    if local {
        SettingsLayer::LocalOverride
    } else {
        SettingsLayer::RepositoryShared
    }
}

fn repo_settings_layer_label(layer: SettingsLayer) -> &'static str {
    match layer {
        SettingsLayer::RepositoryShared => "shared",
        SettingsLayer::LocalOverride => "local",
    }
}

fn repo_settings_path(repo_path: &Path, layer: SettingsLayer) -> PathBuf {
    match layer {
        SettingsLayer::RepositoryShared => repo_path.join(".archductor/settings.toml"),
        SettingsLayer::LocalOverride => repo_path.join(".archductor/settings.local.toml"),
    }
}

fn print_mcp_status(status: archductor_core::mcp::McpStatus) {
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
        println!("No workspaces found. Run: archductor workspace create <repo> --name <name> --branch <branch>");
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

impl From<CliArchcarInputKind> for ArchcarInputKind {
    fn from(value: CliArchcarInputKind) -> Self {
        match value {
            CliArchcarInputKind::User => Self::User,
            CliArchcarInputKind::ReviewPrompt => Self::ReviewPrompt,
            CliArchcarInputKind::ControlCommand => Self::ControlCommand,
        }
    }
}

fn cli_input_delivery(immediate: bool) -> ArchcarInputDelivery {
    if immediate {
        ArchcarInputDelivery::Immediate
    } else {
        ArchcarInputDelivery::Auto
    }
}

fn launch_gtk(args: &[String]) -> Result<()> {
    let binary = gtk_binary_path();
    let status = ProcessCommand::new(&binary)
        .args(args)
        .status()
        .with_context(|| format!("launch GTK app {}", binary.display()))?;
    anyhow::ensure!(status.success(), "GTK app exited with status {status}");
    Ok(())
}

fn gtk_binary_path() -> PathBuf {
    if let Some(path) = std::env::var_os("ARCHDUCTOR_GTK_BIN") {
        return PathBuf::from(path);
    }
    gtk_binary_path_for_cli_exe(std::env::current_exe().ok())
}

fn gtk_binary_path_for_cli_exe(cli_exe: Option<PathBuf>) -> PathBuf {
    let binary_name = format!("archductor-gtk{}", std::env::consts::EXE_SUFFIX);
    if let Some(cli_exe) = cli_exe {
        if let Some(parent) = cli_exe.parent() {
            let sibling = parent.join(&binary_name);
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from(binary_name)
}

fn open_interactive_session(launch: &SessionLaunch, terminal: Option<&str>) -> Result<()> {
    let terminal = terminal
        .map(str::to_owned)
        .or_else(detect_terminal)
        .with_context(|| {
            format!(
                "no supported terminal emulator found; run manually:\n{}",
                render_manual_session_command(launch)
            )
        })?;
    let invocation = build_terminal_invocation(&terminal, &render_manual_session_command(launch))
        .with_context(|| format!("unsupported terminal emulator: {terminal}"))?;
    ProcessCommand::new(&invocation.program)
        .args(&invocation.args)
        .current_dir(&launch.cwd)
        .envs(launch.env.iter().map(|(key, value)| (key, value)))
        .spawn()
        .with_context(|| format!("open interactive session in {}", launch.cwd.display()))?;
    println!(
        "Opened {} session in {}",
        session_kind_label(launch.kind),
        launch.cwd.display()
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct TerminalInvocation {
    program: String,
    args: Vec<String>,
}

fn build_terminal_invocation(terminal: &str, command: &str) -> Option<TerminalInvocation> {
    let terminal_key = terminal_key(terminal)?;
    let args = match terminal_key.as_str() {
        "wt" | "wt.exe" | "windows-terminal" => vec![
            "new-tab".to_owned(),
            "cmd.exe".to_owned(),
            "/D".to_owned(),
            "/S".to_owned(),
            "/C".to_owned(),
            command.to_owned(),
        ],
        "gnome-terminal" | "kgx" => vec![
            "--".to_owned(),
            "bash".to_owned(),
            "-lc".to_owned(),
            command.to_owned(),
        ],
        "konsole" | "alacritty" | "kitty" | "xterm" => {
            vec![
                "-e".to_owned(),
                "bash".to_owned(),
                "-lc".to_owned(),
                command.to_owned(),
            ]
        }
        "tilix" | "terminator" => {
            vec![
                "-e".to_owned(),
                format!("bash -lc {}", quote_shell_word(command)),
            ]
        }
        "foot" => vec!["bash".to_owned(), "-lc".to_owned(), command.to_owned()],
        "wezterm" => vec![
            "start".to_owned(),
            "--".to_owned(),
            "bash".to_owned(),
            "-lc".to_owned(),
            command.to_owned(),
        ],
        "macos-terminal" | "terminal.app" => {
            return Some(TerminalInvocation {
                program: "osascript".to_owned(),
                args: vec![
                    "-e".to_owned(),
                    format!(
                        "tell application \"Terminal\" to do script \"{}\"",
                        escape_applescript_string(command)
                    ),
                    "-e".to_owned(),
                    "tell application \"Terminal\" to activate".to_owned(),
                ],
            });
        }
        "xfce4-terminal" => {
            vec![
                "--command".to_owned(),
                format!("bash -lc {}", quote_shell_word(command)),
            ]
        }
        _ => return None,
    };
    Some(TerminalInvocation {
        program: terminal.to_owned(),
        args,
    })
}

fn terminal_key(terminal: &str) -> Option<String> {
    let trimmed = terminal.trim();
    if trimmed.is_empty() {
        return None;
    }
    Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .or_else(|| Some(trimmed.to_ascii_lowercase()))
}

fn detect_terminal() -> Option<String> {
    if let Ok(term) = std::env::var("TERMINAL") {
        if !term.trim().is_empty() && command_exists(&term) {
            return Some(term);
        }
    }
    #[cfg(windows)]
    let candidates = ["wt.exe"];
    #[cfg(not(windows))]
    let candidates = [
        "gnome-terminal",
        "kgx",
        "konsole",
        "alacritty",
        "kitty",
        "xterm",
        "tilix",
        "terminator",
        "xfce4-terminal",
    ];
    candidates
        .into_iter()
        .find(|candidate| command_exists(candidate))
        .map(str::to_owned)
        .or_else(|| {
            if cfg!(target_os = "macos") && command_exists("osascript") {
                Some("macos-terminal".to_owned())
            } else {
                None
            }
        })
}

fn command_exists(command: &str) -> bool {
    doctor::command_exists(command)
}

#[cfg(not(windows))]
fn interactive_session_command(launch: &SessionLaunch) -> String {
    format!("exec {}", shell_words(&launch.program, &launch.args))
}

#[cfg(windows)]
fn interactive_session_command(launch: &SessionLaunch) -> String {
    shell_words(&launch.program, &launch.args)
}

fn render_manual_session_command(launch: &SessionLaunch) -> String {
    #[cfg(windows)]
    {
        let env = launch
            .env
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .map(|value| format!("set \"{key}={}\"", escape_cmd_set_value(value)))
            })
            .collect::<Vec<_>>();
        let mut parts = env;
        parts.push(format!(
            "cd /D {}",
            quote_shell_word(&launch.cwd.to_string_lossy())
        ));
        parts.push(interactive_session_command(launch));
        parts.join(" && ")
    }
    #[cfg(not(windows))]
    {
        let mut env_parts = Vec::new();
        for (key, value) in &launch.env {
            if let Some(value) = value.to_str() {
                env_parts.push(format!("{key}={}", quote_shell_word(value)));
            }
        }
        let launch_command = if env_parts.is_empty() {
            interactive_session_command(launch)
        } else {
            format!(
                "{} {}",
                env_parts.join(" "),
                interactive_session_command(launch)
            )
        };
        format!(
            "cd {} && {}",
            quote_shell_word(&launch.cwd.to_string_lossy()),
            launch_command
        )
    }
}

fn session_kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "shell",
        SessionKind::Codex => "codex",
        SessionKind::Claude => "claude",
    }
}

fn command_session_kind_label(command: &str) -> &'static str {
    let executable = command.split_whitespace().next().unwrap_or("").trim();
    match PathBuf::from(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
    {
        "codex" => "codex",
        "claude" => "claude",
        _ => "shell",
    }
}

fn wait_for_session_process(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
    timeout: Duration,
) -> Result<ProcessRecord> {
    wait_for_session_process_matching(store, workspace, kind, None, None, timeout)
}

fn wait_for_new_session_process(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
    existing_ids: &HashSet<i64>,
    timeout: Duration,
) -> Result<ProcessRecord> {
    wait_for_session_process_matching(store, workspace, kind, None, Some(existing_ids), timeout)
}

fn wait_for_thread_session_process(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
    thread_id: i64,
    timeout: Duration,
) -> Result<ProcessRecord> {
    wait_for_session_process_matching(store, workspace, kind, Some(thread_id), None, timeout)
}

fn wait_for_session_process_matching(
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
    thread_id: Option<i64>,
    excluded_ids: Option<&HashSet<i64>>,
    timeout: Duration,
) -> Result<ProcessRecord> {
    let started = Instant::now();
    loop {
        for record in store.list_sessions(workspace)? {
            if record.status == ProcessStatus::Running
                && session_record_matches_kind(store, &record, kind)?
                && thread_id.is_none_or(|thread_id| record.chat_thread_id == Some(thread_id))
                && excluded_ids.is_none_or(|ids| !ids.contains(&record.id))
            {
                return Ok(record);
            }
        }
        if started.elapsed() >= timeout {
            anyhow::bail!(
                "timed out waiting for {:?} session record for workspace {}",
                kind,
                workspace
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn running_session_ids(store: &WorkspaceStore, workspace: &str) -> Result<HashSet<i64>> {
    Ok(store
        .list_sessions(workspace)?
        .into_iter()
        .filter(|record| record.status == ProcessStatus::Running)
        .map(|record| record.id)
        .collect())
}

fn latest_running_session(sessions: &[ProcessRecord]) -> Option<&ProcessRecord> {
    sessions
        .iter()
        .filter(|session| session.status == ProcessStatus::Running)
        .max_by_key(|session| session.id)
}

fn session_record_matches_kind(
    store: &WorkspaceStore,
    record: &ProcessRecord,
    kind: SessionKind,
) -> Result<bool> {
    Ok(session_kind_from_process_record(store, record)? == kind)
}

fn session_kind_from_process_record(
    store: &WorkspaceStore,
    record: &ProcessRecord,
) -> Result<SessionKind> {
    if let Some(thread_id) = record.chat_thread_id {
        let thread = store.get_chat_thread_record(thread_id)?;
        return Ok(match thread.provider.as_str() {
            "codex" => SessionKind::Codex,
            "claude" => SessionKind::Claude,
            _ => SessionKind::Shell,
        });
    }

    Ok(match command_session_kind_label(&record.command) {
        "codex" => SessionKind::Codex,
        "claude" => SessionKind::Claude,
        _ => SessionKind::Shell,
    })
}

fn ensure_session_send_target(
    client: &ArchcarClient,
    store: &WorkspaceStore,
    workspace: &str,
    kind: SessionKind,
    thread_id: Option<i64>,
    timeout: Duration,
) -> Result<(i64, i64)> {
    let deadline = Instant::now() + timeout;
    let response = if let Some(thread_id) = thread_id {
        client.send(ArchcarRequest::EnsureChatThreadSession {
            workspace: workspace.to_owned(),
            thread_id,
            kind,
            harness: None,
        })?
    } else {
        client.send(ArchcarRequest::EnsureWorkspaceDefaultSession {
            workspace: workspace.to_owned(),
            kind,
            harness: None,
        })?
    };

    let target = match response {
        ArchcarResponse::SessionSpawned {
            session_id,
            thread_id,
            ..
        } => (session_id, thread_id),
        ArchcarResponse::SessionSpawnQueued { .. } => {
            let remaining = remaining_duration(deadline)?;
            let process = if let Some(thread_id) = thread_id {
                wait_for_thread_session_process(store, workspace, kind, thread_id, remaining)?
            } else {
                wait_for_session_process(store, workspace, kind, remaining)?
            };
            let thread_id = process
                .chat_thread_id
                .context("queued provider session did not record a chat thread id")?;
            (process.id, thread_id)
        }
        ArchcarResponse::Error { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected archcar response: {:?}", other),
    };
    let thread_has_visible_history = !store.list_chat_messages(target.1)?.is_empty();
    if session_send_waits_for_ready(kind, thread_has_visible_history) {
        wait_for_archcar_session_ready(client, target.0, deadline)?;
    }
    Ok(target)
}

fn session_send_waits_for_ready(kind: SessionKind, thread_has_visible_history: bool) -> bool {
    !matches!(kind, SessionKind::Claude) || thread_has_visible_history
}

fn wait_for_archcar_session_ready(
    client: &ArchcarClient,
    session_id: i64,
    deadline: Instant,
) -> Result<()> {
    loop {
        match client.send(ArchcarRequest::GetSessionStatus { session_id })? {
            ArchcarResponse::SessionStatus { ready: true, .. } => return Ok(()),
            ArchcarResponse::SessionStatus {
                status,
                runtime_state,
                ..
            } if archcar_status_is_terminal(&status, runtime_state) => {
                anyhow::bail!(
                    "session {session_id} exited before becoming ready: status={} state={}",
                    status,
                    runtime_state.as_str()
                );
            }
            ArchcarResponse::SessionStatus { .. } => {}
            ArchcarResponse::Error { message } if message.contains("unknown session") => {
                anyhow::bail!("session {session_id} disappeared before becoming ready: {message}");
            }
            ArchcarResponse::Error { message } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected archcar response: {:?}", other),
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for session {session_id} to become ready");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn remaining_duration(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .context("timed out waiting for provider session")
}

fn archcar_status_is_terminal(
    status: &str,
    runtime_state: archductor_core::session_state::AgentSessionState,
) -> bool {
    !matches!(status, "running")
        || matches!(
            runtime_state,
            archductor_core::session_state::AgentSessionState::Interrupted
                | archductor_core::session_state::AgentSessionState::Failed
                | archductor_core::session_state::AgentSessionState::Exited
                | archductor_core::session_state::AgentSessionState::Archived
        )
}

fn message_text_or_stdin(message: Vec<String>) -> Result<String> {
    if !message.is_empty() {
        let input = message.join(" ");
        anyhow::ensure!(!input.trim().is_empty(), "message is required");
        return Ok(input);
    }
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("read message from stdin")?;
    let input = input.trim_end_matches(['\r', '\n']).to_owned();
    anyhow::ensure!(!input.trim().is_empty(), "message is required");
    Ok(input)
}

fn resolve_attachable_session(
    store: &WorkspaceStore,
    workspace: &str,
    process_id: Option<i64>,
) -> Result<ProcessRecord> {
    let sessions = store.list_sessions(workspace)?;
    let process = if let Some(process_id) = process_id {
        sessions
            .into_iter()
            .find(|session| session.id == process_id)
            .with_context(|| {
                format!("session process {process_id} not found for workspace {workspace}")
            })?
    } else {
        sessions
            .into_iter()
            .find(|session| session.status == ProcessStatus::Running)
            .with_context(|| format!("no running session found for workspace {workspace}"))?
    };
    anyhow::ensure!(
        process.status == ProcessStatus::Running,
        "session #{} for workspace {} is not running",
        process.id,
        workspace
    );
    Ok(process)
}

fn terminal_device_path_for_pid(process_id: u32) -> Result<PathBuf> {
    let fd = format!("/proc/{process_id}/fd/0");
    let target = fs::read_link(&fd)
        .with_context(|| format!("process {process_id} is not attached to a PTY slave"))?;
    anyhow::ensure!(
        target.starts_with("/dev/pts/"),
        "process {process_id} is not attached to a PTY slave"
    );
    Ok(target)
}

fn attach_to_session_pty(path: &Path) -> Result<()> {
    let mut reader = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("open PTY for reading {}", path.display()))?;
    let mut writer = OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("open PTY for writing {}", path.display()))?;

    let stdin_thread = thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0u8; 4096];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if writer.write_all(&buffer[..n]).is_err() {
                        break;
                    }
                    if writer.flush().is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut stdout = io::stdout().lock();
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                stdout
                    .write_all(&buffer[..n])
                    .context("write PTY output to stdout")?;
                stdout.flush().context("flush stdout")?;
            }
            Err(err) => return Err(err).context("read PTY output"),
        }
    }

    let _ = stdin_thread.join();
    Ok(())
}

fn shell_words(program: &std::path::Path, args: &[String]) -> String {
    let mut words = vec![quote_shell_word(&program.to_string_lossy())];
    words.extend(args.iter().map(|arg| quote_shell_word(arg)));
    words.join(" ")
}

fn quote_shell_word(value: &str) -> String {
    #[cfg(windows)]
    {
        if value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'\\' | b':' | b'.' | b'_' | b'-')
        }) {
            return value.to_owned();
        }
        format!("\"{}\"", value.replace('"', "\\\""))
    }
    #[cfg(not(windows))]
    {
        if value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        {
            return value.to_owned();
        }
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(windows)]
fn escape_cmd_set_value(value: &str) -> String {
    value.replace('"', "^\"")
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    fn cli_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn parses_app_shared_settings_export() {
        let cli = Cli::try_parse_from([
            "archductor",
            "settings",
            "export",
            "--output",
            "shared.toml",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Settings { .. }));
    }

    #[test]
    fn terminal_invocation_wraps_interactive_command() {
        let invocation =
            build_terminal_invocation("gnome-terminal", "cd /tmp && exec codex").unwrap();
        assert_eq!(invocation.program, "gnome-terminal");
        assert_eq!(
            invocation.args,
            vec!["--", "bash", "-lc", "cd /tmp && exec codex"]
        );

        let invocation = build_terminal_invocation("kitty", "cd /tmp && exec claude").unwrap();
        assert_eq!(
            invocation.args,
            vec!["-e", "bash", "-lc", "cd /tmp && exec claude"]
        );
    }

    #[test]
    fn terminal_invocation_supports_macos_terminal() {
        let invocation =
            build_terminal_invocation("macos-terminal", "cd \"/tmp/work\" && exec codex").unwrap();
        assert_eq!(invocation.program, "osascript");
        assert_eq!(
            invocation.args,
            vec![
                "-e",
                "tell application \"Terminal\" to do script \"cd \\\"/tmp/work\\\" && exec codex\"",
                "-e",
                "tell application \"Terminal\" to activate"
            ]
        );
    }

    #[test]
    fn terminal_invocation_accepts_path_and_case_variants() {
        let invocation =
            build_terminal_invocation("/usr/bin/Kitty", "cd /tmp && exec codex").unwrap();

        assert_eq!(invocation.program, "/usr/bin/Kitty");
        assert_eq!(
            invocation.args,
            vec!["-e", "bash", "-lc", "cd /tmp && exec codex"]
        );
    }

    #[test]
    fn terminal_invocation_matches_gtk_terminal_adapters() {
        let foot = build_terminal_invocation("foot", "cd /tmp && exec codex").unwrap();
        assert_eq!(foot.args, vec!["bash", "-lc", "cd /tmp && exec codex"]);

        let wezterm = build_terminal_invocation("wezterm", "cd /tmp && exec codex").unwrap();
        assert_eq!(
            wezterm.args,
            vec!["start", "--", "bash", "-lc", "cd /tmp && exec codex"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_terminal_invocation_uses_native_cmd_shell() {
        let invocation = build_terminal_invocation("wt.exe", "codex --help").unwrap();
        assert_eq!(invocation.program, "wt.exe");
        assert_eq!(
            invocation.args,
            vec!["new-tab", "cmd.exe", "/D", "/S", "/C", "codex --help"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_manual_session_command_sets_env_and_changes_drive() {
        let launch = SessionLaunch {
            kind: SessionKind::Codex,
            program: PathBuf::from("codex.exe"),
            args: vec!["--model".to_owned(), "gpt-test".to_owned()],
            cwd: PathBuf::from(r"C:\work space"),
            env: vec![(
                "ARCHDUCTOR_WORKSPACE_NAME".to_owned(),
                OsString::from("berlin"),
            )],
            harness_metadata: None,
            session_resume_id: None,
        };

        let command = render_manual_session_command(&launch);
        assert!(command.contains("set \"ARCHDUCTOR_WORKSPACE_NAME=berlin\""));
        assert!(command.contains("cd /D \"C:\\work space\""));
        assert!(command.ends_with("codex.exe --model gpt-test"));
        assert!(!command.contains("exec "));
    }

    #[test]
    #[cfg(not(windows))]
    fn manual_session_command_includes_workspace_env_and_program() {
        let launch = SessionLaunch {
            kind: SessionKind::Codex,
            program: PathBuf::from("codex"),
            args: Vec::new(),
            cwd: PathBuf::from("/tmp/work space"),
            env: vec![
                (
                    "ARCHDUCTOR_WORKSPACE_NAME".to_owned(),
                    OsString::from("berlin"),
                ),
                ("ARCHDUCTOR_PORT".to_owned(), OsString::from("3000")),
            ],
            harness_metadata: None,
            session_resume_id: None,
        };

        let command = render_manual_session_command(&launch);
        assert!(command.contains("cd '/tmp/work space'"));
        assert!(command.contains("ARCHDUCTOR_WORKSPACE_NAME=berlin"));
        assert!(command.contains("ARCHDUCTOR_PORT=3000"));
        assert!(command.contains("ARCHDUCTOR_PORT=3000 exec codex"));
        assert!(command.ends_with("exec codex"));
    }

    #[test]
    fn manual_codex_session_command_keeps_bootstrap_env_out_of_prompt() {
        let launch = SessionLaunch {
            kind: SessionKind::Codex,
            program: PathBuf::from("codex"),
            args: vec!["--model".to_owned(), "gpt-5.6-sol".to_owned()],
            cwd: PathBuf::from("/tmp/work"),
            env: vec![
                (
                    "ARCHDUCTOR_WORKSPACE_NAME".to_owned(),
                    OsString::from("berlin"),
                ),
                (
                    "ARCHDUCTOR_SESSION_BOOTSTRAP".to_owned(),
                    OsString::from("[archductor bootstrap for codex]\n/plan\n"),
                ),
            ],
            harness_metadata: Some("harness=codex;plan=true".to_owned()),
            session_resume_id: None,
        };

        let command = render_manual_session_command(&launch);
        assert!(command.contains("ARCHDUCTOR_SESSION_BOOTSTRAP"));
        #[cfg(not(windows))]
        {
            assert!(command.contains("exec codex --model gpt-5.6-sol"));
            assert!(!command.ends_with("'[archductor bootstrap for codex]\n/plan\n'"));
            assert!(!command.contains("exec codex '[archductor bootstrap for codex]"));
        }
        #[cfg(windows)]
        {
            assert!(command.contains(
                "set \"ARCHDUCTOR_SESSION_BOOTSTRAP=[archductor bootstrap for codex]\n/plan\n\""
            ));
            assert!(command.ends_with("codex --model gpt-5.6-sol"));
            assert!(!command.ends_with("[archductor bootstrap for codex]\n/plan\n"));
        }
    }

    #[test]
    fn cli_session_start_and_open_accept_explicit_model() {
        let start = Cli::try_parse_from([
            "archductor",
            "session",
            "start",
            "berlin",
            "--kind",
            "codex",
            "--model",
            "gpt-5.6-luna",
        ])
        .unwrap();
        let Command::Session {
            command: SessionCommand::Start {
                model: start_model, ..
            },
        } = start.command
        else {
            panic!("expected session start");
        };
        assert_eq!(start_model.as_deref(), Some("gpt-5.6-luna"));

        let open = Cli::try_parse_from([
            "archductor",
            "session",
            "open",
            "berlin",
            "--kind",
            "claude",
            "--model",
            "claude-sonnet-5",
            "--print-command",
        ])
        .unwrap();
        let Command::Session {
            command: SessionCommand::Open {
                model: open_model, ..
            },
        } = open.command
        else {
            panic!("expected session open");
        };
        assert_eq!(open_model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn cli_session_start_routes_provider_native_agents_through_archcar() {
        assert!(cli_session_start_uses_archcar(CliSessionKind::Codex));
        assert!(cli_session_start_uses_archcar(CliSessionKind::Claude));
        assert!(!cli_session_start_uses_archcar(CliSessionKind::Shell));
    }

    #[test]
    fn cli_session_stop_routes_provider_native_agents_through_archcar() {
        assert!(cli_session_stop_uses_archcar(SessionKind::Codex));
        assert!(cli_session_stop_uses_archcar(SessionKind::Claude));
        assert!(!cli_session_stop_uses_archcar(SessionKind::Shell));
    }

    #[test]
    fn cli_archcar_send_accepts_automation_input_kinds() {
        let control = Cli::try_parse_from([
            "archductor",
            "archcar",
            "send",
            "7",
            "--kind",
            "control-command",
            "/model",
            "gpt-5.6-sol",
        ])
        .unwrap();
        let Command::Archcar {
            command:
                ArchcarCommand::Send {
                    session_id,
                    kind,
                    visible_input,
                    immediate,
                    input,
                },
        } = control.command
        else {
            panic!("expected archcar send");
        };
        assert_eq!(session_id, 7);
        assert_eq!(kind, CliArchcarInputKind::ControlCommand);
        assert_eq!(visible_input, None);
        assert!(!immediate);
        assert_eq!(input, vec!["/model".to_owned(), "gpt-5.6-sol".to_owned()]);

        let review = Cli::try_parse_from([
            "archductor",
            "archcar",
            "send",
            "8",
            "--kind",
            "review-prompt",
            "--visible-input",
            "Review selected comments",
            "--immediate",
            "address",
            "comments",
        ])
        .unwrap();
        let Command::Archcar {
            command:
                ArchcarCommand::Send {
                    session_id,
                    kind,
                    visible_input,
                    immediate,
                    input,
                },
        } = review.command
        else {
            panic!("expected archcar send");
        };
        assert_eq!(session_id, 8);
        assert_eq!(kind, CliArchcarInputKind::ReviewPrompt);
        assert_eq!(visible_input.as_deref(), Some("Review selected comments"));
        assert!(immediate);
        assert_eq!(input, vec!["address".to_owned(), "comments".to_owned()]);
    }

    #[test]
    fn cli_archcar_model_uses_structured_model_request() {
        let cli =
            Cli::try_parse_from(["archductor", "archcar", "model", "7", "gpt-5.6-terra"]).unwrap();

        let Command::Archcar {
            command: ArchcarCommand::Model { session_id, model },
        } = cli.command
        else {
            panic!("expected archcar model");
        };
        assert_eq!(session_id, 7);
        assert_eq!(model, "gpt-5.6-terra");
    }

    #[test]
    fn cli_archcar_control_parses_effort_and_permission_mode() {
        let effort = Cli::try_parse_from(["archductor", "archcar", "effort", "7", "high"]).unwrap();
        let Command::Archcar {
            command: ArchcarCommand::Effort { session_id, level },
        } = effort.command
        else {
            panic!("expected archcar effort");
        };
        assert_eq!(session_id, 7);
        assert_eq!(level, "high");

        let permission =
            Cli::try_parse_from(["archductor", "archcar", "permission-mode", "7", "default"])
                .unwrap();
        let Command::Archcar {
            command: ArchcarCommand::PermissionMode { session_id, mode },
        } = permission.command
        else {
            panic!("expected archcar permission-mode");
        };
        assert_eq!(session_id, 7);
        assert_eq!(mode, "default");
    }

    #[test]
    fn cli_archcar_interrupt_parses_session_id() {
        let cli = Cli::try_parse_from(["archductor", "archcar", "interrupt", "7"]).unwrap();

        let Command::Archcar {
            command: ArchcarCommand::Interrupt { session_id },
        } = cli.command
        else {
            panic!("expected archcar interrupt");
        };

        assert_eq!(session_id, 7);
    }

    #[test]
    fn cli_archcar_messages_reads_thread_messages() {
        let cli = Cli::try_parse_from(["archductor", "archcar", "messages", "42"]).unwrap();

        let Command::Archcar {
            command: ArchcarCommand::Messages { thread_id },
        } = cli.command
        else {
            panic!("expected archcar messages");
        };

        assert_eq!(thread_id, 42);
    }

    #[test]
    fn cli_archcar_provider_interactions_parse_commands() {
        let list = Cli::try_parse_from([
            "archductor",
            "archcar",
            "interactions",
            "list",
            "--thread-id",
            "42",
            "--all",
        ])
        .unwrap();
        let Command::Archcar {
            command:
                ArchcarCommand::Interactions {
                    command: ArchcarInteractionsCommand::List { thread_id, all, .. },
                },
        } = list.command
        else {
            panic!("expected archcar interactions list");
        };
        assert_eq!(thread_id, Some(42));
        assert!(all);

        let answer = Cli::try_parse_from([
            "archductor",
            "archcar",
            "interactions",
            "answer",
            "interaction-1",
            "--answers-json",
            r#"{"scope":"yes"}"#,
        ])
        .unwrap();
        let Command::Archcar {
            command:
                ArchcarCommand::Interactions {
                    command:
                        ArchcarInteractionsCommand::Answer {
                            interaction_id,
                            answers_json,
                        },
                },
        } = answer.command
        else {
            panic!("expected archcar interactions answer");
        };
        assert_eq!(interaction_id, "interaction-1");
        assert_eq!(
            parse_answers_json(&answers_json).unwrap(),
            vec![("scope".to_owned(), "yes".to_owned())]
        );

        let always = Cli::try_parse_from([
            "archductor",
            "archcar",
            "interactions",
            "allow",
            "interaction-1",
            "--always",
        ])
        .unwrap();
        assert!(matches!(
            always.command,
            Command::Archcar {
                command: ArchcarCommand::Interactions {
                    command: ArchcarInteractionsCommand::Allow { always: true, .. }
                }
            }
        ));
    }

    #[test]
    fn archcar_interactions_allow_always_is_rejected_until_persistence_exists() {
        assert_eq!(
            archcar_allow_resolution(true).unwrap_err().to_string(),
            "--always is not supported yet; run allow without --always for a one-time approval"
        );
    }

    #[test]
    fn provider_interactions_render_concise_lines_without_raw_payload() {
        let interaction = provider_interaction_fixture();
        let concise = render_provider_interactions(std::slice::from_ref(&interaction), false);
        let detail = render_provider_interactions(&[interaction], true);

        assert!(concise.contains("interaction-1 provider=claude kind=question status=pending"));
        assert!(concise.contains("Need input: Pick a scope"));
        assert!(!concise.contains("\"secret\""));
        assert!(detail.contains("request="));
        assert!(detail.contains("\"secret\""));
    }

    #[test]
    fn cli_session_send_accepts_provider_thread_and_message() {
        let cli = Cli::try_parse_from([
            "archductor",
            "session",
            "send",
            "berlin",
            "--kind",
            "claude",
            "--thread-id",
            "42",
            "--input-kind",
            "review-prompt",
            "--visible-input",
            "Review selected comments",
            "--timeout-ms",
            "2500",
            "--immediate",
            "fix",
            "the",
            "bug",
        ])
        .unwrap();

        let Command::Session {
            command:
                SessionCommand::Send {
                    workspace,
                    kind,
                    thread_id,
                    input_kind,
                    visible_input,
                    timeout_ms,
                    immediate,
                    message,
                },
        } = cli.command
        else {
            panic!("expected session send");
        };

        assert_eq!(workspace, "berlin");
        assert_eq!(kind, CliSessionKind::Claude);
        assert_eq!(thread_id, Some(42));
        assert_eq!(input_kind, CliArchcarInputKind::ReviewPrompt);
        assert_eq!(visible_input.as_deref(), Some("Review selected comments"));
        assert_eq!(timeout_ms, 2500);
        assert!(immediate);
        assert_eq!(
            message,
            vec!["fix".to_owned(), "the".to_owned(), "bug".to_owned()]
        );
    }

    #[test]
    fn cli_session_send_keeps_distinct_claude_thread_targets() {
        let first = Cli::try_parse_from([
            "archductor",
            "session",
            "send",
            "berlin",
            "--kind",
            "claude",
            "--thread-id",
            "101",
            "first",
        ])
        .unwrap();
        let second = Cli::try_parse_from([
            "archductor",
            "session",
            "send",
            "berlin",
            "--kind",
            "claude",
            "--thread-id",
            "202",
            "second",
        ])
        .unwrap();

        assert_eq!(
            session_send_thread_target(first),
            Some((101, "first".to_owned()))
        );
        assert_eq!(
            session_send_thread_target(second),
            Some((202, "second".to_owned()))
        );
    }

    #[test]
    fn cli_claude_session_send_does_not_wait_for_ready_before_first_input() {
        assert!(!session_send_waits_for_ready(SessionKind::Claude, false));
        assert!(session_send_waits_for_ready(SessionKind::Claude, true));
        assert!(session_send_waits_for_ready(SessionKind::Codex, false));
    }

    fn session_send_thread_target(cli: Cli) -> Option<(i64, String)> {
        let Command::Session {
            command:
                SessionCommand::Send {
                    kind,
                    thread_id,
                    message,
                    ..
                },
        } = cli.command
        else {
            return None;
        };
        if kind == CliSessionKind::Claude {
            thread_id.map(|thread_id| (thread_id, message.join(" ")))
        } else {
            None
        }
    }

    #[test]
    fn history_list_render_shows_local_session_rows() {
        let text = render_history_list(&[LocalChatHistorySummary {
            process_id: 9,
            chat_thread_id: None,
            repository_name: "demo".to_owned(),
            workspace_name: "berlin".to_owned(),
            workspace_path: PathBuf::from("/tmp/berlin"),
            agent_type: "Codex".to_owned(),
            status: "exited".to_owned(),
            started_at: "2026-06-21T01:00:00Z".to_owned(),
            updated_at: "2026-06-21T01:05:00Z".to_owned(),
            message_count: 3,
            preview: "fixed tests\nwith detail".to_owned(),
            harness: Some("plan=true".to_owned()),
        }]);

        assert!(text.contains("#9\texited\t2026-06-21T01:05:00Z\tdemo\tberlin\t3 message(s)"));
        assert!(text.contains("fixed tests with detail"));
    }

    #[test]
    fn history_message_render_labels_transcript_roles() {
        let text = render_history_messages(&[
            LocalChatHistoryMessage {
                role: "user".to_owned(),
                content: "run tests".to_owned(),
            },
            LocalChatHistoryMessage {
                role: "agent".to_owned(),
                content: "tests passed".to_owned(),
            },
        ]);

        assert!(text.contains("You\nrun tests\n\n"));
        assert!(text.contains("Agent\ntests passed\n\n"));
    }

    #[test]
    fn archcar_messages_render_projected_provider_events_without_raw_payloads() {
        let text = render_archcar_protocol_messages(&[
            ArchcarMessage {
                id: -1,
                role: "assistant".to_owned(),
                content: "Here is the answer".to_owned(),
                source: "provider_event".to_owned(),
                inline_event: None,
                context_usage: None,
            },
            ArchcarMessage {
                id: -2,
                role: "reasoning".to_owned(),
                content: "Reasoning\nChecking constraints".to_owned(),
                source: "provider_event".to_owned(),
                inline_event: None,
                context_usage: None,
            },
        ]);

        assert!(text.contains("Assistant\nHere is the answer\n\n"));
        assert!(text.contains("Reasoning\nChecking constraints\n\n"));
    }

    #[test]
    fn archcar_message_render_preserves_chat_content_that_starts_with_label() {
        let text = render_archcar_protocol_messages(&[
            ArchcarMessage {
                id: 1,
                role: "user".to_owned(),
                content: "You\nshould keep this heading".to_owned(),
                source: "user_send".to_owned(),
                inline_event: None,
                context_usage: None,
            },
            ArchcarMessage {
                id: -1,
                role: "reasoning".to_owned(),
                content: "Reasoning\nbut projection titles can be stripped".to_owned(),
                source: "provider_event".to_owned(),
                inline_event: None,
                context_usage: None,
            },
        ]);

        assert!(text.contains("You\nYou\nshould keep this heading\n\n"));
        assert!(text.contains("Reasoning\nbut projection titles can be stripped\n\n"));
    }

    #[test]
    fn message_text_or_stdin_rejects_blank_positional_message() {
        let err = message_text_or_stdin(vec!["   ".to_owned(), "\t".to_owned()]).unwrap_err();

        assert!(err.to_string().contains("message is required"));
    }

    #[test]
    fn linked_directory_render_lists_target_and_context_link() {
        let text = render_linked_directories(&[LinkedDirectory {
            id: 1,
            workspace_id: 10,
            workspace_name: "frontend".to_owned(),
            workspace_path: PathBuf::from("/tmp/frontend"),
            target_workspace_id: 11,
            target_workspace_name: "backend".to_owned(),
            target_workspace_path: PathBuf::from("/tmp/backend"),
            link_path: PathBuf::from("/tmp/frontend/.context/linked-directories/backend"),
            created_at: "2026-06-21T12:00:00Z".to_owned(),
        }]);

        assert_eq!(
            text,
            "backend\t/tmp/backend\t/tmp/frontend/.context/linked-directories/backend\n"
        );
    }

    #[test]
    fn cli_rejects_removed_internal_run_codex_session_command() {
        let parse = Cli::try_parse_from(["archductor", "internal", "run-codex-session", "demo"]);

        assert!(parse.is_err());
    }

    #[test]
    fn cli_parses_gtk_launcher_with_passthrough_args() {
        let parse = Cli::try_parse_from([
            "archductor",
            "gtk",
            "--workspace",
            "berlin",
            "--tab",
            "checks",
        ])
        .unwrap();

        match parse.command {
            Command::Gtk { args } => {
                assert_eq!(
                    args,
                    vec![
                        "--workspace".to_owned(),
                        "berlin".to_owned(),
                        "--tab".to_owned(),
                        "checks".to_owned(),
                    ]
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn server_mode_detection_ignores_gtk_trailing_archcar_serve() {
        assert!(should_run_archcar_server_mode([
            "archductor",
            "--archcar-serve"
        ]));
        assert!(!should_run_archcar_server_mode([
            "archductor",
            "gtk",
            "--archcar-serve"
        ]));
    }

    #[test]
    fn gtk_binary_path_prefers_env_override() {
        let _guard = cli_env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let override_path = temp.path().join("custom-gtk");
        let previous = std::env::var_os("ARCHDUCTOR_GTK_BIN");
        std::env::set_var("ARCHDUCTOR_GTK_BIN", &override_path);

        let selected = gtk_binary_path();

        match previous {
            Some(previous) => std::env::set_var("ARCHDUCTOR_GTK_BIN", previous),
            None => std::env::remove_var("ARCHDUCTOR_GTK_BIN"),
        }
        assert_eq!(selected, override_path);
    }

    #[test]
    fn gtk_binary_path_prefers_existing_sibling() {
        let temp = tempfile::tempdir().unwrap();
        let cli = temp
            .path()
            .join(format!("archductor{}", std::env::consts::EXE_SUFFIX));
        let gtk = temp
            .path()
            .join(format!("archductor-gtk{}", std::env::consts::EXE_SUFFIX));
        fs::write(&gtk, "").unwrap();

        assert_eq!(gtk_binary_path_for_cli_exe(Some(cli)), gtk);
    }

    #[cfg(unix)]
    #[test]
    fn launch_gtk_forwards_child_arguments_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = cli_env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let fake = temp.path().join("archductor-gtk");
        let args_out = temp.path().join("args.txt");
        fs::write(
            &fake,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ARCHDUCTOR_GTK_ARGS_OUT\"\n",
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let previous_bin = std::env::var_os("ARCHDUCTOR_GTK_BIN");
        let previous_out = std::env::var_os("ARCHDUCTOR_GTK_ARGS_OUT");
        std::env::set_var("ARCHDUCTOR_GTK_BIN", &fake);
        std::env::set_var("ARCHDUCTOR_GTK_ARGS_OUT", &args_out);

        launch_gtk(&[
            "--workspace".to_owned(),
            "berlin".to_owned(),
            "--archcar-serve".to_owned(),
        ])
        .unwrap();

        match previous_bin {
            Some(previous) => std::env::set_var("ARCHDUCTOR_GTK_BIN", previous),
            None => std::env::remove_var("ARCHDUCTOR_GTK_BIN"),
        }
        match previous_out {
            Some(previous) => std::env::set_var("ARCHDUCTOR_GTK_ARGS_OUT", previous),
            None => std::env::remove_var("ARCHDUCTOR_GTK_ARGS_OUT"),
        }
        assert_eq!(
            fs::read_to_string(args_out).unwrap(),
            "--workspace\nberlin\n--archcar-serve\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_gtk_reports_nonzero_child_exit() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = cli_env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let fake = temp.path().join("archductor-gtk");
        fs::write(&fake, "#!/bin/sh\nexit 17\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let previous = std::env::var_os("ARCHDUCTOR_GTK_BIN");
        std::env::set_var("ARCHDUCTOR_GTK_BIN", &fake);

        let err = launch_gtk(&[]).unwrap_err();

        match previous {
            Some(previous) => std::env::set_var("ARCHDUCTOR_GTK_BIN", previous),
            None => std::env::remove_var("ARCHDUCTOR_GTK_BIN"),
        }
        assert!(format!("{err:#}").contains("GTK app exited with status"));
    }

    #[test]
    fn cli_parses_workspace_delete_cleanup_flags() {
        let parse = Cli::try_parse_from([
            "archductor",
            "workspace",
            "delete",
            "berlin",
            "--remove-worktree",
            "--delete-branch",
        ])
        .unwrap();

        match parse.command {
            Command::Workspace {
                command:
                    WorkspaceCommand::Delete {
                        name,
                        remove_worktree,
                        delete_branch,
                    },
            } => {
                assert_eq!(name, "berlin");
                assert!(remove_worktree);
                assert!(delete_branch);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_workspace_branch_actions() {
        let parse = Cli::try_parse_from([
            "archductor",
            "workspace",
            "branch",
            "berlin",
            "checkout",
            "lc/next",
        ])
        .unwrap();

        match parse.command {
            Command::Workspace {
                command:
                    WorkspaceCommand::Branch {
                        workspace,
                        command: WorkspaceBranchCommand::Checkout { branch },
                    },
            } => {
                assert_eq!(workspace, "berlin");
                assert_eq!(branch, "lc/next");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_workspace_duplicate_branch() {
        let parse = Cli::try_parse_from([
            "archductor",
            "workspace",
            "duplicate",
            "berlin",
            "oslo",
            "--branch",
            "lc/oslo",
        ])
        .unwrap();

        match parse.command {
            Command::Workspace {
                command:
                    WorkspaceCommand::Duplicate {
                        name,
                        new_name,
                        branch,
                    },
            } => {
                assert_eq!(name, "berlin");
                assert_eq!(new_name, "oslo");
                assert_eq!(branch.as_deref(), Some("lc/oslo"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn cli_parses_workspace_timeline_filter() {
        let parse = Cli::try_parse_from([
            "archductor",
            "workspace",
            "timeline",
            "berlin",
            "--kind",
            "branch.renamed",
        ])
        .unwrap();

        match parse.command {
            Command::Workspace {
                command: WorkspaceCommand::Timeline { workspace, kind },
            } => {
                assert_eq!(workspace, "berlin");
                assert_eq!(kind.as_deref(), Some("branch.renamed"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn timeline_render_outputs_append_only_rows() {
        let text = render_workspace_timeline(&[WorkspaceTimelineEvent {
            id: 7,
            workspace_id: 2,
            workspace_name: "berlin".to_owned(),
            kind: "branch.renamed".to_owned(),
            summary: "Renamed branch lc/a to lc/b".to_owned(),
            created_at: "2026-07-09T12:00:00Z".to_owned(),
        }]);

        assert_eq!(
            text,
            "#7\t2026-07-09T12:00:00Z\tbranch.renamed\tRenamed branch lc/a to lc/b\n"
        );
    }

    fn provider_interaction_fixture() -> ProviderInteractionRecord {
        ProviderInteractionRecord {
            id: "interaction-1".to_owned(),
            provider_key: "claude".to_owned(),
            workspace: "berlin".to_owned(),
            thread_id: 42,
            session_id: 7,
            native_session_id: Some("claude-session-1".to_owned()),
            native_id: "toolu-1".to_owned(),
            kind: archductor_core::archcar::harness_contract::ProviderInteractionKind::UserQuestion,
            title: "Need input".to_owned(),
            detail: "Pick a scope".to_owned(),
            choices: vec!["yes".to_owned(), "no".to_owned()],
            native_request: serde_json::json!({"secret": "raw"}),
            request_fingerprint: "fingerprint".to_owned(),
            status: archductor_core::provider_interactions::ProviderInteractionStatus::Pending,
            resolution: None,
            native_response: None,
            error: None,
            created_at: "1".to_owned(),
            resolved_at: None,
            consumed_at: None,
        }
    }
}
