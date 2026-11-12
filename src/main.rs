use memchr::{memchr, memchr_iter, memrchr};
use regex::bytes::{Regex, RegexBuilder};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicUsize, Ordering}, mpsc};
use std::{fs, process::exit, thread};

#[derive(Default, PartialEq, Clone, Copy)]
enum Bin { #[default] Binary, Text, WithoutMatch }

#[derive(Default)]
struct Opt {
    bin: Bin, null_data: bool, null: bool, implicit_dot: bool, line_buffered: bool,
    invert: bool, count: bool, list: bool, list_non: bool, number: bool,
    only: bool, quiet: bool, no_msg: bool, recursive: bool,
    max: Option<usize>, after: usize, before: usize,
    with_name: Option<bool>,
    include: Vec<String>, exclude: Vec<String>, exclude_dir: Vec<String>,
}

// shortcut: BRE backreferences (\1) unsupported by the regex crate; upgrade via fancy-regex.
fn bre(p: &str) -> String {
    let mut o = String::new();
    let mut c = p.chars();
    while let Some(ch) = c.next() {
        match ch {
            '\\' => match c.next() {
                Some(n) if "(){}|+?".contains(n) => o.push(n),
                Some(n) => { o.push('\\'); o.push(n) }
                None => o.push_str("\\\\"),
            },
            '(' | ')' | '{' | '}' | '|' | '+' | '?' => { o.push('\\'); o.push(ch) }
            _ => o.push(ch),
        }
    }
    o
}

// shortcut: byte-wise globbing (non-ASCII bracket classes) and exponential backtracking on many `*`.
fn glob(p: &[u8], s: &[u8]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some(b'*') => (0..=s.len()).any(|i| glob(&p[1..], &s[i..])),
        Some(b'?') => !s.is_empty() && glob(&p[1..], &s[1..]),
        Some(b'[') => {
            let Some(&c) = s.first() else { return false };
            let neg = matches!(p.get(1), Some(b'!' | b'^'));
            let (st, mut i, mut ok) = (1 + neg as usize, 1 + neg as usize, false);
            loop {
                match p.get(i) {
                    None => return c == b'[' && glob(&p[1..], &s[1..]), // unterminated: literal '['
                    Some(b']') if i > st => break,
                    Some(&lo) => {
                        if p.get(i + 1) == Some(&b'-') && p.get(i + 2).is_some_and(|&h| h != b']') {
                            ok |= lo <= c && c <= p[i + 2];
                            i += 3;
                        } else {
                            ok |= lo == c;
                            i += 1;
                        }
                    }
                }
            }
            ok != neg && glob(&p[i + 1..], &s[1..])
        }
        Some(b'\\') if p.len() > 1 => s.first() == Some(&p[1]) && glob(&p[2..], &s[1..]),
        Some(&c) => s.first() == Some(&c) && glob(&p[1..], &s[1..]),
    }
}

impl Opt {
    fn eol(&self) -> u8 { if self.null_data { 0 } else { b'\n' } }

    /// --exclude/--include: a glob matches the whole path or any suffix that starts after a separator.
    fn wanted(&self, p: &Path) -> bool {
        !Self::any_glob(&self.exclude, p) && (self.include.is_empty() || Self::any_glob(&self.include, p))
    }

    fn any_glob(gs: &[String], p: &Path) -> bool {
        let name = p.to_string_lossy();
        let b = name.trim_end_matches(['/', '\\']).as_bytes();
        let starts: Vec<usize> = std::iter::once(0)
            .chain(b.iter().enumerate().filter(|(_, c)| **c == b'/' || **c == b'\\').map(|(i, _)| i + 1))
            .collect();
        gs.iter().any(|g| starts.iter().any(|&i| glob(g.as_bytes(), &b[i..])))
    }
}

const HELP: &str = "\
Usage: grep [OPTION]... PATTERNS [FILE]...
Search for PATTERNS in each FILE.
Example: grep -i 'hello world' menu.h main.c
PATTERNS can contain multiple patterns separated by newlines.

