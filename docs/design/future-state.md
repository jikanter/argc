# argc: design and future-state analysis

Status: accepted 2026-10-01. Scope: the `jk` fork of argc 1.24.0.
Decisions: build B0, B2, B3 and B4; defer B1 (recipe dependencies); do not
integrate `just`.

The goal is a short list of extensions that make argc a well-rounded
build tool, command runner and bash extension, and an explicit list of
things not to build. Each item is ranked by evidence from real Argcfiles,
not by what other tools happen to have.

## 1. Where argc is today

argc has two jobs and does the first one very well.

| Job | State |
| :-- | :---- |
| Bash CLI framework: comment tags become parsing, validation, help, completions (7 shells), man pages, `--argc-build` standalone scripts, `--argc-export` JSON | Mature. Nothing in `just`, `make` or `task` comes close on argument handling. |
| Command runner: find `Argcfile.sh` up the tree, cd to it, run a recipe | Thin, and its one piece of advice on composing recipes ("call them sequentially within other functions") leads into a bash trap. See section 2. |

How a run works, because every extension has to fit this shape:

1. `argc build` finds the Argcfile and execs bash on it (`src/bin/argc/main.rs`, `run_command`).
2. The script's last line calls back: `eval "$(argc --argc-eval "$0" "$@")"`.
3. argc parses the script's comment tags, matches the arguments, and prints bash: variable assignments, hooks, then the recipe function call (`ArgcValue::to_bash`).

Three constraints follow from that:

- **Two code paths.** `--argc-eval` computes in Rust; `--argc-build` emits a
  pure-bash script that must behave the same without argc installed
  (`src/build.rs`). Any runtime feature is implemented twice or is declared
  runner-only.
- **`@meta` is the only compatible extension point.** Stock argc accepts
  unknown `@meta` keys and passes them through `--argc-export`, but rejects
  unknown tags outright (`@dep(line 4) is unknown tag`). New `@`-tags would
  make an Argcfile unusable on stock argc; new `@meta` keys do not.
- **The overhead budget is small and already met.** Measured on a 721-line,
  41-command Argcfile: `argc --argc-eval` takes about 5 ms, bash startup
  about 6 ms. A full `argc <recipe>` self-call costs roughly 11 ms.

Upstream (`sigoden/argc`) is active: v1.24.0 shipped 2026-05-20 and about ten
PRs merged in June 2026. The fork should stay rebasable, which favors small,
self-contained changes that could be offered upstream.

## 2. Evidence: what real Argcfiles do

Five Argcfiles from projects that use argc daily were surveyed (9 to 794
lines, 120 recipes in total), plus one justfile from a project that chose
`just` instead.

| Observation | Count | What it says |
| :---------- | :---- | :----------- |
| Recipes that run other recipes by spawning `argc <recipe>` | 45 call sites in 4 files | Recipes compose as subprocesses, not as function calls. About half the call sites read closely are "run X first"; the rest are mid-body calls or dynamic dispatch (`argc "chat-$provider"`), which a declared dependency could not express. |
| A commit titled "fix argc (set -e does not work with argc)" that removes `set -e` | 1 | The reason for the self-spawns. See below. |
| A dependency reached twice in one run (a diamond) | 0 | Nothing for a dependency graph to deduplicate. |
| The justfile uses recipe dependencies | 3 recipes | All linear with a single parent. Plain sequencing covers them. |
| Hand-written "already done?" guards that gate a recipe call, e.g. `[[ ! -d .venv ]] && argc setup` | 2, both the same check in 1 file | Weak signal. The other 11 existence tests found are input validation. No timestamp or hash comparison appears anywhere. |
| One flat help listing of 41 commands, every one carrying a hand-written `ns:name` alias | 1 file, 41 aliases | Help does not scale with namespaced recipes. |
| A loop running `argc --argc-run <dir>/Argcfile.sh build` over 5 nested Argcfiles | 1 file | Modules are wanted, but only one project needs them so far. |
| A hand-written 8-line bash 4.4 version gate (macOS ships bash 3.2) | 1 file | A cheap declarative guard is missing. |
| `# @env require-tools <tool>`, a typo for `@meta`, accepted silently | 1 | There is no lint. Typos in tags fail silently. |
| Hand-rolled dry-run via an env var | 2 files | Wanted occasionally; cannot be solved generically. |
| Uses of `--argc-parallel` | 0 | Parallel execution is not a present need. |
| Confirmation prompts | 0 | Not a present need. |

### The `set -e` trap

This is the most important finding, because it explains the largest pattern.

```sh
set -e
# @cmd
a() { echo start; false; echo "should not print"; }
# @cmd
b() { a && echo "a ok"; }
```

`argc a` stops at `false` and exits 1, as expected. `argc b` prints
`should not print` and `a ok`, and exits 0. Bash ignores errexit inside any
function called from an `&&` or `||` list, an `if`, `while` or `until`
condition, or after `!`. This is bash behavior, not an argc bug, but argc's
documented way to compose recipes ("call the function") walks straight
into it.

