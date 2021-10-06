//  * This file is part of the uutils coreutils package.
//  *
//  * (c) Alex Lyon <arcterus@mail.com>
//  *
//  * For the full copyright and license information, please view the LICENSE
//  * file that was distributed with this source code.

// spell-checker:ignore (ToDO) sbytes slen dlen memmem

#[macro_use]
extern crate uucore;

use clap::{crate_version, App, Arg};
use memchr::memmem;
use std::io::{stdin, stdout, BufReader, Read, Write};
use std::{fs::File, path::Path};
use uucore::display::Quotable;
use uucore::InvalidEncodingHandling;

static NAME: &str = "tac";
static USAGE: &str = "[OPTION]... [FILE]...";
static SUMMARY: &str = "Write each file to standard output, last line first.";

mod options {
    pub static BEFORE: &str = "before";
    pub static REGEX: &str = "regex";
    pub static SEPARATOR: &str = "separator";
    pub static FILE: &str = "file";
}

pub fn uumain(args: impl uucore::Args) -> i32 {
    let args = args
        .collect_str(InvalidEncodingHandling::ConvertLossy)
        .accept_any();

    let matches = uu_app().get_matches_from(args);

    let before = matches.is_present(options::BEFORE);
    let regex = matches.is_present(options::REGEX);
    let raw_separator = matches.value_of(options::SEPARATOR).unwrap_or("\n");
    let separator = if raw_separator.is_empty() {
        "\0"
    } else {
        raw_separator
    };

    let files: Vec<&str> = match matches.values_of(options::FILE) {
        Some(v) => v.collect(),
        None => vec!["-"],
    };

    tac(files, before, regex, separator)
}

pub fn uu_app() -> App<'static, 'static> {
    App::new(uucore::util_name())
        .name(NAME)
        .version(crate_version!())
        .usage(USAGE)
        .about(SUMMARY)
        .arg(
            Arg::with_name(options::BEFORE)
                .short("b")
                .long(options::BEFORE)
                .help("attach the separator before instead of after")
                .takes_value(false),
        )
        .arg(
            Arg::with_name(options::REGEX)
                .short("r")
                .long(options::REGEX)
                .help("interpret the sequence as a regular expression")
                .takes_value(false),
        )
        .arg(
            Arg::with_name(options::SEPARATOR)
                .short("s")
                .long(options::SEPARATOR)
                .help("use STRING as the separator instead of newline")
                .takes_value(true),
        )
        .arg(Arg::with_name(options::FILE).hidden(true).multiple(true))
}

/// Print lines of a buffer in reverse, with line separator given as a regex.
///
/// `data` contains the bytes of the file.
///
/// `pattern` is the regular expression given as a
/// [`regex::bytes::Regex`] (not a [`regex::Regex`], since the input is
/// given as a slice of bytes). If `before` is `true`, then each match
/// of this pattern in `data` is interpreted as the start of a line. If
/// `before` is `false`, then each match of this pattern is interpreted
/// as the end of a line.
///
/// This function writes each line in `data` to [`std::io::Stdout`] in
/// reverse.
///
/// # Errors
///
/// If there is a problem writing to `stdout`, then this function
/// returns [`std::io::Error`].
fn buffer_tac_regex(
    data: &[u8],
    pattern: &regex::bytes::Regex,
    before: bool,
) -> std::io::Result<()> {
    let out = stdout();
    let mut out = out.lock();

    // The index of the line separator for the current line.
    //
    // As we scan through the `data` from right to left, we update this
    // variable each time we find a new line separator. We restrict our
    // regular expression search to only those bytes up to the line
    // separator.
    let mut this_line_end = data.len();

    // The index of the start of the next line in the `data`.
    //
    // As we scan through the `data` from right to left, we update this
    // variable each time we find a new line.
    //
    // If `before` is `true`, then each line starts immediately before
    // the line separator. Otherwise, each line starts immediately after
    // the line separator.
    let mut following_line_start = data.len();

    // Iterate over each byte in the buffer in reverse. When we find a
    // line separator, write the line to stdout.
    //
    // The `before` flag controls whether the line separator appears at
    // the end of the line (as in "abc\ndef\n") or at the beginning of
    // the line (as in "/abc/def").
    for i in (0..data.len()).rev() {
        // Determine if there is a match for `pattern` starting at index
        // `i` in `data`. Only search up to the line ending that was
        // found previously.
        if let Some(match_) = pattern.find_at(&data[..this_line_end], i) {
            // Record this index as the ending of the current line.
            this_line_end = i;

            // The length of the match (that is, the line separator), in bytes.
            let slen = match_.end() - match_.start();

            if before {
                out.write_all(&data[i..following_line_start])?;
                following_line_start = i;
            } else {
                out.write_all(&data[i + slen..following_line_start])?;
                following_line_start = i + slen;
            }
        }
    }

    // After the loop terminates, write whatever bytes are remaining at
    // the beginning of the buffer.
    out.write_all(&data[0..following_line_start])?;
    Ok(())
}

