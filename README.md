# reap

A tool for parsing Ruby heap dumps.

Builds a dominator tree from the reference graph showing which objects are holding on to large quantities of memory.

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

Objects unreachable from root
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

Wrote 1 nodes & 0 edges to out.dot
```

Run with `--help` for full options.

# Getting a heap dump

If you have `rbtrace` installed, and required in the process you're planning to trace, you can run:

```sh
rbtrace -p $PID -e "Thread.new{require 'objspace';io=open('/tmp/heap.json', 'w');GC.start;ObjectSpace.dump_all(output: io, full: true);io.close}"
```

Otherwise, you can connect to the Ruby process with `gdb`, then run:

```gdb
call rb_eval_string_protect("Thread.new{require 'objspace';io=open('/tmp/heap.json', 'w');GC.start;ObjectSpace.dump_all(output: io, full: true);io.close}", 0)
```