Spawning `argc a && argc b` avoids the trap because each recipe gets its
own process and a real exit status. It also gives each recipe its own
argument defaults and `@env` validation. At 11 ms per call it is cheap.

argc cannot change how bash treats errexit. What it can do is say so in the
runner documentation, recommend the self-spawn form, and warn when a script
falls into the trap. That is B0.

## 3. Design principles

1. **Bash owns recipe bodies and how they compose. argc owns argument
   parsing, environment checks, help and diagnostics.** No expression
   language, no templating, no variables DSL. If something can be written
   as three lines of bash, it stays bash.
2. **Extend through `@meta` only.** Fork Argcfiles keep parsing on stock argc.
3. **Recipes compose as processes.** `argc <recipe>` from inside another
   recipe is the supported pattern. It has correct failure semantics, and it
   is the precondition for a dependency graph or parallel execution if those
   are ever built.
4. **Every feature states its `--argc-build` behavior**: emitted as bash,
   or runner-only.
5. **Evidence first.** A feature with no occurrence in section 2 is not in
   the build list, however standard it is elsewhere.

## 4. Future state

What a project Argcfile looks like once the build list lands:

```sh
#!/usr/bin/env bash
set -e
# @meta require-bash 4.4
# @meta require-tools uv jq
# @meta group-commands

# @cmd Create the virtualenv
env:setup() { [[ -d .venv ]] || { uv venv && uv sync; }; }

# @cmd Run the test suite
test:unit() {
    argc env:setup
    uv run pytest
}

# @cmd Verify invariants
check:tags() {
    argc env:setup
    uv run python tools/check.py
}

# @cmd Everything CI runs
ci() { argc check:tags && argc test:unit; }

eval "$(argc --argc-eval "$0" "$@")"
```

`argc ci` stops at the first failing recipe with that recipe's exit code.
`argc --help` shows `CHECK COMMANDS`, `ENV COMMANDS` and `TEST COMMANDS`
sections instead of one list. On bash 3.2 the script stops with a clear
message before anything runs. `argc --argc-check` reports a misspelled
`@meta` key, and would warn if `ci` were written as
`check:tags && test:unit`.

## 5. Build list

Sizes: XS under 50 lines, S under 200, M under 600, including tests.

### B0. Bless the self-spawn pattern and flag the `set -e` trap (S)

- **Evidence:** 45 self-spawn call sites; the `set -e` commit.
- **Docs:** `docs/command-runner.md` recommends `argc <recipe>` for running
  one recipe from another and explains the trap, replacing the advice to
  call recipe functions directly.
- **Lint:** `--argc-check` (B2) warns when a script enables errexit and
  calls a `@cmd` function directly from an `&&`/`||` list, an `if`, `while`
  or `until` condition, or after `!`. The check is a line-based heuristic
  and prefers a missed case to a false positive. A plain statement call
  (`a` alone on a line) keeps errexit and is not flagged.
- **`--argc-build`:** no runtime change.

### B2. Lint: `argc --argc-check` (S)

- **Evidence:** the silently accepted `@env require-tools` typo.
- **Checks:** unknown `@meta` key with a did-you-mean suggestion; `@env`
  name that is not a valid variable name; alias equal to its own command
  name; missing `--argc-eval` line; the errexit trap (B0); everything the
  parser already rejects.
- **Output:** `<path>:<line>: <error|warning>: <message>` on stderr. Exit 1
  on any error; warnings alone exit 0.
- **`--argc-build`:** no runtime change.

### B3. Grouped help: `@meta group-commands` (S)

- **Evidence:** the 41-command flat listing.
- **Semantics:** root-level opt-in. Recipes named `ns:name`, `ns.name` or
  `ns@name` are listed under a `<NS> COMMANDS:` section per namespace, in
  order of first appearance; un-namespaced recipes stay under `COMMANDS:`.
  Aliases that argc generated itself (the `-`/`_` variants) are not listed.
  Without the key, help is unchanged.
- **Where:** `render_subcommands` in `src/command/mod.rs`. Display only;
  matching and completion are unchanged. `--argc-build` gets it for free
  because help text is rendered at build time.

### B4. Bash version guard: `@meta require-bash <version>` (XS)

- **Evidence:** the hand-written 4.4 gate; macOS `/bin/bash` is 3.2.
- **Semantics:** compare against `BASH_VERSINFO` before anything else runs;
  on failure print the found and required versions and exit. The check
  itself is valid bash 3.2. Emitted the same way in eval and build output,
  alongside `require-tools`.

## 6. Deferred

