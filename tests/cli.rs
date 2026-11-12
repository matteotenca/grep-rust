//! Black-box tests: one or more per CLI option. Run with `cargo test`.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct R { out: String, err: String, code: i32 }

/// Fresh directory under the system temp dir holding the given files.
fn fx(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("grep-rust-tests-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for (f, c) in files {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
    d
}

fn run_in(dir: &Path, args: &[&str], stdin: &[u8]) -> R {
    let mut c = Command::new(env!("CARGO_BIN_EXE_grep"))
        .args(args).current_dir(dir)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    let _ = c.stdin.take().unwrap().write_all(stdin); // early exit (-q) may close the pipe
    let o = c.wait_with_output().unwrap();
    R {
        out: String::from_utf8_lossy(&o.stdout).replace('\\', "/"),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
        code: o.status.code().unwrap(),
    }
}

/// Run on stdin only, in an empty directory.
fn g(args: &[&str], stdin: &str) -> R { run_in(&std::env::temp_dir(), args, stdin.as_bytes()) }

const S: &str = "foo\nbar\nbaz foo\nqux\nzzz\nfoo2\n";
const NINE: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";

#[test]
fn plain_match_and_exit_codes() {
    let r = g(&["foo"], S);
    assert_eq!((r.out.as_str(), r.code), ("foo\nbaz foo\nfoo2\n", 0));
    let r = g(&["nomatch"], S);
    assert_eq!((r.out.as_str(), r.code), ("", 1));
    assert_eq!(g(&["foo", "-"], S).out, "foo\nbaz foo\nfoo2\n"); // "-" is stdin
}

#[test]
fn basic_regexp_g() {
    assert_eq!(g(&["-G", r"\(bar\|qux\)"], S).out, "bar\nqux\n");
    assert_eq!(g(&["-G", "fo+"], "fo+\nfoo\n").out, "fo+\n"); // + is literal in BRE
    assert_eq!(g(&["--basic-regexp", "bar"], S).out, "bar\n");
}

#[test]
fn extended_regexp_e() {
    assert_eq!(g(&["-E", "ba(r|z)"], S).out, "bar\nbaz foo\n");
    assert_eq!(g(&["--extended-regexp", "fo+2"], S).out, "foo2\n");
}

#[test]
fn fixed_strings_f() {
    assert_eq!(g(&["-F", "a.c"], "a.c\nabc\n").out, "a.c\n");
    assert_eq!(g(&["--fixed-strings", "a.c"], "a.c\nabc\n").out, "a.c\n");
}

#[test]
fn perl_regexp_p() {
    assert_eq!(g(&["-P", "foo(?=2)"], S).out, "foo2\n");
    assert_eq!(g(&["-oP", "(?<=baz )foo"], S).out, "foo\n");
    assert_eq!(g(&["-P", r"(z)\1"], S).out, "zzz\n");
    assert_eq!(g(&["--perl-regexp", r"\d"], "a1\nb\n").out, "a1\n");
    assert_eq!(g(&["-wP", "foo"], "foo bar\nfoobar\n").out, "foo bar\n");
    assert_eq!(g(&["-P", "("], "x\n").code, 2); // bad pattern
}

#[test]
fn pattern_options_e_regexp_and_f() {
    assert_eq!(g(&["-e", "bar", "-e", "qux"], S).out, "bar\nqux\n");
    assert_eq!(g(&["--regexp=bar"], S).out, "bar\n");
    assert_eq!(g(&["bar\nqux"], S).out, "bar\nqux\n"); // newline separates patterns
    let d = fx("pats", &[("pats.txt", b"bar\nqux\n"), ("in.txt", S.as_bytes())]);
    assert_eq!(run_in(&d, &["-f", "pats.txt", "in.txt"], b"").out, "bar\nqux\n");
    assert_eq!(run_in(&d, &["--file=pats.txt", "in.txt"], b"").out, "bar\nqux\n");
    assert_eq!(run_in(&d, &["-f", "-", "in.txt"], b"qux\n").out, "qux\n"); // patterns from stdin
    assert_eq!(run_in(&d, &["-f", "missing.txt"], b"").code, 2);
}

#[test]
fn ignore_case_i_y() {
    assert_eq!(g(&["-i", "foo"], "Foo\nfoo\nFOO\nbar\n").out, "Foo\nfoo\nFOO\n");
    assert_eq!(g(&["-y", "foo"], "Foo\nbar\n").out, "Foo\n");
    assert_eq!(g(&["--ignore-case", "foo"], "Foo\nbar\n").out, "Foo\n");
    assert_eq!(g(&["-i", "--no-ignore-case", "foo"], "Foo\nfoo\n").out, "foo\n");
}

#[test]
fn word_regexp_w() {
    let i = "foo\nfoobar\nbar foo baz\nfoo_x\n";
    assert_eq!(g(&["-w", "foo"], i).out, "foo\nbar foo baz\n");
    assert_eq!(g(&["--word-regexp", "foo"], i).out, "foo\nbar foo baz\n");
}

#[test]
fn line_regexp_x() {
    let i = "foo\nfoobar\nbar foo\n";
    assert_eq!(g(&["-x", "foo"], i).out, "foo\n");
    assert_eq!(g(&["--line-regexp", "foo"], i).out, "foo\n");
    assert_eq!(g(&["-xE", "foo|bar foo"], i).out, "foo\nbar foo\n");
}

#[test]
fn invert_match_v() {
    assert_eq!(g(&["-v", "foo"], S).out, "bar\nqux\nzzz\n");
    assert_eq!(g(&["--invert-match", "foo"], S).out, "bar\nqux\nzzz\n");
    assert_eq!(g(&["-vn", "foo"], S).out, "2:bar\n4:qux\n5:zzz\n");
}

#[test]
fn count_c() {
    assert_eq!(g(&["-c", "foo"], S).out, "3\n");
    assert_eq!(g(&["--count", "foo"], S).out, "3\n");
    assert_eq!(g(&["-vc", "foo"], S).out, "3\n");
    assert_eq!(g(&["-c", "nomatch"], S).out, "0\n");
    assert_eq!(g(&["-c", "-m", "2", "foo"], S).out, "2\n");
    let d = fx("count", &[("a.txt", S.as_bytes()), ("b.txt", b"x\n")]);
    assert_eq!(run_in(&d, &["-c", "foo", "a.txt", "b.txt"], b"").out, "a.txt:3\nb.txt:0\n");
}

#[test]
fn files_with_and_without_match_l_cap_l() {
    let d = fx("lL", &[("a.txt", S.as_bytes()), ("b.txt", b"nothing\n")]);
    assert_eq!(run_in(&d, &["-l", "foo", "a.txt", "b.txt"], b"").out, "a.txt\n");
    assert_eq!(run_in(&d, &["--files-with-matches", "foo", "a.txt", "b.txt"], b"").out, "a.txt\n");
    assert_eq!(run_in(&d, &["-L", "foo", "a.txt", "b.txt"], b"").out, "b.txt\n");
    assert_eq!(run_in(&d, &["--files-without-match", "foo", "a.txt", "b.txt"], b"").out, "b.txt\n");
}

#[test]
fn max_count_m() {
    assert_eq!(g(&["-m", "2", "foo"], S).out, "foo\nbaz foo\n");
    assert_eq!(g(&["--max-count=1", "foo"], S).out, "foo\n");
    assert_eq!(g(&["-m", "2", "-v", "foo"], S).out, "bar\nqux\n");
    assert_eq!(g(&["-m", "abc", "foo"], S).code, 2);
}

#[test]
fn only_matching_o() {
    assert_eq!(g(&["-o", "fo*"], "foo bar fo\n").out, "foo\nfo\n");
    assert_eq!(g(&["--only-matching", "ba."], "bar baz\n").out, "bar\nbaz\n");
    assert_eq!(g(&["-on", "fo*"], "foo bar fo\n").out, "1:foo\n1:fo\n");
    assert_eq!(g(&["-o", "x*"], "abc\n").out, ""); // empty matches are not printed
}

#[test]
fn quiet_q() {
    for flag in ["-q", "--quiet", "--silent"] {
        let r = g(&[flag, "foo"], S);
        assert_eq!((r.out.as_str(), r.code), ("", 0));
        assert_eq!(g(&[flag, "nomatch"], S).code, 1);
    }
    let d = fx("quiet", &[("a.txt", S.as_bytes())]);
    assert_eq!(run_in(&d, &["-q", "foo", "a.txt", "nofile"], b"").code, 0); // match wins over error
}

#[test]
fn no_messages_s_and_missing_files() {
    let r = g(&["-s", "foo", "nofile"], "");
    assert_eq!((r.out.as_str(), r.err.as_str(), r.code), ("", "", 2));
    assert_eq!(g(&["--no-messages", "foo", "nofile"], "").err, "");
    let r = g(&["foo", "nofile"], "");
    assert!(r.err.contains("nofile") && r.code == 2);
}

#[test]
fn line_number_n() {
    assert_eq!(g(&["-n", "foo"], S).out, "1:foo\n3:baz foo\n6:foo2\n");
    assert_eq!(g(&["--line-number", "qux"], S).out, "4:qux\n");
}

#[test]
fn with_and_without_filename_h_cap_h() {
    let d = fx("names", &[("a.txt", S.as_bytes()), ("b.txt", b"foo\n")]);
    assert_eq!(run_in(&d, &["-H", "bar", "a.txt"], b"").out, "a.txt:bar\n");
    assert_eq!(run_in(&d, &["--with-filename", "bar", "a.txt"], b"").out, "a.txt:bar\n");
    assert_eq!(g(&["-H", "bar"], S).out, "(standard input):bar\n");
    assert_eq!(run_in(&d, &["bar", "a.txt", "b.txt"], b"").out, "a.txt:bar\n"); // default with 2+ files
    assert_eq!(run_in(&d, &["-h", "foo", "a.txt", "b.txt"], b"").out, "foo\nbaz foo\nfoo2\nfoo\n");
    assert_eq!(run_in(&d, &["--no-filename", "bar", "a.txt", "b.txt"], b"").out, "bar\n");
    assert_eq!(run_in(&d, &["-inH", "FOO", "b.txt"], b"").out, "b.txt:1:foo\n"); // clustered flags
}

#[test]
fn context_a_b_c_num() {
    assert_eq!(g(&["-A", "1", "5"], NINE).out, "5\n6\n");
    assert_eq!(g(&["-A1", "5"], NINE).out, "5\n6\n"); // value attached
    assert_eq!(g(&["--after-context=1", "5"], NINE).out, "5\n6\n");
    assert_eq!(g(&["-B", "1", "5"], NINE).out, "4\n5\n");
    assert_eq!(g(&["--before-context=1", "5"], NINE).out, "4\n5\n");
    assert_eq!(g(&["-C", "1", "5"], NINE).out, "4\n5\n6\n");
    assert_eq!(g(&["--context=1", "5"], NINE).out, "4\n5\n6\n");
    assert_eq!(g(&["-2", "5"], NINE).out, "3\n4\n5\n6\n7\n");
    assert_eq!(g(&["-nA1", "5"], NINE).out, "5:5\n6-6\n");
    assert_eq!(g(&["-E", "-C1", "2|8"], NINE).out, "1\n2\n3\n--\n7\n8\n9\n"); // gap -> separator
    assert_eq!(g(&["-E", "-C1", "2|4"], NINE).out, "1\n2\n3\n4\n5\n"); // adjacent -> none
    assert_eq!(g(&["-A", "x", "5"], NINE).code, 2);
    assert_eq!(g(&["-A1", "-m1", "-E", "5|6"], NINE).out, "5\n6\n"); // trailing context after last match
}

#[test]
fn context_separator_between_files() {
    let d = fx("ctxfiles", &[("a.txt", b"x\nfoo\n"), ("b.txt", b"foo\ny\n")]);
    assert_eq!(run_in(&d, &["-A1", "foo", "a.txt", "b.txt"], b"").out, "a.txt:foo\n--\nb.txt:foo\nb.txt-y\n");
}

#[test]
fn color() {
    let m = "\x1b[01;31m\x1b[Kfoo\x1b[m\x1b[K";
    assert_eq!(g(&["--color=always", "foo"], "foo\n").out, format!("{m}\n"));
    assert_eq!(g(&["--colour=always", "foo"], "foo\n").out, format!("{m}\n"));
    assert_eq!(g(&["--color=never", "foo"], "foo\n").out, "foo\n");
    assert_eq!(g(&["--color", "foo"], "foo\n").out, "foo\n"); // auto, stdout is a pipe
    assert_eq!(g(&["--color=auto", "foo"], "foo\n").out, "foo\n");
    assert_eq!(g(&["--color=always", "-n", "foo"], "x foo y\n").out,
        format!("\x1b[32m\x1b[K1\x1b[m\x1b[K\x1b[36m\x1b[K:\x1b[m\x1b[Kx {m} y\n"));
    assert_eq!(g(&["--color=always", "-o", "foo"], "foo\n").out, format!("{m}\n"));
    assert_eq!(g(&["--color=always", "-v", "x"], "foo\n").out, "foo\n"); // no highlight with -v
    let d = fx("colorname", &[("a.txt", b"foo\n")]);
    assert_eq!(run_in(&d, &["--color=always", "-l", "foo", "a.txt"], b"").out, "\x1b[35m\x1b[Ka.txt\x1b[m\x1b[K\n");
    let r = g(&["--color=bad", "foo"], "foo\n");
    assert!(r.code == 2 && r.err.contains("--color"));
}

#[test]
fn binary_files_a_cap_i() {
    let b: &[u8] = b"foo\0\nfoo\n";
    let d = fx("bin", &[]);
    let r = run_in(&d, &["foo"], b);
    assert_eq!((r.out.as_str(), r.code), ("Binary file (standard input) matches\n", 0));
    assert_eq!(run_in(&d, &["--binary-files=binary", "foo"], b).out, "Binary file (standard input) matches\n");
    for a in [&["-a", "foo"][..], &["--text", "foo"], &["--binary-files=text", "foo"]] {
        assert_eq!(run_in(&d, a, b).out, "foo\0\nfoo\n");
    }
    for a in [&["-I", "foo"][..], &["--binary-files=without-match", "foo"]] {
        let r = run_in(&d, a, b);
        assert_eq!((r.out.as_str(), r.code), ("", 1));
    }
    assert_eq!(run_in(&d, &["-c", "foo"], b).out, "2\n"); // counts every line
    assert_eq!(run_in(&d, &["-q", "foo"], b).code, 0);
    assert_eq!(run_in(&d, &["--binary-files=bad", "foo"], b).code, 2);
    assert_eq!(run_in(&d, &["foo"], b"foo\n").out, "foo\n"); // text unaffected
}

#[test]
fn recursive_r_cap_r() {
    let d = fx("rec", &[("a.txt", b"foo\nbar\n"), ("sub/c.txt", b"foo\n"), ("sub/d.log", b"foo\n"), ("sub/e.txt", b"nope\n")]);
    assert_eq!(run_in(&d, &["-r", "foo", "sub"], b"").out, "sub/c.txt:foo\nsub/d.log:foo\n");
    assert_eq!(run_in(&d, &["-R", "foo", "sub"], b"").out, "sub/c.txt:foo\nsub/d.log:foo\n");
    assert_eq!(run_in(&d, &["--recursive", "foo", "sub"], b"").out, "sub/c.txt:foo\nsub/d.log:foo\n");
    assert_eq!(run_in(&d, &["--dereference-recursive", "foo", "sub"], b"").out, "sub/c.txt:foo\nsub/d.log:foo\n");
    // no FILE: search "." and print names without "./"
    assert_eq!(run_in(&d, &["-r", "foo"], b"").out, "a.txt:foo\nsub/c.txt:foo\nsub/d.log:foo\n");
    // explicit "." keeps the prefix
    assert_eq!(run_in(&d, &["-r", "foo", "."], b"").out, "./a.txt:foo\n./sub/c.txt:foo\n./sub/d.log:foo\n");
    assert_eq!(run_in(&d, &["-rh", "foo"], b"").out, "foo\nfoo\nfoo\n");
    assert_eq!(run_in(&d, &["-rl", "foo"], b"").out, "a.txt\nsub/c.txt\nsub/d.log\n");
    assert_eq!(run_in(&d, &["-rc", "foo", "sub"], b"").out, "sub/c.txt:1\nsub/d.log:1\nsub/e.txt:0\n");
    assert_eq!(run_in(&d, &["-rq", "foo"], b"").code, 0);
    let r = run_in(&d, &["foo", "sub"], b""); // directory without -r
    assert!(r.err.contains("Is a directory"));
}

#[test]
fn recursive_output_order_is_stable_with_threads() {
    let files: Vec<(String, Vec<u8>)> = (0..30).map(|i| (format!("f{i:02}.txt"), b"foo\n".to_vec())).collect();
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, c)| (n.as_str(), c.as_slice())).collect();
    let d = fx("order", &refs);
    let want: String = (0..30).map(|i| format!("f{i:02}.txt:foo\n")).collect();
    for _ in 0..5 { assert_eq!(run_in(&d, &["-r", "foo"], b"").out, want); }
}

