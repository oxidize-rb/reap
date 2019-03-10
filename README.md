# reap

A tool for parsing Ruby heap dumps.

Builds a [dominator tree](https://en.wikipedia.org/wiki/Dominator_(graph_theory)) from the reference graph showing which objects are holding on to large quantities of memory.

(Node `v` "dominates" node `w` in a directed graph if all paths from a given root to `w` run through `v`. In the context of memory references, this implies that object `w` is only live because object `v` is live.)

Supports drilldown into subtrees and optional graphical output.

# Use 

Basic usage:

```sh
$ cargo run -q --release -- /tmp/heap.json -d out.dot -n 3
Object types using the most live memory:
Thread: 2.1 MB (40 objects)
String: 462.6 KB (9235 objects)
Class: 223.7 KB (287 objects)
...: 653.0 KB (5909 objects)

Objects retaining the most live memory:
root: 3.4 MB (15472 objects)
Thread[0x7f83df87dc40]: 1.1 MB (25 objects)
Thread[0x7f83e107cd78]: 1.0 MB (7 objects)
...: 4.6 MB (59857 objects)

Object types retaining the most live memory:
ROOT: 3.4 MB (15472 objects)
Thread: 2.1 MB (70 objects)
ARRAY: 949.3 KB (13053 objects)
...: 3.6 MB (46766 objects)

Objects unreachable from root:
Class: 189.6 KB (617 objects)
String: 81.8 KB (1174 objects)
ARRAY: 38.6 KB (298 objects)
...: 91.5 KB (1422 objects)

Wrote 33 nodes & 32 edges to out.dot
```

Dig into a subtree (in this case, the larger Thread):

```sh
$ cargo run -q --release -- /tmp/heap.json -d out.dot -n 3 -r 0x7f83df87dc40
Object types using the most live memory:
Thread: 1.0 MB (1 objects)
Class: 1.6 KB (3 objects)
Hash: 1.3 KB (7 objects)
...: 980 B (14 objects)

Objects retaining the most live memory:
Thread[0x7f83df87dc40]: 1.1 MB (25 objects)
Hash[0x7f83e10452d8][size=5]: 1.2 KB (6 objects)
Object[0x7f83df8d62c8][CLASS]: 992 B (8 objects)
...: 3.0 KB (24 objects)

Object types retaining the most live memory:
Thread: 1.1 MB (25 objects)
Hash: 2.2 KB (12 objects)
Class: 1.9 KB (10 objects)
...: 1.1 KB (16 objects)

Objects reachable from, but not dominated by, 0x7f83df87dc40:
String: 352.3 KB (6604 objects)
Class: 220.6 KB (283 objects)
Regexp: 108.8 KB (139 objects)
...: 465.2 KB (5716 objects)

Wrote 1 nodes & 0 edges to out.dot
```

Run with `--help` for full options.

# Getting a heap dump

If you have `rbtrace` installed, and required in the process you're planning to trace, you can run:

```sh
rbtrace -p $PID -e "Thread.new{require 'objspace';f=open('/tmp/heap.json','w');ObjectSpace.dump_all(output: f, full: true);f.close}"
```

Otherwise, you can connect to the Ruby process with `gdb`, then run:

```gdb
call rb_eval_string_protect("Thread.new{require 'objspace';f=open('/tmp/heap.json','w');ObjectSpace.dump_all(output: f, full: true);f.close}", 0)
```
