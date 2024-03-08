use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use dwat::prelude::*;

use tree_sitter as ts;
use tree_sitter_c as ts_c;
use ts::{Parser as TsParser, Query, QueryCursor};

use clap::ArgAction::Append;
use clap::Parser;
use lazy_static::lazy_static;
use memmap2::Mmap;
use regex::Regex;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;
use tokio::task;

#[derive(Parser)]
struct CmdArgs {
    /// Path to vmlinux file
    #[clap(help = "The path to the vmlinux file.")]
    vmlinux_path: PathBuf,

    /// Path to Linux source code
    #[clap(help = "The path to the Linux source code directory.")]
    source_path: PathBuf,

    /// The lower bound for struct size
    #[clap(help = "The lower bound for struct sizes (exclusive).")]
    lower_bound: usize,

    /// The upper bound for struct size
    #[clap(help = "The upper bound for struct sizes (inclusive).")]
    upper_bound: usize,

    /// Silence dwat/weggli output, only print struct names
    #[clap(
        long,
        action,
        help = "Silence dwat/weggli output, only print struct \
                                 names."
    )]
    quiet: bool,

    /// Allocation flags flags argument regex
    #[clap(long, help = "Allocation flags argument regex")]
    flags: Option<String>,

    /// Glob to exclude files, can be specified multiple times to provide
    /// multiple globs
    #[clap(long, action=Append, help = "Glob to exclude files based on, can be \
                                        specified multiple times")]
    exclude: Vec<String>,

    /// Number of threads to scale up to
    #[clap(long, help = "Number of threads to scale up to")]
    threads: Option<usize>,
}