#[test]
fn include_exclude_globs() {
    let d = fx("globs", &[("a.txt", b"foo\n"), ("b.log", b"foo\n"), ("sub/c.txt", b"foo\n"), ("sub/d.md", b"foo\n"),
        ("ex.txt", b"*.log\nd.*\n")]);
    let names = |a: &[&str]| run_in(&d, a, b"").out;
    assert_eq!(names(&["-rl", "--include=*.txt", "foo"]), "a.txt\nex.txt\nsub/c.txt\n".replace("ex.txt\n", "")); // ex.txt has no "foo"
    assert_eq!(names(&["-rl", "--include", "*.log", "foo"]), "b.log\n");
    assert_eq!(names(&["-rl", "--include=*.log", "--include=*.md", "foo"]), "b.log\nsub/d.md\n");
    assert_eq!(names(&["-rl", "--exclude=*.txt", "foo"]), "b.log\nsub/d.md\n");
    assert_eq!(names(&["-rl", "--exclude=*.txt", "--exclude=*.md", "foo"]), "b.log\n");
    assert_eq!(names(&["-rl", "--include=*.txt", "--exclude=c*", "foo"]), "a.txt\n"); // exclude wins
    assert_eq!(names(&["-rl", "--exclude=[ab].*", "foo"]), "sub/c.txt\nsub/d.md\n");
    assert_eq!(names(&["-rl", "--exclude=?.log", "foo"]), "a.txt\nsub/c.txt\nsub/d.md\n");
    assert_eq!(names(&["-rl", "--exclude=[!a].*", "foo"]), "a.txt\n");
    assert_eq!(names(&["-l", "--exclude=a.txt", "foo", "a.txt", "b.log"]), "b.log\n"); // command-line files too
    assert_eq!(names(&["-l", "--include=*.log", "foo", "a.txt", "b.log"]), "b.log\n");
}

