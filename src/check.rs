use crate::command::Command;
use crate::param::Param;
use crate::parser::{parse, Event, EventData};
use crate::utils::{
    META_BINNAME, META_COMBINE_SHORTS, META_DEFAULT_SUBCOMMAND, META_DOTENV,
    META_EXTERNAL_SUBCOMMANDS, META_GROUP_COMMANDS, META_INHERIT_FLAG_OPTIONS, META_MAN_SECTION,
    META_REQUIRE_BASH, META_REQUIRE_TOOLS, META_SYMBOL, META_VERSION, ROOT_NAME,
};

use std::collections::HashSet;
use std::fmt;

const META_KEYS: [&str; 12] = [
    META_VERSION,
    META_BINNAME,
    META_DOTENV,
    META_DEFAULT_SUBCOMMAND,
    META_INHERIT_FLAG_OPTIONS,
    META_SYMBOL,
    META_COMBINE_SHORTS,
    META_EXTERNAL_SUBCOMMANDS,
    META_MAN_SECTION,
    META_REQUIRE_TOOLS,
    META_REQUIRE_BASH,
    META_GROUP_COMMANDS,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    fn error(line: usize, message: String) -> Self {
        Self {
            line,
            severity: Severity::Error,
            message,
        }
    }

    fn warning(line: usize, message: String) -> Self {
        Self {
            line,
            severity: Severity::Warning,
            message,
        }
    }
}

/// Formats as `<line>: <severity>: <message>`, callers prefix the script path
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.line, self.severity, self.message)
    }
}

/// Lint an argc-based script, returning diagnostics sorted by line
pub fn check(source: &str) -> Vec<Diagnostic> {
    let mut output = vec![];
    if let Err(err) = Command::new(source, ROOT_NAME) {
        let (line, message) = split_error_line(&err.to_string());
        output.push(Diagnostic::error(line, message));
    }
    if let Ok(events) = parse(source) {
        check_events(&events, &mut output);
        check_errexit(source, &events, &mut output);
    }
    check_eval_line(source, &mut output);
    output.sort_by_key(|v| v.line);
    output
}

// Errors embed their line as `(line N)` or `at line N`
fn split_error_line(message: &str) -> (usize, String) {
    if let Some(start) = message.find("(line ") {
        let rest = &message[start + 6..];
        if let Some(end) = rest.find(')') {
            if let Ok(line) = rest[..end].parse() {
                let message = format!("{}{}", &message[..start], &rest[end + 1..]);
                return (line, message);
            }
        }
    }
    if let Some(start) = message.find("at line ") {
        let digits: String = message[start + 8..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(line) = digits.parse() {
            return (line, message.to_string());
        }
    }
    (1, message.to_string())
}

fn check_events(events: &[Event], output: &mut Vec<Diagnostic>) {
    let mut in_cmd = false;
    let mut aliases: Option<(&Vec<String>, usize)> = None;
    for Event { data, position } in events {
        let position = *position;
        match data {
            EventData::Meta(key, _) => {
                if !META_KEYS.contains(&key.as_str()) {
                    let mut message = format!("unknown @meta key `{key}`");
                    if let Some(known) = suggest_meta_key(key) {
                        message.push_str(&format!(", did you mean `{known}`?"));
                    }
                    output.push(Diagnostic::warning(position, message));
                }
            }
            EventData::Env(param) => {
                let name = param.id();
                if !is_shell_var_name(name) {
                    let mut message =
                        format!("@env name `{name}` is not a valid shell variable name");
                    if META_KEYS.contains(&name) {
                        message.push_str(&format!(", did you mean `@meta {name}`?"));
                    }
                    output.push(Diagnostic::warning(position, message));
                }
            }
            EventData::Cmd(_) => {
                in_cmd = true;
                aliases = None;
            }
            EventData::Aliases(values) => {
                if in_cmd {
                    aliases = Some((values, position));
                }
            }
            EventData::Func(name) => {
                if let (true, Some((values, alias_pos))) = (in_cmd, aliases.take()) {
                    let child = name.rsplit("::").next().unwrap_or(name);
                    let cmd_name = child.trim_end_matches('_');
                    // An alias equal to the function name itself is already a build error
                    if cmd_name != child && values.iter().any(|v| v == cmd_name) {
                        output.push(Diagnostic::warning(
                            alias_pos,
                            format!("@alias `{cmd_name}` is the same as its command's name"),
                        ));
                    }
                }
                in_cmd = false;
            }
            _ => {}
        }
    }
}

fn check_eval_line(source: &str, output: &mut Vec<Diagnostic>) {
    // A script built by `--argc-build` no longer needs argc
    if source.contains("# ARGC-BUILD {") {
        return;
    }
    let found = source.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#') && line.contains("--argc-eval")
    });
    if !found {
        output.push(Diagnostic::warning(
            source.lines().count().max(1),
            r#"missing `eval "$(argc --argc-eval "$0" "$@")"`"#.to_string(),
        ));
    }
}