| | Feature | Trigger to build | Note |
| :-- | :-- | :-- | :-- |
| B1 | Recipe dependencies: `@meta dep <recipe>...` | The first diamond, or the first need for parallel steps or a printed plan | Today it would be declarative sugar over `argc a && argc b`, which works. Sketch below. Size M. |
| L1 | Modules: `@meta mount <name> <path>` so `argc <name> <recipe>` delegates to a nested Argcfile with help and completion | A second project with nested Argcfiles | Reuses the external-subcommand machinery added in 1.24 (#407). Size M. |
| L2 | Parallel dependencies with streamed, line-prefixed output, `--jobs`, fail-fast | A dependency graph whose wall time hurts | Needs B1 first. Today `parallel.rs` buffers all output until every job ends and is hard-wired to the CPU count; that is worth fixing only when something uses it. |
| L3 | `argc --argc-plan <recipe>`: print the resolved order | After B1 | Close to free once the graph exists. This is the only honest generic "dry run": argc cannot know what a bash body will do. |
| L4 | Skip guard: `@meta creates <path>`, skipping a dependency whose output already exists | If guards multiply | Two occurrences today, and `[[ -d .venv ]] \|\| ...` inside the recipe is one line of bash, so principle 1 says wait. No `sources`/`generates` timestamp model: nothing surveyed needs it. |
| L5 | `ARGC_DRY_RUN` convention plus a `--dry-run` flag that sets it | If more recipes hand-roll it | Convention only; bodies still implement it. |

### B1 sketch, for when it is revived

- **Syntax:** `# @meta dep <recipe>...`, one or more lines per recipe.
- **Semantics:** on the top-level invocation argc resolves the full graph,
  orders it topologically, and runs each dependency once as a subprocess
  (`"$0" <recipe>`) before the recipe body. The first failure stops the run
  and its exit code is returned. Child processes receive an env marker so
  they do not resolve dependencies again.
- **Errors at parse time:** unknown recipe name, cycle (report the path).
- **`--argc-build`:** supported. The graph is static, so the ordered list
  per recipe is computed at build time and emitted as plain bash.
- **Open before building:** whether dependencies take arguments. Names-only
  keeps `@meta dep a b` unambiguous; one dependency per line with arguments
  (`@meta dep build --release`) conflicts with the list form.
- **Risk:** an Argcfile using `@meta dep` would run on stock argc without
  its dependencies and without an error.

## 7. Not building

| Idea | Source | Why not |
| :--- | :----- | :------ |
| Using `just` for the dependency graph, with recipe bodies calling `argc` | just | Two files and two entry points. `just`'s help and completion cannot see argc's parameters, which gives up argc's best feature. Deduplication only holds inside one `just` run. It solves what `argc a && argc b` already solves. |
| Parse cache | uv | Measured 5 ms on the largest file. Nothing to win. |
| Shebang script mode, removing the `eval` line | uv | The line is one line, costs nothing, and is what lets the script run directly and through `--argc-build`. |
| Tool version pinning and a lockfile | uv | `--version` output is not uniform across tools, so this turns into a per-tool parser. `mise` already does it and is already installed here. Keep `require-tools` as a presence check. |
| Tool provisioning | uv | Same: `mise` territory. |
| Workspaces, `--all` | uv | L1 covers the one real case. |
| Variables and expressions (`:=`, `{{ }}`) | just | Bash already has variables. This is the line principle 1 draws. |
| `[confirm]` prompts | just | Zero occurrences; `read -p` is one line of bash. |
| OS-specific recipe attributes | just | `ARGC_OS` and a `case` statement already do it. |
| Watch mode | task | `watchexec -- argc build` composes fine. |
| Interactive chooser | just | Completions already cover discovery. |
| New `@`-tags | — | Breaks stock argc (section 1). |
| An in-process dependency model | — | Calling recipe functions directly is what the `set -e` trap punishes. |

## 8. Sequence

1. **B0, B2, B3, B4.** Independent of each other, except that B0's warning
   lives inside B2. Each is small enough to offer upstream on its own.
2. Migrate one real Argcfile (the 41-command one is the best test): turn on
   `group-commands`, replace the bash gate with `require-bash`, run
   `--argc-check` and fix what it reports.
3. Revisit section 6 with what the migration shows. A diamond or a slow
   sequential chain is the signal for B1.

Changes carry snapshot tests in the existing `tests/` layout
(`tests/meta.rs`, `tests/snapshots`).

## 9. Risks and open questions

- **Stock argc ignores the new keys.** An Argcfile using `group-commands`
  or `require-bash` still runs on stock argc; it just shows flat help and
  skips the version guard. That is a soft downgrade, not a failure. Note
  that the `argc` currently on `PATH` here reports `1.24.0`, not
  `1.24.0-jk1`, so the fork build is not the one in use yet.
- **The errexit warning is a heuristic.** It reads lines, not a bash syntax
  tree, so multi-line conditions and calls built from variables are missed.
- **Why the `ns@name` function plus `ns:name` alias pattern?** Bash accepts
  `:` in function names and argc parses them. If something else forced the
  `@` form, B3 should account for it; if not, the aliases can simply go.
- **Upstream.** B2, B3 and B4 are uncontroversial and worth offering. The
  documentation change in B0 is too.