Pattern selection and interpretation:
  -E, --extended-regexp     PATTERNS are extended regular expressions
  -F, --fixed-strings       PATTERNS are strings
  -G, --basic-regexp        PATTERNS are basic regular expressions (default)
  -P, --perl-regexp         PATTERNS are Perl regular expressions (PCRE2)
  -e, --regexp=PATTERNS     use PATTERNS for matching
  -f, --file=FILE           take PATTERNS from FILE (- is standard input)
  -i, --ignore-case         ignore case distinctions in patterns and data
  -y                        synonym for -i
      --no-ignore-case      do not ignore case distinctions (default)
  -w, --word-regexp         match only whole words
  -x, --line-regexp         match only whole lines

Miscellaneous:
  -s, --no-messages         suppress error messages
  -v, --invert-match        select non-matching lines
  -V, --version             display version information and exit
      --help               display this help text and exit

Output control:
  -m, --max-count=NUM       stop after NUM selected lines
  -n, --line-number         print line number with output lines
  -H, --with-filename       print file name with output lines
  -h, --no-filename         suppress the file name prefix on output
  -o, --only-matching       show only nonempty parts of lines that match
  -q, --quiet, --silent     suppress all normal output
      --binary-files=TYPE   assume that binary files are TYPE;
                            TYPE is 'binary', 'text', or 'without-match'
  -a, --text                equivalent to --binary-files=text
  -I                        equivalent to --binary-files=without-match
  -r, --recursive           read all files under each directory, recursively,
                            without following symbolic links
  -R, --dereference-recursive
                            same as -r (symbolic links are not followed)
      --include=GLOB        search only files whose base name matches GLOB
      --exclude=GLOB        skip files whose base name matches GLOB
      --exclude-from=FILE   skip files matching any file pattern from FILE
      --exclude-dir=GLOB    skip directories that match GLOB
  -L, --files-without-match print only names of FILEs with no selected lines
  -l, --files-with-matches  print only names of FILEs with selected lines
  -c, --count               print only a count of selected lines per FILE
      --color[=WHEN], --colour[=WHEN]
                            use markers to highlight the matching strings;
                            WHEN is 'always', 'never', or 'auto'
      --line-buffered       flush output on every line
  -Z, --null                print 0 byte after FILE name
  -z, --null-data           a data line ends in 0 byte, not newline

Context control:
  -B, --before-context=NUM  print NUM lines of leading context
  -A, --after-context=NUM   print NUM lines of trailing context
  -C, --context=NUM         print NUM lines of output context
  -NUM                      same as --context=NUM

Accepted and ignored: -U, -T.
When FILE is '-' or absent (without -r), read standard input.
With -r and no FILE, search the working directory.
Exit status is 0 if a line is selected, 1 if none were, 2 if an error occurred.
";

fn die(msg: &str) -> ! { eprintln!("grep: {msg}"); exit(2) }

enum Re { Rx(Regex), Pcre(pcre2::bytes::Regex) }

impl Re {
    fn is_match(&self, l: &[u8]) -> bool {
        match self {
            Re::Rx(r) => r.is_match(l),
            Re::Pcre(r) => r.is_match(l).unwrap_or_else(|e| die(&e.to_string())),
        }
    }
    fn find_at(&self, h: &[u8], at: usize) -> Option<usize> {
        match self {
            Re::Rx(r) => r.find_at(h, at).map(|m| m.start()),
            Re::Pcre(r) => r.find_at(h, at).unwrap_or_else(|e| die(&e.to_string())).map(|m| m.start()),
        }
    }
    fn spans(&self, l: &[u8]) -> Vec<(usize, usize)> {
        match self {
            Re::Rx(r) => r.find_iter(l).map(|m| (m.start(), m.end())).collect(),
            Re::Pcre(r) => r.find_iter(l)
                .map(|m| m.map(|m| (m.start(), m.end())).unwrap_or_else(|e| die(&e.to_string())))
                .collect(),
        }
    }
}

enum Data { Map(memmap2::Mmap), Buf(Vec<u8>) }

