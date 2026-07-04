# PTY Parser Fixture Corpus

These fixtures are sanitized Codex PTY replay logs for parser regression tests.
Each `raw/*.json` file stores PTY chunks in arrival order, including ANSI
control sequences and chunk timing metadata where relevant. Each
`expected/*.json` file stores the parsed transcript items expected from the
final terminal screen after replay through `vt100`.

The integration test replays these logs locally and does not launch Codex.
