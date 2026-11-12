# grep (Rust)

A fast, small `grep` written in Rust. It follows the command line of GNU grep but is **not** a line-by-line port of it: matching is done by the [`regex`](https://crates.io/crates/regex) crate (and [`pcre2`](https://crates.io/crates/pcre2) for `-P`).

## Build

```sh
cargo build --release      # binary: target/release/grep(.exe)
cargo test --release       # integration tests in tests/cli.rs
```

`-P` compiles the PCRE2 C library, so you need a C toolchain (on Windows, the MSVC build tools; build from PowerShell or a Developer prompt, because Git's `link.exe` can shadow the MSVC linker).

## Usage

```
grep [OPTION]... PATTERNS [FILE]...
```

With no `FILE` (or `-`), standard input is read. With `-r` and no `FILE`, the current directory is searched and names are printed without a `./` prefix. Run `grep --help` for the full option list.

Exit status: `0` a line was selected, `1` none was, `2` an error occurred (`-q` with a match exits `0` even if an error happened).

### Options

| Area | Options |
|---|---|
| Pattern syntax | `-G` basic (default), `-E` extended, `-F` fixed strings, `-P` Perl (PCRE2) |
| Patterns | `-e PATTERNS`, `-f FILE`, newline-separated patterns |
| Matching | `-i`, `-y`, `--no-ignore-case`, `-w`, `-x`, `-v` |
| Output | `-c`, `-l`, `-L`, `-m NUM`, `-o`, `-q`, `-s`, `-n`, `-H`, `-h`, `--color[=WHEN]` |
| Context | `-A NUM`, `-B NUM`, `-C NUM`, `-NUM` |
| Binary files | `-a`, `-I`, `--binary-files=binary\|text\|without-match` |
| Files | `-r`, `-R`, `--include=GLOB`, `--exclude=GLOB`, `--exclude-from=FILE`, `--exclude-dir=GLOB` |
| Separators | `-z`/`--null-data` (records end in NUL), `-Z`/`--null` (NUL after file names) |
| Misc | `--line-buffered` (flush every line; streams stdin without context options), `--help`, `-V`/`--version`; `-U` and `-T` are accepted and ignored |

Long forms (`--ignore-case`, `--max-count=N`, …) exist for each option; see `--help`.

## Performance

- Without `-v` or context options, the whole buffer is searched at once and the program jumps from match to match instead of testing each line.
- Files of 128 KB or more are memory-mapped.
- With several files and no context options (`-A/-B/-C`), files are searched on multiple threads; output order is always the same as a single-threaded run.
- Release profile uses fat LTO, one codegen unit and `panic = "abort"`.

## Differences from GNU grep

- `-G` is converted to the regex-crate syntax with a small rewrite; **back-references are unsupported** outside `-P`.
- `-w` (non-`-P`) uses `\b`, which differs when a pattern starts or ends with a non-word character. With `-P` it uses `(?<!\w)…(?!\w)`.
- `-P` matches are byte-based; `-R` does not follow symlinks (same as `-r`).
- Colors are the fixed GNU defaults; `GREP_COLORS` is not read. With `--color=auto` ANSI support of the console is not enabled on Windows.
- A file is binary if it has a NUL byte in its first 8000 bytes. With `-z`, files are never treated as binary.
- Glob matching (`--include` etc.) is byte-wise, so non-ASCII characters in bracket classes won't match, and patterns with many `*` can be slow. A glob matches the whole path or any suffix after a `/` or `\`.
- Memory-mapped files that are truncated by another process during the search can crash the program.
- Not implemented: `-b`, `-T` (ignored), `-U` (ignored), `-d`/`-D`, `--label`, `--no-group-separator`, `--group-separator`, `GREP_COLORS`, locale handling.
- `-L` sets the exit status from matched lines, not from listed files.
- Parts of the `dfa.c` / `kwset` machinery of GNU grep are intentionally not reproduced.

## Layout

```
src/main.rs     argument parsing, matching, output
tests/cli.rs    black-box tests, at least one per option
```
