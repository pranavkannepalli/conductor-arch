#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextTodo {
    pub done: bool,
    pub text: String,
}

pub fn parse_context_todos(contents: &str) -> Vec<ContextTodo> {
    contents
        .lines()
        .filter_map(parse_context_todo_line)
        .collect()
}

fn parse_context_todo_line(line: &str) -> Option<ContextTodo> {
    let trimmed = line.trim();
    let (done, text) = if let Some(rest) = trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
    {
        (true, rest)
    } else {
        let rest = trimmed.strip_prefix("- [ ] ")?;
        (false, rest)
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(ContextTodo {
        done,
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_context_todos_reads_open_and_done_markdown_tasks() {
        let todos = parse_context_todos(
            "# Todos\n\n- [ ] finish parser\n- [x] write tests\n- [X] ship fix\n- [ ]   \n",
        );

        assert_eq!(
            todos,
            vec![
                ContextTodo {
                    done: false,
                    text: "finish parser".to_owned(),
                },
                ContextTodo {
                    done: true,
                    text: "write tests".to_owned(),
                },
                ContextTodo {
                    done: true,
                    text: "ship fix".to_owned(),
                },
            ]
        );
    }
}