// Bash ignores `set -e` inside a function called from `&&`, `||`, `if`, `while`, `until` or `!`
fn check_errexit(source: &str, events: &[Event], output: &mut Vec<Diagnostic>) {
    let mut cmd_fns: HashSet<&str> = HashSet::new();
    let mut in_cmd = false;
    for event in events {
        match &event.data {
            EventData::Cmd(_) => in_cmd = true,
            EventData::Func(name) => {
                if in_cmd {
                    cmd_fns.insert(name.as_str());
                }
                in_cmd = false;
            }
            _ => {}
        }
    }
    if cmd_fns.is_empty() {
        return;
    }
    let lines = tokenize_lines(source);
    if !lines.iter().any(|tokens| enables_errexit(tokens)) {
        return;
    }
    for (idx, tokens) in lines.iter().enumerate() {
        let mut names = vec![];
        collect_conditional_calls(tokens, &cmd_fns, &mut names);
        let mut seen = HashSet::new();
        for name in names {
            if !seen.insert(name) {
                continue;
            }
            let recipe = name
                .split("::")
                .map(|v| v.trim_end_matches('_'))
                .collect::<Vec<_>>()
                .join(" ");
            output.push(Diagnostic::warning(
                idx + 1,
                format!("errexit is ignored inside `{name}` when it is called from `&&`, `||`, `if`, `while` or `!`; run it as `argc {recipe}` instead"),
            ));
        }
    }
}

// Whether a line runs `set -e`, `set -o errexit` or a combined form like `set -euo pipefail`
fn enables_errexit(tokens: &[String]) -> bool {
    let mut cmd_pos = true;
    for (i, token) in tokens.iter().enumerate() {
        if is_list_operator(token) || (cmd_pos && is_keyword(token)) {
            cmd_pos = true;
            continue;
        }
        if cmd_pos && token == "set" {
            for (j, arg) in tokens[i + 1..].iter().enumerate() {
                if is_list_operator(arg) {
                    break;
                }
                if let Some(flags) = arg.strip_prefix('-') {
                    if !flags.chars().all(|c| c.is_ascii_alphabetic()) {
                        continue;
                    }
                    if flags.contains('e') {
                        return true;
                    }
                    if flags.ends_with('o')
                        && tokens.get(i + j + 2).map(|v| v.as_str()) == Some("errexit")
                    {
                        return true;
                    }
                }
            }
        }
        cmd_pos = false;
    }
    false
}

// Collect @cmd functions called in command position where errexit is ignored
fn collect_conditional_calls<'a>(
    tokens: &[String],
    cmd_fns: &HashSet<&'a str>,
    output: &mut Vec<&'a str>,
) {
    let mut cmd_pos = true;
    let mut condition = false;
    let mut negate = false;
    let mut fn_name_next = false;
    // `[[ ... ]]` or `(( ... ))`, where `&&` and `||` are not list operators
    let mut test_end: Option<&str> = None;
    // Functions in the current pipeline, flagged if it is followed by `&&` or `||`
    let mut pipeline: Vec<&'a str> = vec![];
    for (i, token) in tokens.iter().enumerate() {
        let token = token.as_str();
        if let Some(end) = test_end {
            if token.ends_with(end) {
                test_end = None;
            }
            continue;
        }
        match token {
            "&&" | "||" => {
                output.append(&mut pipeline);
                cmd_pos = true;
                negate = false;
                continue;
            }
            ";" | "&" | ";;" => {
                pipeline.clear();
                cmd_pos = true;
                negate = false;
                continue;
            }
            "|" | "|&" => {
                cmd_pos = true;
                continue;
            }
            _ => {}
        }
        if !cmd_pos {
            continue;
        }
        match token {
            "if" | "elif" | "while" | "until" => condition = true,
            "then" | "do" => {
                condition = false;
                negate = false;
            }
            "else" | "fi" | "done" | "esac" | "{" | "}" | "(" | ")" | "time" => {}
            "!" => negate = true,
            "function" => fn_name_next = true,
            "[[" => {
                test_end = Some("]]");
                cmd_pos = false;
            }
            _ if fn_name_next => fn_name_next = false,
            _ if is_assignment(token) => {}
            _ => {
                let word = token.trim_start_matches('(');
                // Function definition, e.g. `foo()`, `foo ()` or `foo() {`
                if word.contains("()") || tokens.get(i + 1).is_some_and(|v| v.starts_with("()")) {
                    continue;
                }
                // Case pattern, e.g. `foo)`
                if word.ends_with(')') {
                    continue;
                }
                cmd_pos = false;
                if word.starts_with("((") {
                    if !word.ends_with("))") {
                        test_end = Some("))");
                    }
                    continue;
                }
                if let Some(name) = cmd_fns.get(word) {
                    if condition || negate {
                        output.push(name);
                    } else {
                        pipeline.push(name);
                    }
                }
            }
        }
    }
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "if" | "then" | "elif" | "else" | "fi" | "do" | "done" | "while" | "until" | "{" | "}"
    )
}