// Define a global static mutex for stdout
lazy_static! {
    static ref STDOUT_MUTEX: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

fn collect_src_files(dir: &PathBuf) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .map_or(false, |ext| ext == "c" || ext == "h")
        })
        .collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = CmdArgs::parse();

    let file = File::open(args.vmlinux_path).await?;
    let mmap = unsafe { Mmap::map(&file) }?;

    let dwarf = dwat::dwarf::OwnedDwarf::load(&*mmap)?;
    let struct_map = dwarf.get_named_types_map::<dwat::Struct>()?;

    let struct_map: HashMap<String, dwat::Struct> = {
        struct_map
            .into_iter()
            .filter(|(_, struc)| {
                if let Ok(bytesz) = struc.byte_size(&dwarf) {
                    args.lower_bound < bytesz && bytesz <= args.upper_bound
                } else {
                    false
                }
            })
            .collect()
    };

    let iter_files: Vec<PathBuf> = collect_src_files(&args.source_path);
    let mut files: Vec<PathBuf> = vec![];

    let exclude_globs = &args.exclude;
    if exclude_globs.len() > 0 {
        let mut builder = globset::GlobSetBuilder::new();
        for glob in exclude_globs {
            builder.add(globset::Glob::new(glob)?);
        }

        let set = builder.build()?;

        // Filter files that do not match the exclusion pattern
        for file in iter_files {
            if !set.is_match(&file) {
                files.push(file)
            }
        }
    } else {
        files = iter_files;
    }

    if files.len() == 0 {
        println!("Exiting, no files to process");
        return Ok(());
    }

    let mut handles = vec![];
    let threads = args.threads.unwrap_or(1);
    if threads > 1000 {
        panic!("The max number of threads allowed is 1000!");
    }
    let flimit_sem = Arc::new(Semaphore::new(threads));

    let shared_dwarf = Arc::new(RwLock::new(dwarf));
    let shared_struct_map = Arc::new(RwLock::new(struct_map));

    for file in files {
        let permit = flimit_sem.clone().acquire_owned().await.unwrap();
        let shared_struct_map = Arc::clone(&shared_struct_map);
        let shared_dwarf = Arc::clone(&shared_dwarf);
        let flags_regex_str = args.flags.clone();
        let handle = tokio::spawn(async move {
            read_and_process_file(file, shared_struct_map, shared_dwarf, flags_regex_str)
                .await
                .unwrap();
            drop(permit);
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }

    Ok(())
}

fn byte_offset_to_line_number(content: &Vec<u8>, byte_offset: usize) -> Option<usize> {
    let mut line_number = 1;
    let mut current_byte_index = 0;

    for char in content {
        if current_byte_index >= byte_offset {
            return Some(line_number);
        }
        if char == &b'\n' {
            line_number += 1;
        }
        current_byte_index += 1;
    }
    None
}

fn apply_highlight_ranges(
    content: &mut String,
    base_range: &std::ops::Range<usize>,
    highlight_ranges: &Vec<std::ops::Range<usize>>,
) {
    let mut added_bytes = 0;
    for range in highlight_ranges {
        let insert_start = (range.start - base_range.start) + added_bytes;
        content.insert(insert_start, '\x1b');
        content.insert(insert_start + 1, '[');
        content.insert(insert_start + 2, '3');
        content.insert(insert_start + 3, '1');
        content.insert(insert_start + 4, 'm');

        let insert_end = (range.end - base_range.start) + added_bytes + 5;
        content.insert(insert_end, '\x1b');
        content.insert(insert_end + 1, '[');
        content.insert(insert_end + 2, '0');
        content.insert(insert_end + 3, 'm');
        added_bytes += 9;
    }
}

fn display_match(
    content: &Vec<u8>,
    path: &PathBuf,
    struct_: &dwat::Struct,
    dwarf: &Arc<RwLock<dwat::dwarf::OwnedDwarf>>,
    qm: &QueryMatch,
) {
    let struct_name = qm.struct_name.utf8_text(&content).unwrap();

    let dwarf = dwarf.read().expect("failed to aqcuire dwarf rwlock");
    let struct_str = struct_.to_string_verbose(&*dwarf, 1).unwrap();
    drop(dwarf);

    let decl_line_start =
        byte_offset_to_line_number(&content, qm.function_definition.byte_range().start).unwrap();

    let mut match_ranges: Vec<std::ops::Range<usize>> = vec![];
    match_ranges.push(qm.struct_name.byte_range());
    match_ranges.push(qm.decl_name.byte_range());
    match_ranges.push(qm.assign_name.byte_range());
    match_ranges.push(qm.assign_func.byte_range());
    match_ranges.push(qm.flags.byte_range());

    let base_range: std::ops::Range<usize> = qm.function_definition.byte_range();

    let mut function_src = qm
        .function_definition
        .utf8_text(content)
        .unwrap()
        .to_string();

    // find line index opening brace is on
    let brace_location = function_src
        .find('{')
        .expect("no opening brace in function source?");
    let end_decl_line =
        byte_offset_to_line_number(&content[decl_line_start..].to_vec(), brace_location)
            .expect("failed when looking for line number of opening brace")
            - 1;

    // determine which lines will be included, always include lines up to the opening brace
    let mut included_lines: Vec<usize> = (0..end_decl_line).collect();
    let mut seen: usize = 0;
    for (idx, line) in function_src.lines().enumerate() {
        for range in &match_ranges {
            if (seen..seen + line.len() + 1).contains(&(&range.start - &base_range.start)) {
                if !included_lines.contains(&idx) {
                    included_lines.push(idx);
                }
            }
        }
        seen += line.len() + 1;
    }

    // highlight captures
    if std::io::stdout().is_terminal() {
        apply_highlight_ranges(&mut function_src, &base_range, &match_ranges);
    }

    let lock = STDOUT_MUTEX.lock().expect("failed to acquire stdout lock");

    println!("======== Found allocation site for: struct {struct_name} ========\n");
    println!("{}", struct_str);
    println!("");
    if std::io::stdout().is_terminal() {
        println!(
            "\x1b[1m{}\x1b[0m:{}",
            path.to_str().unwrap(),
            decl_line_start
        );
    } else {
        println!("{}:{}", path.to_str().unwrap(), decl_line_start);
    }

    let src_lines = function_src.lines().collect::<Vec<&str>>();

    // add last line if it wasn't already included and it is a return
    let src_lines_ct = function_src.lines().count() - 1;
    if !included_lines.contains(&(src_lines_ct - 1)) {
        if src_lines[src_lines_ct - 1].contains("return ")
            && src_lines[src_lines_ct - 1].ends_with(";")
        {
            included_lines.push(src_lines_ct - 1);
        }
    }
    // add last line if it wasn't already included
    if !included_lines.contains(&src_lines_ct) {
        included_lines.push(src_lines_ct);
    }

    // set initially to max, so that the elipses won't print the first time through
    // minus one so that it doesn't overflow in the or condition for debug builds
    let mut last_line = usize::MAX-1;
    for line_idx in included_lines {
        if line_idx == usize::MAX-1 || last_line + 1 != line_idx {
            println!("...");
        }
        println!("{}", src_lines[line_idx]);
        last_line = line_idx;
    }
    println!("");

    drop(lock);
}

#[derive(Debug)]
struct QueryMatch<'a> {
    function_definition: ts::Node<'a>,
    struct_name: ts::Node<'a>,
    decl_name: ts::Node<'a>,
    assign_name: ts::Node<'a>,
    _assign_call: ts::Node<'a>,
    assign_func: ts::Node<'a>,
    flags: ts::Node<'a>,
}