#[test]
fn exclude_from_and_exclude_dir() {
    let d = fx("exfrom", &[("a.txt", b"foo\n"), ("b.log", b"foo\n"), ("sub/c.txt", b"foo\n"), ("sub/d.md", b"foo\n"),
        ("deep/x/e.txt", b"foo\n"), ("ex.lst", b"*.log\n\nd.*\n")]);
    let names = |a: &[&str]| run_in(&d, a, b"").out;
    assert_eq!(names(&["-rl", "--exclude-from=ex.lst", "foo"]), "a.txt\ndeep/x/e.txt\nsub/c.txt\n");
    assert_eq!(names(&["-rl", "--exclude-from", "ex.lst", "foo"]), "a.txt\ndeep/x/e.txt\nsub/c.txt\n");
    assert_eq!(names(&["-rl", "--exclude-dir=sub", "foo"]), "a.txt\nb.log\ndeep/x/e.txt\n");
    assert_eq!(names(&["-rl", "--exclude-dir=x", "foo"]), "a.txt\nb.log\nsub/c.txt\nsub/d.md\n");
    assert_eq!(names(&["-rl", "--exclude-dir=s*", "--exclude-dir=deep", "foo"]), "a.txt\nb.log\n");
    assert_eq!(names(&["-rl", "--exclude-dir=sub", "foo", "sub/"]), ""); // command-line dir, trailing slash
    assert_eq!(run_in(&d, &["-rl", "--exclude-from=missing", "foo"], b"").code, 2);
}

