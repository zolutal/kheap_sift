/*
Original Copyright notice:

Copyright 2021 Google LLC

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

     https://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

Copyright notice retained because this file largely contains code copied from:
https://github.com/weggli-rs/weggli/blob/main/src/main.rs

Modified by Justin Miller (https://github.com/zolutal), 2024
*/

use rayon::iter::{IntoParallelIterator, ParallelIterator, ParallelBridge};
use regex::Regex;
use weggli::result::QueryResult;
use thread_local::ThreadLocal;
use std::sync::mpsc::Receiver;
use weggli::query::QueryTree;
use std::sync::mpsc::Sender;
use std::cell::RefCell;
use std::path::PathBuf;
use colored::Colorize;
use tree_sitter::Tree;
use walkdir::WalkDir;
use std::sync::mpsc;
use std::path::Path;
use std::sync::Arc;
use std::fs;

pub(super) struct WorkItem {
    pub(crate) qt: QueryTree,
    pub(crate) identifiers: Vec<String>,
}

pub(super) struct ResultsCtx {
    _query_index: usize,
    _path: String,
    _source: std::sync::Arc<String>,
    _result: weggli::result::QueryResult,
}


pub(super) fn weggling_time(work: &[WorkItem], files: Vec<PathBuf>,
                            struct_name: String, struct_type: dwat::Struct,
                            flags_regex: Regex, dwarf: &dwat::Dwarf,
                            quiet: bool) {
    rayon::scope(|s| {
        // spin up channels for worker communication
        let (ast_tx, ast_rx) = mpsc::channel();
        let (results_tx, _results_rx) = mpsc::channel();

        // avoid lifetime issues
        let cpp = false;
        let w = &work;
        let files = files.clone();
        let quiet = &quiet;
        let _enable_line_numbers = false;

        // Spawn worker to iterate through files, parse potential matches and forward ASTs
        s.spawn(move |_| parse_files_worker(files, ast_tx, &w, cpp));

        // Run search queries on ASTs and apply CLI constraints
        // on the results. For single query executions, we can
        // directly print any remaining matches. For multi
        // query runs we forward them to our next worker function
        s.spawn(move |_| execute_queries_worker(ast_rx, results_tx, &w,
                                       struct_name, struct_type, flags_regex,
                                       dwarf, quiet));
    });
}

/// Recursively iterate through all files under `path` that match an ending listed in `extensions`
pub(super) fn iter_files(path: &Path, extensions: Vec<String>) -> Vec<PathBuf> {
    let is_hidden = |entry: &walkdir::DirEntry| {
        entry
            .file_name()
            .to_str()
            .map(|s| s.starts_with('.'))
            .unwrap_or(false)
    };

    WalkDir::new(path)
        .into_iter()
        .filter_entry(move |e| !is_hidden(e))
        .filter_map(|e| e.ok())
        .filter(move |entry| {
            if entry.file_type().is_dir() {
                return false;
            }

            let path = entry.path();

            match path.extension() {
                None => return false,
                Some(ext) => {
                    let s = ext.to_str().unwrap_or_default();
                    if !extensions.contains(&s.to_string()) {
                        return false;
                    }
                }
            }
            true
        }).map(|d| d.into_path()).collect()

}

/// Fetches parsed ASTs from `receiver`, runs all queries in `work` on them and
/// filters the results based on the provided regex `constraints` and --unique --limit switches.
/// For single query runs, the remaining results are directly printed. Otherwise they get forwarded
/// to `multi_query_worker` through the `results_tx` channel.
pub(super) fn execute_queries_worker(
    receiver: Receiver<(Arc<String>, Tree, String)>,
    results_tx: Sender<ResultsCtx>,
    work: &[WorkItem],
    struct_name: String,
    struct_type: dwat::Struct,
    flags_regex: Regex,
    dwarf: &dwat::Dwarf,
    quiet: &bool,
) {
    receiver.into_iter().par_bridge().for_each_with(
        results_tx,
        |_results_tx, (source, tree, path)| {
            // For each query
            work.iter()
                .enumerate()
                .for_each(|(_i, WorkItem { qt, identifiers: _ })| {
                    // Run query
                    let matches = qt.matches(tree.root_node(), &source);

                    if matches.is_empty() {
                        return;
                    }

                    let fmt = struct_type.to_string_verbose(dwarf, 1).expect(
                        &format!("dwat failed when formatting struct: {}", struct_name)
                    );

                    if !quiet {
                        // Print match or forward it if we are in a multi query context
                        let process_match = |m: QueryResult| -> Option<String> {
                            // single query
                            if let Some(flags_capture) = m.captures.last() {
                                let range = &flags_capture.range;
                                let flags = &source[range.start..range.end];
                                if flags_regex.find(flags).is_none() {
                                    return None;
                                }
                            }

                            let line = source[..m.start_offset()].matches('\n').count() + 1;
                            Some(format!(
                                "{}:{}\n{}",
                                path.clone().bold(),
                                line,
                                m.display(&source, 2, 2, false) // b4 after line_nos
                            ))
                        };

                        let mut results: Vec<String> = vec![];
                        for m in matches.into_iter() {
                            if let Some(r) = process_match(m) {
                                results.push(r);
                            }
                        }

                        if results.len() > 0 {
                            println!("======== Found allocation sites for: struct {} ========\n", struct_name);
                            println!("{}\n", fmt);
                            results.into_iter().for_each(|r| println!("{}", r))
                        }

                    } else {
                        println!("{struct_name}");
                    }
                });
        },
    );
}

/// Iterate over all paths in `files`, parse files that might contain a match for any of the queries
/// in `work` and send them to the next worker using `sender`.
fn parse_files_worker(
    files: Vec<PathBuf>,
    sender: Sender<(Arc<String>, Tree, String)>,
    work: &[WorkItem],
    is_cpp: bool,
) {
    let tl = ThreadLocal::new();

    files
        .into_par_iter()
        .for_each_with(sender, move |sender, path| {
            let maybe_parse = |path| {
                let c = match fs::read(path) {
                    Ok(content) => content,
                    Err(_) => return None,
                };

                let source = String::from_utf8_lossy(&c);

                let potential_match = work.iter().any(|WorkItem { qt: _, identifiers }| {
                    identifiers.iter().all(|i| source.find(i).is_some())
                });

                if !potential_match {
                    None
                } else {
                    let mut parser = tl
                        .get_or(|| RefCell::new(weggli::get_parser(is_cpp)))
                        .borrow_mut();
                    let tree = parser.parse(&source.as_bytes(), None).unwrap();
                    Some((tree, source.to_string()))
                }
            };
            if let Some((source_tree, source)) = maybe_parse(&path) {
                sender
                    .send((
                        std::sync::Arc::new(source),
                        source_tree,
                        path.display().to_string(),
                    ))
                    .unwrap();
            }
        });
}