impl Deref for Data {
    type Target = [u8];
    fn deref(&self) -> &[u8] { match self { Data::Map(m) => m, Data::Buf(v) => v } }
}

// shortcut: a mapped file truncated by another process mid-search can fault; upgrade by reading instead of mapping.
fn load(p: &Path) -> io::Result<Data> {
    let f = fs::File::open(p)?;
    let len = f.metadata()?.len();
    if len >= 1 << 17 {
        if let Ok(m) = unsafe { memmap2::Mmap::map(&f) } { return Ok(Data::Map(m)); }
    }
    let mut v = Vec::with_capacity(len as usize);
    (&f).read_to_end(&mut v)?;
    Ok(Data::Buf(v))
}

/// Collect files to search, sorted per directory. Reports walk errors.
fn walk(p: &Path, top: bool, o: &Opt, files: &mut Vec<PathBuf>, err: &mut bool) {
    let md = if top { fs::metadata(p) } else { fs::symlink_metadata(p) };
    let fail = |e: io::Error, err: &mut bool| {
        *err = true;
        if !o.no_msg { eprintln!("grep: {}: {e}", p.display()); }
    };
    match md {
        Ok(m) if m.is_dir() && o.recursive && Opt::any_glob(&o.exclude_dir, p) => {}
        Ok(m) if m.is_dir() && o.recursive => match fs::read_dir(p) {
            Ok(rd) => {
                let mut es: Vec<_> = rd.flatten().map(|e| e.path()).collect();
                es.sort();
                for e in es { walk(&e, false, o, files, err); }
            }
            Err(e) => fail(e, err),
        },
        Ok(m) if m.is_dir() => { if !o.no_msg { eprintln!("grep: {}: Is a directory", p.display()); } }
        Ok(m) if m.file_type().is_symlink() => {}
        Ok(_) => { if o.wanted(p) { files.push(p.to_path_buf()) } }
        Err(e) => fail(e, err),
    }
}

/// Search files; with several files and no context, search in parallel and print in file order.
fn run_files(o: &Opt, re: &Re, show_name: bool, color: bool, files: &[PathBuf], err: &mut bool) -> bool {
    let threads = thread::available_parallelism().map_or(1, |n| n.get()).min(files.len());
    let mut hit = false;
    let report = |e: &str| if !o.no_msg { eprintln!("{e}") };
    if threads < 2 || o.after > 0 || o.before > 0 || o.line_buffered {
        let mut cx = Ctx::new(o, re, show_name, color, Some(io::stdout().lock()));
        for f in files {
            match cx.search(f) {
                Ok(n) => hit |= n > 0,
                Err(e) => { *err = true; report(&format!("grep: {}: {e}", f.display())) }
            }
            if o.quiet && hit { break; }
        }
        return hit;
    }
    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Vec<u8>, usize, Option<String>)>();
    thread::scope(|s| {
        for _ in 0..threads {
            let (tx, next) = (tx.clone(), &next);
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(f) = files.get(i) else { break };
                let mut cx = Ctx::new(o, re, show_name, color, None);
                let (n, e) = match cx.search(f) {
                    Ok(n) => (n, None),
                    Err(e) => (0, Some(format!("grep: {}: {e}", f.display()))),
                };
                if tx.send((i, cx.out, n, e)).is_err() { break; }
            });
        }
        drop(tx);
        let mut so = io::stdout().lock();
        let (mut pending, mut want) = (BTreeMap::new(), 0);
        for (i, buf, n, e) in rx {
            pending.insert(i, (buf, n, e));
            while let Some((buf, n, e)) = pending.remove(&want) {
                if so.write_all(&buf).is_err() { exit(2); }
                if let Some(m) = e { *err = true; report(&m); }
                hit |= n > 0;
                if o.quiet && hit { so.flush().ok(); exit(0); }
                want += 1;
            }
        }
        so.flush().ok();
    });
    hit
}