/// Write lines from `data` to stdout in reverse.
///
/// This function writes to [`stdout`] each line appearing in `data`,
/// starting with the last line and ending with the first line. The
/// `separator` parameter defines what characters to use as a line
/// separator.
///
/// If `before` is `false`, then this function assumes that the
/// `separator` appears at the end of each line, as in `"abc\ndef\n"`.
/// If `before` is `true`, then this function assumes that the
/// `separator` appears at the beginning of each line, as in
/// `"/abc/def"`.
fn buffer_tac(data: &[u8], before: bool, separator: &str) -> std::io::Result<()> {
    let out = stdout();
    let mut out = out.lock();

    // The number of bytes in the line separator.
    let slen = separator.as_bytes().len();

    // The index of the start of the next line in the `data`.
    //
    // As we scan through the `data` from right to left, we update this
    // variable each time we find a new line.
    //
    // If `before` is `true`, then each line starts immediately before
    // the line separator. Otherwise, each line starts immediately after
    // the line separator.
    let mut following_line_start = data.len();

    // Iterate over each byte in the buffer in reverse. When we find a
    // line separator, write the line to stdout.
    //
    // The `before` flag controls whether the line separator appears at
    // the end of the line (as in "abc\ndef\n") or at the beginning of
    // the line (as in "/abc/def").
    for i in memmem::rfind_iter(data, separator) {
        if before {
            out.write_all(&data[i..following_line_start])?;
            following_line_start = i;
        } else {
            out.write_all(&data[i + slen..following_line_start])?;
            following_line_start = i + slen;
        }
    }

    // After the loop terminates, write whatever bytes are remaining at
    // the beginning of the buffer.
    out.write_all(&data[0..following_line_start])?;
    Ok(())
}

fn tac(filenames: Vec<&str>, before: bool, regex: bool, separator: &str) -> i32 {
    let mut exit_code = 0;

    let pattern = if regex {
        Some(crash_if_err!(1, regex::bytes::Regex::new(separator)))
    } else {
        None
    };

    for &filename in &filenames {
        let mut file = BufReader::new(if filename == "-" {
            Box::new(stdin()) as Box<dyn Read>
        } else {
            let path = Path::new(filename);
            if path.is_dir() || path.metadata().is_err() {
                if path.is_dir() {
                    show_error!("{}: read error: Invalid argument", filename.maybe_quote());
                } else {
                    show_error!(
                        "failed to open {} for reading: No such file or directory",
                        filename.quote()
                    );
                }
                exit_code = 1;
                continue;
            }
            match File::open(path) {
                Ok(f) => Box::new(f) as Box<dyn Read>,
                Err(e) => {
                    show_error!("failed to open {} for reading: {}", filename.quote(), e);
                    exit_code = 1;
                    continue;
                }
            }
        });

        let mut data = Vec::new();
        if let Err(e) = file.read_to_end(&mut data) {
            show_error!("failed to read {}: {}", filename.quote(), e);
            exit_code = 1;
            continue;
        };
        if let Some(pattern) = &pattern {
            buffer_tac_regex(&data, pattern, before)
        } else {
            buffer_tac(&data, before, separator)
        }
        .unwrap_or_else(|e| crash!(1, "failed to write to stdout: {}", e));
    }
    exit_code
}
