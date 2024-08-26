extern crate bytesize;
extern crate inferno;
extern crate petgraph;
extern crate serde_json;
extern crate structopt;
extern crate timed_function;

mod analyze;
mod object;
mod parse;

use crate::object::*;
use bytesize::ByteSize;
use inferno::flamegraph;
use petgraph::dot;
use std::error;
use std::fmt::Display;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

type Result<T> = std::result::Result<T, Box<dyn error::Error>>;

fn write_dot_file(graph: &ReferenceGraph, filename: &Path) -> Result<()> {
    let mut file = File::create(filename)?;
    write!(
        file,
        "{}",
        dot::Dot::with_config(&graph, &[dot::Config::EdgeNoLabel])
    )?;
    Ok(())
}

fn write_flamegraph(lines: &[String], filename: &Path) -> Result<()> {
    let mut opts = flamegraph::Options::default();
    opts.direction = flamegraph::Direction::Inverted;
    opts.count_name = "bytes".to_string();

    let file = File::create(filename)?;
    flamegraph::from_lines(&mut opts, lines.iter().map(|s| s.as_str()), file).unwrap();
    Ok(())
}

fn write_folded(lines: &[String], filename: &Path) -> Result<()> {
    let file = File::create(filename)?;
    let mut writer = std::io::BufWriter::new(file);
    for line in lines {
        writeln!(writer, "{}", line)?;
    }
    Ok(())
}

fn print_largest<K: Display>(largest: &[(K, Stats)], rest: Stats) {
    if largest.is_empty() {
        println!("None");
        return;
    }

    for (k, stats) in largest {
        println!(
            "{}: {} ({} objects)",
            k,
            ByteSize(stats.bytes as u64),
            stats.count
        );
    }

    if rest.count > 0 {
        println!(
            "...: {} ({} objects)",
            ByteSize(rest.bytes as u64),
            rest.count
        );
    }
}

fn parse(
    file: &Path,
    rooted_at: Option<usize>,
    class_name_only: bool,
) -> std::io::Result<analyze::Analysis> {
    let file = File::open(file)?;
    let mut reader = BufReader::new(file);
    let (root, graph) = parse::parse(&mut reader, class_name_only)?;

    let subgraph_root = rooted_at
        .map(|address| {
            graph
                .node_indices()
                .find(|i| graph[*i].address == address)
                .expect("Given subtree root address not found")
        })
        .unwrap_or(root);

    Ok(analyze::analyze(
        root,
        subgraph_root,
        graph,
        class_name_only,
    ))
}

#[derive(StructOpt, Debug)]
#[structopt(name = "reap")]
struct Opt {
    /// Path to JSON heap dump file to process
    #[structopt(name = "INPUT", parse(from_os_str))]
    input: PathBuf,

    /// Filter to subtree rooted at object with this address
    #[structopt(short, long)]
    root: Option<String>,

    /// Flamegraph SVG output for dominator tree
    #[structopt(short, long, parse(from_os_str))]
    flamegraph: Option<PathBuf>,

    /// Folded stack output for dominator tree
    #[structopt(long, parse(from_os_str))]
    folded: Option<PathBuf>,

    /// Dot file output for dominator tree
    #[structopt(short, long, parse(from_os_str))]
    dot: Option<PathBuf>,

    /// Include nodes retaining at least this fraction of memory in dot output
    #[structopt(short, long, default_value = "0.005")]
    threshold: f64,

    /// Print this many of the types & objects retaining the most memory
    #[structopt(short, long, default_value = "10")]
    count: usize,

    /// Remove address from flamegraph labels
    #[structopt(long = "class-name-only")]
    class_name_only: bool,
}