#[test]
fn null_data_z() {
    let i = b"a\nb\0c\0abc\0";
    let d = fx("null-data", &[]);
    assert_eq!(run_in(&d, &["-z", "b"], i).out, "a\nb\0abc\0");
    assert_eq!(run_in(&d, &["--null-data", "b"], i).out, "a\nb\0abc\0");
    assert_eq!(run_in(&d, &["-zc", "."], i).out, "3\n");
    assert_eq!(run_in(&d, &["-zn", "c"], i).out, "2:c\x003:abc\0");
    assert_eq!(run_in(&d, &["-zo", "b"], i).out, "b\0b\0");
    assert_eq!(run_in(&d, &["-zv", "a"], i).out, "c\0");
    assert_eq!(run_in(&d, &["-z", "^b"], i).out, "a\nb\0"); // ^ matches after embedded newline
}

#[test]
fn null_after_name_cap_z() {
    let d = fx("null-name", &[("a.txt", b"hi\nfoo\n"), ("b.txt", b"foo\n")]);
    let n = |a: &[&str]| run_in(&d, a, b"").out;
    assert_eq!(n(&["-Z", "foo", "a.txt", "b.txt"]), "a.txt\0foo\nb.txt\0foo\n");
    assert_eq!(n(&["--null", "-l", "foo", "a.txt", "b.txt"]), "a.txt\0b.txt\0");
    assert_eq!(n(&["-lZ", "foo", "a.txt", "b.txt"]), "a.txt\0b.txt\0");
    assert_eq!(n(&["-LZ", "hi", "a.txt", "b.txt"]), "b.txt\0");
    assert_eq!(n(&["-cZ", "foo", "a.txt", "b.txt"]), "a.txt\01\nb.txt\01\n");
    assert_eq!(n(&["-nZ", "foo", "a.txt", "b.txt"]), "a.txt\02:foo\nb.txt\01:foo\n");
}

