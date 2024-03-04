use weggli::{parse_search_pattern, RegexMap};
use std::collections::HashSet;
use std::collections::HashMap;
use clap::ArgAction::Append;
use std::path::PathBuf;
use std::fs::File;
use memmap2::Mmap;
use regex::Regex;
use clap::Parser;
use dwat::Dwarf;

mod wegg;

#[derive(Parser)]
struct CmdArgs {
    /// Path to vmlinux file
    #[clap(help = "The path to the vmlinux file.")]
    vmlinux_path: PathBuf,

    /// Path to Linux source code
    #[clap(help = "The path to the Linux source code directory.")]
    source_path: PathBuf,

    /// The lower bound for struct size
    #[clap(help = "The lower bound for struct sizes (inclusive).")]
    lower_bound: usize,

    /// The upper bound for struct size
    #[clap(help = "The upper bound for struct sizes (exclusive).")]
    upper_bound: usize,

    /// Silence dwat/weggli output, only print struct names
    #[clap(long, action, help = "Silence dwat/weggli output, only print struct \
                                 names.")]
    quiet: bool,

    /// Allocation flags flags argument regex
    #[clap(long, help = "Allocation flags argument regex")]
    flags: Option<String>,

    /// Glob to exclude files, can be specified multiple times to provide
    /// multiple globs
    #[clap(long, action=Append, help = "Glob to exclude files based on")]
    exclude: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let args = CmdArgs::parse();

    let file = File::open(args.vmlinux_path)?;
    let mmap = unsafe { Mmap::map(&file) }?;

    let dwarf = Dwarf::parse(&*mmap)?;
    let struct_map = dwarf.get_named_items_map::<dwat::Struct>()?;

    let struct_map: HashMap<String, dwat::Struct> = {
        struct_map.into_iter().filter(|(_, struc)| {
            if let Ok(bytesz) = struc.byte_size(&dwarf) {
                args.lower_bound <= bytesz && bytesz < args.upper_bound
            } else {
                false
            }
        }).collect()
    };

    let extensions = ["c", "h"].map(|s| s.to_string()).to_vec();
    let iter_files: Vec<PathBuf> = wegg::iter_files(&args.source_path, extensions);
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
                //println!("{:?}", &file);
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

    // regex constraints contains : ("$varname", (negative, regex))
    let mut regex_map = HashMap::new();
    let regex = Regex::new("\\bk[mvz]alloc")?;
    regex_map.extend(vec![("$alloc".to_string(), (false, regex))]);
    let regex_constraints = RegexMap::new(regex_map);

    let mut flags_regex = Regex::new(".*")?;
    if let Some(flags_arg) = args.flags {
        flags_regex = Regex::new(&flags_arg)?;
    }

    // query first for two argument matches
    for (name, _struct) in struct_map.clone() {
        let query = format!("\
        {{
            struct {name} *$var;
            $var = $alloc(_, $flags);
        }}");

        // build weggli query tree
        let qt = parse_search_pattern(
            &query,
            false, // is_cpp
            false, // force_query
            Some(regex_constraints.clone())
        ).expect("Failed to parse query");

        let mut variables = HashSet::new();
        variables.extend(qt.variables());

        let identifiers = qt.identifiers();
        let work = vec![wegg::WorkItem { qt, identifiers }];

        // weggle
        wegg::weggling_time(&work, files.clone(),
                            name, _struct, flags_regex.clone(),
                            &dwarf, args.quiet);
    }

    // query again for three argument matches
    for (name, _struct) in struct_map {
        let query = format!("\
        {{
            struct {name} *$var;
            $var = $alloc(_, _, $flags);
        }}");

        // build weggli query tree
        let qt = parse_search_pattern(
            &query,
            false, // is_cpp
            false, // force_query
            Some(regex_constraints.clone())
        ).expect("Failed to parse query");

        let mut variables = HashSet::new();
        variables.extend(qt.variables());

        let identifiers = qt.identifiers();
        let work = vec![wegg::WorkItem { qt, identifiers }];

        // weggle
        wegg::weggling_time(&work, files.clone(),
                            name, _struct, flags_regex.clone(),
                            &dwarf, args.quiet);
    }
    Ok(())
}