async fn process_file_content(
    path: PathBuf,
    content: Vec<u8>,
    struct_map: Arc<RwLock<HashMap<String, dwat::Struct>>>,
    dwarf: Arc<RwLock<dwat::dwarf::OwnedDwarf>>,
    flags_regex_str: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut parser = TsParser::new();

    parser
        .set_language(ts_c::language())
        .expect("Error loading C grammar");

    let parsed = parser
        .parse(&content, None)
        .expect("Parser returned no tree");
    let root_node = parsed.root_node();

    let query_str = "
    (
        function_definition
        declarator: (_) @function.decl
        body: (
            compound_statement (
                declaration type: (
                    struct_specifier name: (
                        type_identifier
                    ) @struct.name
                ) declarator: (
                    pointer_declarator declarator: (
                        identifier
                    ) @declaration.name
                )
            )
            (expression_statement (
                assignment_expression
                    left: (identifier) @assignment.name
                    right: (
                        (call_expression
                            function: (identifier) @assignment.function
                            (#match? @assignment.function \"k[mz]alloc\")
                            arguments: (argument_list
                                (_) @flags .
                            )
                        ) @assignment.call
                    )
                )
            )
            (#eq? @declaration.name @assignment.name)
        )
    ) @function.def
    ";

    let query = Query::new(ts_c::language(), query_str).expect("Error parsing query");
    let mut query_cursor = QueryCursor::new();
    let matches = query_cursor.matches(&query, root_node, &content[..]);

    for match_ in matches {
        let captures = match_.captures;
        let struct_name = captures
            .get(2)
            .unwrap()
            .node
            .utf8_text(&content)
            .unwrap_or("")
            .to_string();

        let struct_map = struct_map.read().unwrap();
        if let Some(struct_) = struct_map.get(&struct_name) {
            let mut flags_regex = Regex::new(".*")?;
            if let Some(ref flags_regex_str) = flags_regex_str {
                flags_regex = Regex::new(&flags_regex_str)?;
            }
            let flags = captures
                .get(7)
                .unwrap()
                .node
                .utf8_text(&content)
                .unwrap_or("")
                .to_string();
            if flags_regex.find(&flags).is_none() {
                continue;
            }

            let qm = QueryMatch {
                function_definition: captures.get(0).unwrap().node,
                struct_name: captures.get(2).unwrap().node,
                decl_name: captures.get(3).unwrap().node,
                assign_name: captures.get(4).unwrap().node,
                _assign_call: captures.get(5).unwrap().node,
                assign_func: captures.get(6).unwrap().node,
                flags: captures.get(7).unwrap().node,
            };

            display_match(&content, &path, &struct_, &dwarf, &qm);
        }
    }

    Ok(())
}

async fn read_and_process_file(
    path: PathBuf,
    struct_map: Arc<RwLock<HashMap<String, dwat::Struct>>>,
    dwarf: Arc<RwLock<dwat::dwarf::OwnedDwarf>>,
    flags_regex_str: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::open(path.clone()).await?;
    let mut contents = vec![];
    file.read_to_end(&mut contents).await?;

    let struct_map = struct_map.clone();
    let dwarf = dwarf.clone();
    let _ = task::spawn_blocking(move || {
        process_file_content(path, contents, struct_map, dwarf, flags_regex_str)
    })
    .await?
    .await;
    Ok(())
}