#[test]
fn help_and_version() {
    let r = g(&["--help"], "");
    assert!(r.code == 0 && r.out.starts_with("Usage: grep"));
    // every option the tests above exercise is documented
    for o in ["-E", "-F", "-G", "-P", "-e", "-f", "-i", "-y", "--no-ignore-case", "-w", "-x", "-s", "-v", "-V",
        "--version", "--help", "-m", "-n", "-H", "-h", "-o", "-q", "--binary-files", "-a", "-I", "-r", "-R",
        "--include", "--exclude", "--exclude-from", "--exclude-dir", "-L", "-l", "-c", "--color", "--colour",
        "-z", "-Z", "--null", "--null-data", "--line-buffered", "-B", "-A", "-C", "-NUM"] {
        assert!(r.out.contains(o), "help is missing {o}");
    }
    assert_eq!(g(&["foo", "--", "--help"], "").out, ""); // after "--" it is a file name, not the option
    for flag in ["-V", "--version"] {
        let r = g(&[flag], "");
        assert!(r.code == 0 && r.out.starts_with("grep (Rust) "));
    }
}

#[test]
fn ignored_and_invalid_options() {
    assert_eq!(g(&["-U", "foo"], S).out, "foo\nbaz foo\nfoo2\n");
    assert_eq!(g(&["-T", "bar"], S).out, "bar\n");
    let r = g(&["--bogus", "x"], "");
    assert!(r.code == 2 && r.err.contains("unrecognized option '--bogus'"));
    let r = g(&["-k", "x"], "");
    assert!(r.code == 2 && r.err.contains("invalid option"));
    let r = g(&[], "");
    assert!(r.code == 2 && r.err.contains("Usage"));
    assert_eq!(g(&["-e"], "").code, 2); // missing argument
    assert_eq!(g(&["-E", "("], "x\n").code, 2); // bad regex
}

