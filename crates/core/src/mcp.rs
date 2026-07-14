use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpStatus {
    pub workspace_path: PathBuf,
    pub claude_user: Vec<McpServer>,
    pub claude_project: Vec<McpServer>,
    pub codex_user: Vec<McpServer>,
    pub codex_project: Vec<McpServer>,
    pub cursor_user: Vec<McpServer>,
    pub cursor_project: Vec<McpServer>,
    pub codex_provider: Option<String>,
    pub claude_provider: Option<String>,
    pub codex_executable_available: bool,
    pub claude_executable_available: bool,
    pub cursor_executable_available: bool,
    pub codex_authenticated: bool,
    pub claude_authenticated: bool,
    pub cursor_authenticated: bool,
}

pub fn workspace_mcp_status(workspace_path: &Path) -> McpStatus {
    let home = crate::platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let settings = crate::settings::load_repository_settings(workspace_path).ok();
    let codex_provider = settings
        .as_ref()
        .and_then(|settings| settings.providers.codex_provider.clone());
    let claude_provider = settings
        .as_ref()
        .and_then(|settings| settings.providers.claude_provider.clone());
    let codex_executable = settings
        .as_ref()
        .and_then(|settings| settings.providers.codex_executable_path.as_deref())
        .filter(|path| !path.trim().is_empty())
        .unwrap_or("codex");
    let claude_executable = settings
        .as_ref()
        .and_then(|settings| settings.providers.claude_code_executable_path.as_deref())
        .filter(|path| !path.trim().is_empty())
        .unwrap_or("claude");
    let codex_authenticated = is_codex_auth_present(codex_provider.as_deref());
    let claude_authenticated = is_claude_auth_present(claude_provider.as_deref());
    let cursor_authenticated = is_file_non_empty(home.join(".cursor/mcp.json").as_path());

    McpStatus {
        workspace_path: workspace_path.to_path_buf(),
        claude_user: read_claude_mcp(&home.join(".claude.json")),
        claude_project: read_json_mcp_servers(&workspace_path.join(".mcp.json")),
        codex_user: read_codex_mcp(&home.join(".codex/config.toml")),
        codex_project: read_codex_mcp(&workspace_path.join(".codex/config.toml")),
        cursor_user: read_cursor_mcp(&home.join(".cursor/mcp.json")),
        cursor_project: read_cursor_mcp(&workspace_path.join(".cursor/mcp.json")),
        codex_provider,
        claude_provider,
        codex_executable_available: command_or_file_exists(codex_executable),
        claude_executable_available: command_or_file_exists(claude_executable),
        cursor_executable_available: command_or_file_exists("cursor"),
        codex_authenticated,
        claude_authenticated,
        cursor_authenticated,
    }
}

fn is_file_non_empty(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.len() != 0)
        .unwrap_or(false)
}

fn command_or_file_exists(path_or_command: &str) -> bool {
    if Path::new(path_or_command).exists() {
        true
    } else {
        command_exists(path_or_command)
    }
}

fn is_codex_auth_present(provider: Option<&str>) -> bool {
    let configured_openai = provider
        .map(|value| {
            value.to_ascii_lowercase().contains("openai")
                || value.to_ascii_lowercase().contains("open-ai")
        })
        .unwrap_or(true);
    if configured_openai {
        std::env::var_os("OPENAI_API_KEY").is_some()
            || std::env::var_os("OPENAI_API_TOKEN").is_some()
    } else {
        false
    }
}

fn is_claude_auth_present(provider: Option<&str>) -> bool {
    let configured_anthropic = provider
        .map(|value| {
            value.to_ascii_lowercase().contains("anthropic")
                || value.to_ascii_lowercase().contains("claude")
        })
        .unwrap_or(true);
    if configured_anthropic {
        std::env::var_os("ANTHROPIC_API_KEY").is_some()
    } else {
        false
    }
}

fn command_exists(command: &str) -> bool {
    crate::doctor::command_exists(command)
}

fn read_claude_mcp(path: &Path) -> Vec<McpServer> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    parse_claude_json_servers(&contents, path.display().to_string())
}

