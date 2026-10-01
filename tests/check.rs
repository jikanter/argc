fn lint(source: &str) -> Vec<String> {
    argc::check(source).iter().map(|v| v.to_string()).collect()
}

const EVAL_LINE: &str = r#"eval "$(argc --argc-eval "$0" "$@")""#;

fn errexit_script(body: &str) -> String {
    format!(
        r#"set -e

# @cmd
a() {{ echo start; false; echo "should not print"; }}

# @cmd
ns:b.c@d-e() {{ :; }}

# @cmd
foo() {{ :; }}

# @cmd
foo::bar() {{ :; }}

# @cmd
{body}

{EVAL_LINE}
"#
    )
}

fn errexit_warning(line: usize, name: &str, recipe: &str) -> String {
    format!("{line}: warning: errexit is ignored inside `{name}` when it is called from `&&`, `||`, `if`, `while` or `!`; run it as `argc {recipe}` instead")
}

#[test]
fn clean() {
    let script = format!(
        r#"set -euo pipefail

# @meta require-bash 4.4
# @meta group-commands
# @env FOO_BAR

# @cmd
# @alias b
build() {{
    echo build
}}

{EVAL_LINE}
"#
    );
    assert_eq!(lint(&script), Vec::<String>::new());
}

#[test]
fn build_error() {
    let script = format!("\n# @meta require-bash 4.x\n{EVAL_LINE}\n");
    assert_eq!(
        lint(&script),
        ["2: error: @meta invalid require-bash value `4.x`, expected major[.minor[.patch]]"]
    );
    let script = format!("\n\n# @baz\n{EVAL_LINE}\n");
    assert_eq!(lint(&script), ["3: error: @baz is unknown tag"]);
}

#[test]
fn unknown_meta() {
    let script = format!("# @meta require-tool gcloud\n# @meta foo\n{EVAL_LINE}\n");
    assert_eq!(
        lint(&script),
        [
            "1: warning: unknown @meta key `require-tool`, did you mean `require-tools`?",
            "2: warning: unknown @meta key `foo`",
        ]
    );
}

#[test]
fn invalid_env_name() {
    let script = format!("# @env require-tools gcloud\n# @env foo-bar\n{EVAL_LINE}\n");
    assert_eq!(
        lint(&script),
        [
            "1: warning: @env name `require-tools` is not a valid shell variable name, did you mean `@meta require-tools`?",
            "2: warning: @env name `foo-bar` is not a valid shell variable name",
        ]
    );
}

#[test]
fn alias_same_as_name() {
    let script = format!("# @cmd\n# @alias bar\nbar_() {{ :; }}\n{EVAL_LINE}\n");
    assert_eq!(
        lint(&script),
        ["2: warning: @alias `bar` is the same as its command's name"]
    );
    // Already rejected when building the command, reported once as an error
    let script = format!("# @cmd\n# @alias foo\nfoo() {{ :; }}\n{EVAL_LINE}\n");
    assert_eq!(
        lint(&script),
        ["2: error: @alias conflicts with cmd or alias at line 3"]
    );
}

#[test]
fn missing_eval_line() {
    let script = "# @cmd\nfoo() { :; }\n# eval \"$(argc --argc-eval \"$0\" \"$@\")\"\n";
    assert_eq!(
        lint(script),
        [r#"3: warning: missing `eval "$(argc --argc-eval "$0" "$@")"`"#]
    );
}

#[test]
fn errexit_trap() {
    let body = r#"b() {
    a && echo "a ok"
    a || echo "a failed"
    if a; then echo ok; fi
    if ! a; then echo failed; fi
    ! a
    while a; do break; done
    until a; do break; done
    x=1 a && true
    false || ns:b.c@d-e arg || true
    foo::bar | cat && true
}"#;
    let script = errexit_script(body);
    assert_eq!(
        lint(&script),
        [
            errexit_warning(17, "a", "a"),
            errexit_warning(18, "a", "a"),
            errexit_warning(19, "a", "a"),
            errexit_warning(20, "a", "a"),
            errexit_warning(21, "a", "a"),
            errexit_warning(22, "a", "a"),
            errexit_warning(23, "a", "a"),
            errexit_warning(24, "a", "a"),
            errexit_warning(25, "ns:b.c@d-e", "ns:b.c@d-e"),
            errexit_warning(26, "foo::bar", "foo bar"),
        ]
    );
}

#[test]
fn errexit_trap_negative() {
    let body = r#"b() {
    a
    a; foo
    argc a && echo "a ok"
    "$0" a && echo "a ok"
    $0 a || exit 1
    echo a && echo foo
    true && a
    echo "a && b" # a && b
    foo-bar && true
    ns:b && true
    cat <<EOF
a && b
EOF
    if [[ -n "$x" && a ]]; then :; fi
    case "$1" in
    a) echo ;;
    esac
}"#;
    let script = errexit_script(body);
    assert_eq!(lint(&script), Vec::<String>::new());
}

#[test]
fn errexit_variants() {
    for set in [
        "set -e",
        "set -eu",
        "set -euo pipefail",
        "set -o errexit",
        "if true; then set -e; fi",
    ] {
        let script = errexit_script("b() { a && :; }").replace("set -e\n", &format!("{set}\n"));
        assert_eq!(lint(&script), [errexit_warning(16, "a", "a")], "{set}");
    }
    for set in ["", "set -u", "set -o pipefail", "# set -e"] {
        let script = errexit_script("b() { a && :; }").replace("set -e\n", &format!("{set}\n"));
        assert_eq!(lint(&script), Vec::<String>::new(), "{set}");
    }
}