/// Output goes to `out`; with a `sink` it is spilled to stdout in 64 KB chunks.
struct Ctx<'a> { o: &'a Opt, re: &'a Re, out: Vec<u8>, sink: Option<io::StdoutLock<'a>>, show_name: bool, printed: bool, color: bool }

impl<'a> Ctx<'a> {
    fn new(o: &'a Opt, re: &'a Re, show_name: bool, color: bool, sink: Option<io::StdoutLock<'a>>) -> Self {
        Ctx { o, re, out: Vec::new(), sink, show_name, printed: false, color }
    }

    fn spill(&mut self, force: bool) -> io::Result<()> {
        if let Some(s) = &mut self.sink {
            let lb = self.o.line_buffered;
            if force || lb || self.out.len() >= 1 << 16 { s.write_all(&self.out)?; self.out.clear(); }
            if lb { s.flush()?; } // stdout is only line-buffered by std; -z output has no newlines
        }
        Ok(())
    }

    fn search(&mut self, p: &Path) -> io::Result<usize> {
        let d = load(p)?;
        let full = p.to_string_lossy();
        // `grep -r PAT` with no FILE searches "." but prints names without the "./" prefix, like GNU.
        let name = if self.o.implicit_dot { full.strip_prefix("./").or_else(|| full.strip_prefix(".\\")).unwrap_or(&full) } else { &full };
        Ok(self.file(name, &d).unwrap_or_else(|_| exit(2)))
    }

    // shortcut: fixed GNU default colors; GREP_COLORS is not parsed.
    fn paint(&mut self, code: &str, s: &[u8]) -> io::Result<()> {
        if self.color {
            write!(self.out, "\x1b[{code}m\x1b[K")?;
            self.out.write_all(s)?;
            self.out.write_all(b"\x1b[m\x1b[K")
        } else {
            self.out.write_all(s)
        }
    }

    /// What follows a file name: a NUL with -Z, else the colored separator.
    fn name_end(&mut self, sep: u8) -> io::Result<()> {
        if self.o.null { self.out.write_all(&[0]) } else { self.paint("36", &[sep]) }
    }

    fn prefix(&mut self, name: &str, n: usize, sep: u8) -> io::Result<()> {
        if self.show_name { self.paint("35", name.as_bytes())?; self.name_end(sep)?; }
        if self.o.number { self.paint("32", n.to_string().as_bytes())?; self.paint("36", &[sep])?; }
        Ok(())
    }

    fn line(&mut self, name: &str, n: usize, l: &[u8], sep: u8) -> io::Result<()> {
        self.line_inner(name, n, l, sep)?;
        self.spill(false)
    }

    fn line_inner(&mut self, name: &str, n: usize, l: &[u8], sep: u8) -> io::Result<()> {
        if self.o.only {
            if sep != b':' || self.o.invert { return Ok(()); }
            for (s, e) in self.re.spans(l).into_iter().filter(|(s, e)| s != e) {
                self.prefix(name, n, sep)?;
                self.paint("01;31", &l[s..e])?;
                self.out.write_all(&[self.o.eol()])?;
            }
            return Ok(());
        }
        self.prefix(name, n, sep)?;
        let mut at = 0;
        if self.color && sep == b':' && !self.o.invert {
            for (s, e) in self.re.spans(l).into_iter().filter(|(s, e)| s != e) {
                self.out.write_all(&l[at..s])?;
                self.paint("01;31", &l[s..e])?;
                at = e;
            }
        }
        self.out.write_all(&l[at..])?;
        self.out.write_all(&[self.o.eol()])
    }