fn read_json_mcp_servers(path: &Path) -> Vec<McpServer> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let source = path.display().to_string();
    parse_json_mcp_keys(&contents, &source)
}

fn read_codex_mcp(path: &Path) -> Vec<McpServer> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let source = path.display().to_string();
    parse_toml_mcp_keys(&contents, &source)
}

fn read_cursor_mcp(path: &Path) -> Vec<McpServer> {
    // Cursor uses the same JSON shape as Claude project .mcp.json: {"mcpServers": {...}}
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    parse_claude_json_servers(&contents, path.display().to_string())
}

fn parse_claude_json_servers(json: &str, source: String) -> Vec<McpServer> {
    // Claude stores servers under {"mcpServers": {"name": {...}}}
    let needle = "\"mcpServers\"";
    let Some(start) = json.find(needle) else {
        return Vec::new();
    };
    let after = &json[start + needle.len()..];
    let Some(brace) = after.find('{') else {
        return Vec::new();
    };
    let block = &after[brace..];
    parse_json_mcp_keys(block, &source)
}

fn parse_json_mcp_keys(json: &str, source: &str) -> Vec<McpServer> {
    let mut servers = Vec::new();
    let mut rest = json;
    while let Some(quote) = rest.find('"') {
        rest = &rest[quote + 1..];
        let Some(close) = rest.find('"') else { break };
        let key = &rest[..close];
        if !key.is_empty() && !key.starts_with('$') && !key.contains(' ') {
            servers.push(McpServer {
                name: key.to_owned(),
                source: source.to_owned(),
            });
        }
        rest = &rest[close + 1..];
        // skip to next top-level key (after the colon and value block)
        let Some(colon) = rest.find(':') else { break };
        rest = &rest[colon + 1..];
        // skip past the value object or string
        let trimmed = rest.trim_start();
        if trimmed.starts_with('{') {
            let depth_start = rest.find('{').unwrap_or(0);
            rest = skip_json_value(&rest[depth_start..]);
        } else if trimmed.starts_with('"') {
            let quote_start = rest.find('"').unwrap_or(0);
            rest = &rest[quote_start + 1..];
            if let Some(end) = rest.find('"') {
                rest = &rest[end + 1..];
            }
        }
    }
    servers
}

fn skip_json_value(json: &str) -> &str {
    let mut depth = 0i32;
    let mut chars = json.char_indices();
    for (i, ch) in &mut chars {
        match ch {
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth <= 0 {
                    return &json[i + 1..];
                }
            }
            _ => {}
        }
    }
    json
}

fn parse_toml_mcp_keys(toml: &str, source: &str) -> Vec<McpServer> {
    // Codex stores servers under [mcp_servers] or [mcpServers] section
    let mut in_mcp = false;
    let mut servers = Vec::new();
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            let section = trimmed.trim_matches(|c| c == '[' || c == ']');
            in_mcp = section == "mcp_servers" || section == "mcpServers";
            continue;
        }
        if in_mcp {
            if let Some(eq) = trimmed.find('=') {
                let key = trimmed[..eq].trim();
                if !key.is_empty() {
                    servers.push(McpServer {
                        name: key.to_owned(),
                        source: source.to_owned(),
                    });
                }
            }
        }
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_json_extracts_server_names() {
        let json =
            r#"{"mcpServers": {"filesystem": {"command": "npx"}, "github": {"command": "npx"}}}"#;
        let servers = parse_claude_json_servers(json, "~/.claude.json".to_owned());
        let names: Vec<_> = servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"filesystem"), "got: {names:?}");
        assert!(names.contains(&"github"), "got: {names:?}");
    }

    #[test]
    fn parse_toml_mcp_reads_section_keys() {
        let toml = "[mcp_servers]\nfilesystem = {}\ngithub = {}\n";
        let servers = parse_toml_mcp_keys(toml, "~/.codex/config.toml");
        let names: Vec<_> = servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"filesystem"));
        assert!(names.contains(&"github"));
    }
}
