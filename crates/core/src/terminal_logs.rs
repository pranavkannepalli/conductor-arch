use anyhow::{Context, Result};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::workspace::ProcessRecord;

const TERMINAL_SEARCH_CONTEXT_LINES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLogMatch {
    pub process_id: i64,
    pub command: String,
    pub log_path: PathBuf,
    pub line_number: usize,
    pub line: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSessionSummary {
    pub process: ProcessRecord,
    pub line_count: usize,
    pub byte_count: usize,
    pub preview: String,
}

pub(crate) fn search_terminal_logs(
    processes: Vec<ProcessRecord>,
    query: &str,
) -> Result<Vec<TerminalLogMatch>> {
    let query = query.trim();
    anyhow::ensure!(!query.is_empty(), "terminal log search query is required");
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    for process in processes {
        let contents = match fs::read_to_string(&process.log_path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("read log {}", process.log_path.display()));
            }
        };
        let lines = contents.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if line.to_lowercase().contains(&needle) {
                let start = index.saturating_sub(TERMINAL_SEARCH_CONTEXT_LINES);
                let end = (index + TERMINAL_SEARCH_CONTEXT_LINES + 1).min(lines.len());
                let mut context_before = Vec::new();
                let mut context_after = Vec::new();

                for line in &lines[start..index] {
                    context_before.push((*line).to_owned());
                }
                for line in &lines[index + 1..end] {
                    context_after.push((*line).to_owned());
                }

                matches.push(TerminalLogMatch {
                    process_id: process.id,
                    command: process.command.clone(),
                    log_path: process.log_path.clone(),
                    line_number: index + 1,
                    line: (*line).to_owned(),
                    context_before,
                    context_after,
                });
            }
        }
    }
    Ok(matches)
}

pub(crate) fn summarize_terminal_sessions(
    processes: Vec<ProcessRecord>,
) -> Result<Vec<TerminalSessionSummary>> {
    processes
        .into_iter()
        .map(|process| {
            let contents = match fs::read_to_string(&process.log_path) {
                Ok(contents) => contents,
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    return Ok(TerminalSessionSummary {
                        process,
                        line_count: 0,
                        byte_count: 0,
                        preview: "(missing transcript)".to_owned(),
                    });
                }
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("read log {}", process.log_path.display()));
                }
            };
            Ok(TerminalSessionSummary {
                process,
                line_count: contents.lines().count(),
                byte_count: contents.len(),
                preview: terminal_log_preview(&contents),
            })
        })
        .collect()
}

pub(crate) fn terminal_log_preview(contents: &str) -> String {
    contents
        .lines()
        .rev()
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .unwrap_or_else(|| "(empty transcript)".to_owned())
}
