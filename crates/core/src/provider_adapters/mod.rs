pub mod claude_hooks;
pub mod claude_stream;
pub mod codex_app_server;

#[cfg(test)]
mod tests {
    #[test]
    fn provider_adapters_do_not_depend_on_legacy_codex_screen_parser_symbols() {
        let sources = [
            ("codex_app_server.rs", include_str!("codex_app_server.rs")),
            ("claude_stream.rs", include_str!("claude_stream.rs")),
        ];
        let forbidden = [
            concat!("visible", "_screen_text"),
            concat!("codex", "_tui"),
            concat!("Codex", "ParseCursor"),
            concat!("parse_codex", "_screen_delta"),
            concat!("parse_codex", "_screen_messages"),
        ];

        for (path, source) in sources {
            for symbol in forbidden {
                assert!(
                    !source.contains(symbol),
                    "{path} must not reference legacy screen parser symbol {symbol}"
                );
            }
        }

        assert!(!include_str!("codex_app_server.rs").contains("claude_stream"));
        assert!(!include_str!("claude_stream.rs").contains("codex_app_server"));
    }
}
