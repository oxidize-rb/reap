extern crate bytesize;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate serde;
extern crate petgraph;
extern crate serde_json;
extern crate timed_function;

mod analyze;
mod object;
mod parse;

use crate::object::*;
use bytesize::ByteSize;
use petgraph::dot;
use std::fmt::Display;
use std::fs::File;
use std::io::prelude::*;

fn write_dot_file(graph: &ReferenceGraph, filename: &str) -> std::io::Result<()> {
    let mut file = File::create(filename)?;
    write!(
        file,
        "{}",
        dot::Dot::with_config(&graph, &[dot::Config::EdgeNoLabel])
    )?;
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

fn parse(file: &str, rooted_at: Option<usize>) -> std::io::Result<analyze::Analysis> {
    let (root, graph) = parse::parse(&file)?;

    let subgraph_root = rooted_at
        .map(|address| {
            graph
                .node_indices()
                .find(|i| graph[*i].address == address)
                .expect("Given subtree root address not found")
        })
        .unwrap_or(root);

    Ok(analyze::analyze(root, subgraph_root, graph))
}

fn main() -> std::io::Result<()> {
    let args = clap_app!(reap =>
        (version: "0.1")
        (about: "A tool for parsing Ruby heap dumps.")
        (@arg INPUT: +required "Path to JSON heap dump file")
        (@arg DOT: -d --dot +takes_value "Dot file output for dominator tree")
        (@arg ROOT: -r --root +takes_value "Filter to subtree rooted at object with this address")
        (@arg THRESHOLD: -t --threshold +takes_value "Include nodes retaining at least this fraction of memory in dot output (defaults to 0.005)")
        (@arg COUNT: -n --top-n +takes_value "Print this many of the types & objects retaining the most memory")
    )
    .get_matches();

    let input = args.value_of("INPUT").unwrap();
    let dot_output = args.value_of("DOT");
    let subtree_root = args
        .value_of("ROOT")
        .map(|r| parse::parse_address(r).expect("Invalid subtree root address"));
    let relevance_threshold: f64 = args
        .value_of("THRESHOLD")
        .map(|t| t.parse().expect("Invalid relevance threshold"))
        .unwrap_or(0.005);
    let top_n: usize = args
        .value_of("COUNT")
        .map(|t| t.parse().expect("Invalid top-n count"))
        .unwrap_or(10);

    let analysis = parse(&input, subtree_root)?;
    println!();

    println!("Object types using the most live memory:");
    let (largest, rest) = analysis.live_stats_by_kind(top_n);
    print_largest(&largest, rest);

    println!("\nObjects retaining the most live memory:");
    let (largest, rest) = analysis.dominator_subtree_stats(top_n);
    print_largest(&largest, rest);

    println!("\nObject types retaining the most live memory:");
    let (largest, rest) = analysis.retained_stats_by_kind(top_n);
    print_largest(&largest, rest);

    if subtree_root.is_none() {
        println!("\nObjects unreachable from root:");
        let (largest, rest) = analysis.unreachable_stats_by_kind(top_n);
        print_largest(&largest, rest);
    } else {
        println!(
            "\nObjects reachable from, but not dominated by, {}:",
            args.value_of("ROOT").unwrap()
        );
        let (largest, rest) = analysis.unreachable_stats_by_kind(top_n);
        print_largest(&largest, rest);
    }

    if let Some(output) = dot_output {
        let dom_graph = analysis.relevant_dominator_subgraph(relevance_threshold);
        write_dot_file(&dom_graph, &output)?;
        println!(
            "\nWrote {} nodes & {} edges to {}",
            dom_graph.node_count(),
            dom_graph.edge_count(),
            &output
        );
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn whole_heap() {
        let analysis = parse("test/heap.json", None).unwrap();

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
        assert_eq!(1174, dead_strs.count);
        assert_eq!(462583, live_strs.bytes);
        assert_eq!(81799, dead_strs.bytes);
        assert_eq!(9408, retained_strs.count);
        assert_eq!(486278, retained_strs.bytes);

        let dom_graph = analysis.relevant_dominator_subgraph(0.005);
        assert_eq!(33, dom_graph.node_count());
        assert_eq!(32, dom_graph.edge_count());
    }

    #[test]
    fn subtree() {
        let analysis = parse("test/heap.json", Some(140204367666240)).unwrap();

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
}