    /// Search the whole buffer at once and jump from hit to hit, instead of testing every line.
    /// Regex is compiled multi-line; a hit that spans lines is rejected by re-testing its own line.
    fn scan_fast(&mut self, name: &str, data: &[u8], stop_first: bool) -> io::Result<usize> {
        let o = self.o;
        let eol = o.eol();
        let (mut count, mut pos, mut lineno) = (0usize, 0usize, 0usize);
        while pos < data.len() && o.max.map_or(true, |m| count < m) {
            let Some(ms) = self.re.find_at(data, pos) else { break };
            let ls = memrchr(eol, &data[..ms]).map_or(0, |i| i + 1);
            if ls >= data.len() { break; }
            let le = memchr(eol, &data[ms..]).map_or(data.len(), |i| ms + i);
            lineno += memchr_iter(eol, &data[pos..ls]).count();
            let l = &data[ls..le];
            if self.re.is_match(l) {
                count += 1;
                if stop_first { break; }
                if !o.count { self.line(name, lineno + 1, l, b':')?; self.printed = true; }
            }
            lineno += 1;
            pos = (le + 1).min(data.len());
        }
        Ok(count)
    }

    /// Returns number of selected lines.
    fn file(&mut self, name: &str, data: &[u8]) -> io::Result<usize> {
        let o = self.o;
        // shortcut: with -z a file is never binary (GNU checks for encoding errors instead).
        let bin = o.bin != Bin::Text && !o.null_data && data[..data.len().min(8000)].contains(&0);
        if bin && o.bin == Bin::WithoutMatch { return Ok(0); }
        let ctx = o.after > 0 || o.before > 0;
        let quiet_scan = o.quiet || o.list || o.list_non;
        let mut count = 0usize;
        if !o.invert && !ctx {
            count = self.scan_fast(name, data, quiet_scan || (bin && !o.count))?;
        } else {
        let mut lines: Vec<&[u8]> = data.split(|&c| c == o.eol()).collect();
        if lines.last().is_some_and(|l| l.is_empty()) { lines.pop(); }
        let (mut after, mut last) = (0usize, 0usize);
        for (i, l) in lines.iter().enumerate() {
            let done = o.max.is_some_and(|m| count >= m);
            if done && after == 0 { break; }
            let sel = !done && self.re.is_match(l) != o.invert;
            if sel {
                count += 1;
                if quiet_scan || (bin && !o.count) { break; }
                if o.count { continue; }
                let start = last.max(i.saturating_sub(o.before));
                let gap = if last == 0 { self.printed } else { start > last };
                if ctx && gap { self.paint("36", b"--")?; self.out.write_all(b"\n")?; }
                for j in start..i { self.line(name, j + 1, lines[j], b'-')?; }
                self.line(name, i + 1, l, b':')?;
                self.printed = true;
                after = o.after;
                last = i + 1;
            } else if after > 0 && !o.count && !quiet_scan {
                self.line(name, i + 1, l, b'-')?;
                after -= 1;
                last = i + 1;
            }
        }
        }
        self.report(name, count, bin)?;
        Ok(count)
    }

    /// Per-file summary line (-l, -L, -c, binary notice), then flush pending output.
    fn report(&mut self, name: &str, count: usize, bin: bool) -> io::Result<()> {
        let o = self.o;
        if o.quiet || o.list || o.list_non {
            if (o.list && count > 0) || (o.list_non && count == 0) { self.paint("35", name.as_bytes())?; self.out.write_all(if o.null { b"\0" } else { b"\n" })?; }
        } else if o.count {
            if self.show_name { self.paint("35", name.as_bytes())?; self.name_end(b':')?; }
            writeln!(self.out, "{count}")?;
        } else if bin && count > 0 {
            writeln!(self.out, "Binary file {name} matches")?;
        }
        self.spill(true)
    }

    /// Search a reader line by line as data arrives (for --line-buffered on a pipe).
    /// shortcut: no context options and no binary detection here; main falls back to reading everything for context.
    fn stream(&mut self, name: &str, mut r: impl io::BufRead) -> io::Result<usize> {
        let o = self.o;
        let (eol, mut buf, mut count, mut n) = (o.eol(), vec![], 0usize, 0usize);
        while o.max.is_none_or(|m| count < m) {
            buf.clear();
            if r.read_until(eol, &mut buf)? == 0 { break; }
            if buf.last() == Some(&eol) { buf.pop(); }
            n += 1;
            if self.re.is_match(&buf) != o.invert {
                count += 1;
                if o.quiet || o.list || o.list_non { break; }
                if !o.count { self.line(name, n, &buf, b':')?; }
            }
        }
        self.report(name, count, false)?;
        Ok(count)
    }
}

