#[test]
fn dotenv() {
    let script = r###"
# @meta dotenv
"###;
    snapshot!(script, &["prog"]);
}

#[test]
fn dotenv_custom_path() {
    let script = r###"
# @meta dotenv .env.local
"###;
    snapshot!(script, &["prog"]);
}

#[test]
fn binname() {
    let script = r###"
# @meta binname test-binname
"###;
    snapshot!(script, &["prog", "-h"]);
}

#[test]
fn require_bash() {
    let script = r###"
# @meta require-bash 3.0

# @cmd
foo() { :; }
"###;
    snapshot_multi!(
        script,
        [
            vec!["prog", "foo"],
            vec!["prog", "-h"],
            vec!["prog", "--version"]
        ]
    );
}

#[test]
fn require_bash_unsatisfied() {
    let script = r###"
# @meta require-bash 99

# @cmd
foo() { echo foo; }
"###;
    let args: Vec<String> = ["prog", "foo"].iter().map(|v| v.to_string()).collect();
    let values = argc::eval(argc::NativeRuntime, script, &args, None, None).unwrap();
    insta::assert_snapshot!(argc::ArgcValue::to_bash(&values));

    let (script_path, _, script_file) = crate::fixtures::create_argc_script(script, "script.sh");
    let build_script_dir = crate::fixtures::tmpdir();
    let build_script_path = crate::fixtures::build_script(&build_script_dir, script, "prog");
    let build_script_path = build_script_path.display().to_string();
    for path in [script_path.as_str(), build_script_path.as_str()] {
        // The found version depends on the bash running the tests
        let output = run_bash(path, &["foo"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "");
        assert!(String::from_utf8_lossy(&output.stderr)
            .starts_with("error: bash 99+ is required, found "));

        // Like require-tools, help and version are not checked
        let output = run_bash(path, &["-h"]);
        assert!(output.status.success());
        let help = [output.stdout, output.stderr].concat();
        assert!(String::from_utf8_lossy(&help).contains("USAGE:"));
        let output = run_bash(path, &["--version"]);
        assert!(output.status.success());
    }
    script_file.close().unwrap();
}

fn run_bash(script_path: &str, args: &[&str]) -> std::process::Output {
    use argc::Runtime;
    let shell_path = argc::NativeRuntime.shell_path().unwrap();
    std::process::Command::new(shell_path)
        .arg(script_path)
        .args(args)
        .env("PATH", crate::fixtures::get_path_env_var())
        .output()
        .unwrap()
}

#[test]
fn group_commands() {
    let script = r###"
# @meta group-commands

# @cmd Build the project
build() { :; }

# @cmd Deploy ace
ace:deploy() { :; }

# @cmd Install tools
# @alias ace@tools
ace@install_tools() { :; }

# @cmd Lint docs
docs.lint() { :; }

# @cmd Lint ace
ace.lint() { :; }

# @cmd Run tests
# @alias t
run_tests() { :; }
"###;
    snapshot_multi!(
        script,
        [
            vec!["prog", "-h"],
            vec!["prog", "ace@install-tools"],
            vec!["prog", "ace@tools"],
            vec!["prog", "run-tests"],
        ]
    );
}

#[test]
fn group_commands_namespaced_only() {
    let script = r###"
# @meta group-commands

# @cmd
db:migrate() { :; }

# @cmd
db:seed() { :; }
"###;
    snapshot!(script, &["prog", "-h"]);
}

#[test]
fn group_commands_disabled() {
    let script = r###"
# @cmd Build the project
build() { :; }

# @cmd Deploy ace
ace:deploy() { :; }

# @cmd Install tools
# @alias ace@tools
ace@install_tools() { :; }

# @cmd Lint docs
docs.lint() { :; }

# @cmd Lint ace
ace.lint() { :; }

# @cmd Run tests
# @alias t
run_tests() { :; }
"###;
    snapshot!(script, &["prog", "-h"]);
}