fn main() -> Result<()> {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    println!("reap v{}", VERSION);

    let opt = Opt::from_args();

    let subtree_root = opt
        .root
        .map(|r| parse::parse_address(r.as_str()).expect("Invalid subtree root address"));

    let class_name_only = opt.class_name_only;

    let analysis = parse(opt.input.as_path(), subtree_root, class_name_only)?;
    println!();

    println!("Object types using the most live memory:");
    let (largest, rest) = analysis.live_stats_by_kind(opt.count);
    print_largest(&largest, rest);

    println!("\nObjects retaining the most live memory:");
    let (largest, rest) = analysis.dominator_subtree_stats(opt.count);
    print_largest(&largest, rest);

    println!("\nObject types retaining the most live memory:");
    let (largest, rest) = analysis.retained_stats_by_kind(opt.count);
    print_largest(&largest, rest);

    if subtree_root.is_none() {
        println!("\nObjects unreachable from root:");
        let (largest, rest) = analysis.unreachable_stats_by_kind(opt.count);
        print_largest(&largest, rest);
    } else {
        println!(
            "\nObjects reachable from, but not dominated by, {}:",
            subtree_root.unwrap(),
        );
        let (largest, rest) = analysis.unreachable_stats_by_kind(opt.count);
        print_largest(&largest, rest);
    }

    if let Some(output) = opt.flamegraph {
        let lines = analysis.flamegraph_lines();
        write_flamegraph(&lines, output.as_path())?;
        println!("\nWrote {} nodes to {}", lines.len(), output.display());
    }

    if let Some(output) = opt.folded {
        let lines = analysis.flamegraph_lines();
        write_folded(&lines, output.as_path())?;
        println!("\nWrote {} nodes to {}", lines.len(), output.display());
    }

    if let Some(output) = opt.dot {
        let dom_graph = analysis.relevant_dominator_subgraph(opt.threshold.abs());
        write_dot_file(&dom_graph, output.as_path())?;
        println!(
            "\nWrote {} nodes & {} edges to {}",
            dom_graph.node_count(),
            dom_graph.edge_count(),
            output.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::rstest;
    #[rstest]
    #[case(false)]
    #[case(true)]
    fn whole_heap(#[case] class_name_only: bool) {
        let analysis = parse(Path::new("test/heap.json"), None, class_name_only).unwrap();

        let totals = analysis.dominated_totals();
        assert_eq!(15472, totals.count);
        assert_eq!(3439119, totals.bytes);

        let (live_by_kind, _) = analysis.live_stats_by_kind(usize::max_value());
        let (dead_by_kind, _) = analysis.unreachable_stats_by_kind(usize::max_value());
        let (retained_by_kind, _) = analysis.retained_stats_by_kind(usize::max_value());

        let live_strs = live_by_kind.iter().find(|(k, _)| *k == "String").unwrap().1;
        let dead_strs = dead_by_kind.iter().find(|(k, _)| *k == "String").unwrap().1;
        let retained_strs = retained_by_kind
            .iter()
            .find(|(k, _)| *k == "String")
            .unwrap()
            .1;

        assert_eq!(9235, live_strs.count);
        assert_eq!(1175, dead_strs.count);
        assert_eq!(462583, live_strs.bytes);
        assert_eq!(81839, dead_strs.bytes);
        assert_eq!(9408, retained_strs.count);
        assert_eq!(486278, retained_strs.bytes);

        let dom_graph = analysis.relevant_dominator_subgraph(0.005);
        assert_eq!(33, dom_graph.node_count());
        assert_eq!(32, dom_graph.edge_count());
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn subtree(#[case] class_name_only: bool) {
        let analysis = parse(
            Path::new("test/heap.json"),
            Some(140204367666240),
            class_name_only,
        )
        .unwrap();

        let totals = analysis.dominated_totals();
        assert_eq!(25, totals.count);
        assert_eq!(1053052, totals.bytes);

        let (live_by_kind, _) = analysis.live_stats_by_kind(usize::max_value());
        let (dead_by_kind, _) = analysis.unreachable_stats_by_kind(usize::max_value());
        let (retained_by_kind, _) = analysis.retained_stats_by_kind(usize::max_value());

        let live_strs = live_by_kind.iter().find(|(k, _)| *k == "String").unwrap().1;
        let dead_strs = dead_by_kind.iter().find(|(k, _)| *k == "String").unwrap().1;
        let retained_strs = retained_by_kind
            .iter()
            .find(|(k, _)| *k == "String")
            .unwrap()
            .1;

        assert_eq!(4, live_strs.count);
        assert_eq!(6604, dead_strs.count);
        assert_eq!(208, live_strs.bytes);
        assert_eq!(352283, dead_strs.bytes);
        assert_eq!(4, retained_strs.count);
        assert_eq!(208, retained_strs.bytes);

        let dom_graph = analysis.relevant_dominator_subgraph(0.0);
        assert_eq!(25, dom_graph.node_count());
        assert_eq!(24, dom_graph.edge_count());
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn flamegraph_lines_output(#[case] class_name_only: bool) {
        let analysis = parse(Path::new("test/heap.json"), None, class_name_only).unwrap();
        let frame_lines = analysis.flamegraph_lines();
        let lines_with_memory_addresses = frame_lines.iter().filter(|&l| l.contains("0x")).count();
        if class_name_only {
            assert_eq!(lines_with_memory_addresses, 125);
        } else {
            assert_eq!(lines_with_memory_addresses, frame_lines.len());
        }
    }
}