fn long_to_short(k: &str) -> char {
    match k {
        "extended-regexp" => 'E', "fixed-strings" => 'F', "basic-regexp" => 'G', "perl-regexp" => 'P',
        "regexp" => 'e', "file" => 'f', "ignore-case" => 'i', "no-ignore-case" => 'Y',
        "word-regexp" => 'w', "line-regexp" => 'x', "invert-match" => 'v',
        "count" => 'c', "files-with-matches" => 'l', "files-without-match" => 'L',
        "max-count" => 'm', "only-matching" => 'o', "quiet" | "silent" => 'q',
        "no-messages" => 's', "line-number" => 'n', "with-filename" => 'H',
        "no-filename" => 'h', "after-context" => 'A', "before-context" => 'B',
        "context" => 'C', "recursive" => 'r', "color" | "colour" => 'K', "include" => '\x01', "exclude" => '\x02', "exclude-dir" => '\x03', "exclude-from" => '\x04', "dereference-recursive" => 'R', "text" => 'a', "null-data" => 'z', "null" => 'Z', "line-buffered" => '\x06', "binary-files" => '\x05',
        _ => die(&format!("unrecognized option '--{k}'")),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut o = Opt::default();
    let (mut pats, mut pos): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let (mut fixed, mut basic, mut icase, mut word, mut line_x, mut have_e) = (false, true, false, false, false, false);
    let mut no_opts = false;
    let mut perl = false;
    let mut color = false;
    let num = |s: &str| s.parse::<usize>().unwrap_or_else(|_| die(&format!("{s}: invalid number")));
    let readf = |f: &str| -> Vec<String> {
        let t = if f == "-" {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s).map(|_| s)
        } else { fs::read_to_string(f) };
        t.unwrap_or_else(|e| die(&format!("{f}: {e}"))).lines().map(String::from).collect()
    };
    while let Some(a) = args.next() {
        if no_opts || !a.starts_with('-') || a == "-" { pos.push(a); continue; }
        if a == "--" { no_opts = true; continue; }
        if a == "--help" { print!("{HELP}"); exit(0); }
        if a == "--version" || a == "-V" { println!("grep (Rust) {}", env!("CARGO_PKG_VERSION")); exit(0); }
        let mut items: Vec<(char, Option<String>)> = vec![];
        if let Some(l) = a.strip_prefix("--") {
            let (k, v) = match l.split_once('=') { Some((k, v)) => (k, Some(v.to_string())), None => (l, None) };
            items.push((long_to_short(k), v));
        } else {
            let cs: Vec<char> = a[1..].chars().collect();
            let mut i = 0;
            while i < cs.len() {
                let c = cs[i];
                if "efmABC".contains(c) {
                    let rest: String = cs[i + 1..].iter().collect();
                    items.push((c, if rest.is_empty() { None } else { Some(rest) }));
                    break;
                }
                if c.is_ascii_digit() {
                    let d: String = cs[i..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    i += d.len();
                    items.push(('C', Some(d)));
                    continue;
                }
                items.push((c, None));
                i += 1;
            }
        }
        for (c, v) in items {
            let mut val = || v.clone().or_else(|| args.next()).unwrap_or_else(|| die(&format!("option '{c}' requires an argument")));
            match c {
                'E' => { basic = false; fixed = false; perl = false }
                'P' => { perl = true; fixed = false; basic = false }
                'F' => fixed = true,
                'G' => { basic = true; fixed = false }
                'e' => { have_e = true; pats.extend(val().split('\n').map(String::from)) }
                'f' => { have_e = true; pats.extend(readf(&val())) }
                'i' | 'y' => icase = true,
                'Y' => icase = false,
                'w' => word = true,
                'x' => line_x = true,
                'v' => o.invert = true,
                'c' => o.count = true,
                'l' => o.list = true,
                'L' => o.list_non = true,
                'm' => o.max = Some(num(&val())),
                'o' => o.only = true,
                'q' => o.quiet = true,
                's' => o.no_msg = true,
                'n' => o.number = true,
                'H' => o.with_name = Some(true),
                'h' => o.with_name = Some(false),
                'A' => o.after = num(&val()),
                'B' => o.before = num(&val()),
                'C' => { let n = num(&val()); o.after = n; o.before = n }
                'r' | 'R' => o.recursive = true,
                'K' => color = match v.as_deref() {
                    None | Some("auto" | "tty" | "if-tty") => io::stdout().is_terminal(),
                    Some("always" | "yes" | "force") => true,
                    Some("never" | "no" | "none") => false,
                    Some(w) => die(&format!("invalid argument '{w}' for '--color'")),
                },
                '\x01' => o.include.push(val()),
                '\x02' => o.exclude.push(val()),
                '\x03' => o.exclude_dir.push(val()),
                '\x04' => o.exclude.extend(readf(&val()).into_iter().filter(|g| !g.is_empty())),
                'z' => o.null_data = true,
                'a' => o.bin = Bin::Text,
                'I' => o.bin = Bin::WithoutMatch,
                '\x05' => o.bin = match val().as_str() {
                    "binary" => Bin::Binary,
                    "text" => Bin::Text,
                    "without-match" => Bin::WithoutMatch,
                    _ => die("unknown binary-files type"),
                },
                'Z' => o.null = true,
                '\x06' => o.line_buffered = true,
                'U' | 'T' => {}
                _ => die(&format!("invalid option -- '{c}'")),
            }
        }
    }
    if !have_e {
        if pos.is_empty() { eprintln!("Usage: grep [OPTION]... PATTERNS [FILE]..."); exit(2); }
        pats.extend(pos.remove(0).split('\n').map(String::from));
    }
    let alts: Vec<String> = pats.iter().map(|p| {
        if fixed { regex::escape(p) } else if basic { bre(p) } else { p.clone() }
    }).collect();
    let mut src = format!("(?:{})", alts.join("|"));
    // -w as non-word lookarounds (GNU -P does the same); \b would break on non-word edge chars.
    if line_x { src = format!("^{src}$") } else if word { src = format!(r"(?<!\w){src}(?!\w)") }
    let re = if perl {
        Re::Pcre(pcre2::bytes::RegexBuilder::new().caseless(icase).multi_line(true).build(&src).unwrap_or_else(|e| die(&e.to_string())))
    } else {
        // shortcut: regex crate has no lookaround, so -w there uses \b (differs when pattern edge is non-word).
        let s = src.replace(r"(?<!\w)", r"\b").replace(r"(?!\w)", r"\b");
        Re::Rx(RegexBuilder::new(&s).case_insensitive(icase).multi_line(true).build().unwrap_or_else(|e| die(&e.to_string())))
    };

    let files = pos;
    o.implicit_dot = files.is_empty() && o.recursive;
    let show_name = o.with_name.unwrap_or(o.recursive || files.len() > 1);
    let (hit, mut err);
    err = false;
    if (files.is_empty() && !o.recursive) || files == ["-"] {
        let mut cx = Ctx::new(&o, &re, show_name, color, Some(io::stdout().lock()));
        if o.line_buffered && o.after == 0 && o.before == 0 {
            hit = cx.stream("(standard input)", io::stdin().lock()).map_or_else(|_| exit(2), |n| n > 0);
        } else {
            let mut d = vec![];
            io::stdin().read_to_end(&mut d).ok();
            hit = cx.file("(standard input)", &d).map_or_else(|_| exit(2), |n| n > 0);
        }
    } else {
        let roots: Vec<PathBuf> = if files.is_empty() { vec![".".into()] } else { files.iter().map(PathBuf::from).collect() };
        let mut list = vec![];
        for r in &roots { walk(r, true, &o, &mut list, &mut err); }
        hit = run_files(&o, &re, show_name, color, &list, &mut err);
    }
    exit(if hit && (o.quiet || !err) { 0 } else if err { 2 } else { 1 });
}