#[test]
fn match_spanning_lines_is_rejected() {
    // fast path searches the whole buffer; `\s` and `[^a]` could cross a newline
    assert_eq!(g(&["-E", r"a\sb"], "a\nb\n").code, 1);
    assert_eq!(g(&["a[^x]b"], "a\nb\n").code, 1);
    assert_eq!(g(&["-n", "-E", r"a\sb"], "a\nb\na b\n").out, "3:a b\n");
}

#[test]
fn line_buffered() {
    // same results as without the flag, stdin and files, with and without context
    assert_eq!(g(&["--line-buffered", "foo"], S).out, "foo\nbaz foo\nfoo2\n");
    assert_eq!(g(&["--line-buffered", "-vn", "foo"], S).out, "2:bar\n4:qux\n5:zzz\n");
    assert_eq!(g(&["--line-buffered", "-c", "foo"], S).out, "3\n");
    assert_eq!(g(&["--line-buffered", "-m1", "foo"], S).out, "foo\n");
    assert_eq!(g(&["--line-buffered", "-A1", "5"], NINE).out, "5\n6\n"); // context: read-all fallback
    assert_eq!(g(&["--line-buffered", "-q", "foo"], S).code, 0);
    assert_eq!(g(&["--line-buffered", "nomatch"], S).code, 1);
    let d = fx("linebuf", &[("a.txt", b"foo\n"), ("b.txt", b"foo\nbar\n")]);
    assert_eq!(run_in(&d, &["--line-buffered", "-r", "foo"], b"").out, "a.txt:foo\nb.txt:foo\n");

    // output appears while stdin is still open
    let mut c = Command::new(env!("CARGO_BIN_EXE_grep"))
        .args(["--line-buffered", "foo"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdin = c.stdin.take().unwrap();
    stdin.write_all(b"bar\nfoo one\n").unwrap();
    stdin.flush().unwrap();
    let out = c.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(out), &mut line).unwrap();
        tx.send(line).unwrap();
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    drop(stdin);
    c.wait().unwrap();
    assert_eq!(got.expect("no output before stdin closed"), "foo one\n");
}