fn is_list_operator(token: &str) -> bool {
    matches!(token, "&&" | "||" | ";" | "&" | ";;" | "|" | "|&")
}

fn is_assignment(token: &str) -> bool {
    match token.split_once('=') {
        Some((name, _)) => is_shell_var_name(name.trim_end_matches('+')),
        None => false,
    }
}

fn is_shell_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Split each line into shell words and operators, dropping comments, the content of
/// quoted strings and heredoc bodies. A quoted string is kept as a bare pair of quotes.
fn tokenize_lines(source: &str) -> Vec<Vec<String>> {
    let mut output = vec![];
    let mut quote: Option<char> = None;
    let mut heredoc: Option<String> = None;
    for line in source.lines() {
        let mut tokens = vec![];
        if let Some(delimiter) = &heredoc {
            if line.trim() == delimiter {
                heredoc = None;
            }
            output.push(tokens);
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut word = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if let Some(q) = quote {
                if c == '\\' && q != '\'' {
                    i += 1;
                } else if c == q {
                    word.push(q);
                    quote = None;
                }
                i += 1;
                continue;
            }
            match c {
                '\'' | '"' | '`' => {
                    word.push(c);
                    quote = Some(c);
                }
                '\\' => {
                    // Keep the escaped char from being read as an operator
                    word.push('\\');
                    i += 1;
                }
                '#' if word.is_empty() => break,
                ' ' | '\t' => push_word(&mut tokens, &mut word),
                ';' | '|' => {
                    push_word(&mut tokens, &mut word);
                    let next = chars.get(i + 1).copied();
                    if next == Some(c) || (c == '|' && next == Some('&')) {
                        tokens.push(format!("{c}{}", next.unwrap()));
                        i += 1;
                    } else {
                        tokens.push(c.to_string());
                    }
                }
                '&' => {
                    let next = chars.get(i + 1).copied();
                    let prev = if i > 0 { Some(chars[i - 1]) } else { None };
                    if next == Some('&') {
                        push_word(&mut tokens, &mut word);
                        tokens.push("&&".to_string());
                        i += 1;
                    } else if matches!(prev, Some('>') | Some('<')) || next == Some('>') {
                        // Redirection, e.g. `>&2` or `&>/dev/null`
                        word.push(c);
                    } else {
                        push_word(&mut tokens, &mut word);
                        tokens.push("&".to_string());
                    }
                }
                '<' if chars.get(i + 1) == Some(&'<')
                    && chars.get(i + 2) != Some(&'<')
                    && !word.ends_with('<') =>
                {
                    let rest: String = chars[i + 2..].iter().collect();
                    let delimiter: String = rest
                        .trim_start_matches('-')
                        .trim_start()
                        .chars()
                        .take_while(|c| !c.is_whitespace() && !matches!(c, ';' | '|' | '&' | ')'))
                        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
                        .collect();
                    // Skip shifts like `$((1<<2))`
                    if delimiter.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                        heredoc = Some(delimiter);
                    }
                    word.push_str("<<");
                    i += 1;
                }
                _ => word.push(c),
            }
            i += 1;
        }
        if quote.is_none() {
            push_word(&mut tokens, &mut word);
        }
        output.push(tokens);
    }
    output
}

fn push_word(tokens: &mut Vec<String>, word: &mut String) {
    if !word.is_empty() {
        tokens.push(std::mem::take(word));
    }
}

fn suggest_meta_key(key: &str) -> Option<&'static str> {
    META_KEYS
        .iter()
        .map(|v| (edit_distance(key, v), *v))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, v)| v)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut curr = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == *cb { 0 } else { 1 };
            curr.push((prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1));
        }
        prev = curr;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edit_distance() {
        assert_eq!(edit_distance("require-tool", "require-tools"), 1);
        assert_eq!(edit_distance("dotenv", "dotenv"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn test_tokenize_lines() {
        let source = r#"a && echo "x && y" # a || b
b() { argc a >&2 || c; }
cat <<'EOF'
a && b
EOF
echo 'multi
a && b' ; d <<< "$x"
echo ${#x[@]} $((1<<2))"#;
        let expect: Vec<Vec<&str>> = vec![
            vec!["a", "&&", "echo", r#""""#],
            vec!["b()", "{", "argc", "a", ">&2", "||", "c", ";", "}"],
            vec!["cat", "<<''"],
            vec![],
            vec![],
            vec!["echo"],
            vec!["'", ";", "d", "<<<", r#""""#],
            vec!["echo", "${#x[@]}", "$((1<<2))"],
        ];
        assert_eq!(tokenize_lines(source), expect);
    }
}
