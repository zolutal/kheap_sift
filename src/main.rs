use weggli::{parse_search_pattern, RegexMap};
use std::collections::HashSet;
use std::collections::HashMap;
use std::path::PathBuf;
use std::fs::File;
use memmap2::Mmap;
use regex::Regex;
use clap::Parser;
use dwat::Dwarf;

mod wegg;

#[derive(Parser)]
struct CmdArgs {
    vmlinux_path: PathBuf,
    source_path: PathBuf,
    lower_bound: usize,
    upper_bound: usize,
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
    let files: Vec<PathBuf> = wegg::iter_files(&args.source_path, extensions);

    // regex constraints contains : ("$varname", (negative, regex))
    let mut regex_map = HashMap::new();
    let regex = Regex::new("\\bk[mvz]alloc")?;
    regex_map.extend(vec![("$alloc".to_string(), (false, regex))]);
    let regex_constraints = RegexMap::new(regex_map);

    for (name, _struct) in struct_map {
        let query = format!("\
        {{
            struct {name} *$var;
            $var = $alloc(_);
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
        wegg::weggling_time(&work, files.clone(), name, _struct, &dwarf)
    }
    Ok(())
}
