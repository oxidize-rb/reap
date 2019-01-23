# reap

A tool for parsing Ruby heap dumps.

Builds a dominator tree from the reference graph showing which objects are holding on to large quantities of memory.

Optional graphical output.

# Use 

```sh
$ cargo run -q --release -- /tmp/heap.json -d out.dot -n 3
Object types using the most memory:
Thread: 2100424 bytes (41 objects)
String: 544382 bytes (10409 objects)
Class: 413352 bytes (904 objects)
...: 782503 bytes (7628 objects)

Objects retaining the most memory:
root: 3439119 bytes (15472 objects)
Thread[0x7f83df87dc40]: 1053052 bytes (25 objects)
Thread[0x7f83e107cd78]: 1049592 bytes (7 objects)
...: 4988696 bytes (63367 objects)

Wrote 33 nodes & 37 edges to out.dot
$
$
$ cargo run -q --release -- /tmp/heap.json -d out.dot -n 3 -r 0x7f83e107cd78
Object types using the most memory:
Thread: 1049160 bytes (1 objects)
Hash: 232 bytes (2 objects)
Proc: 80 bytes (1 objects)
...: 120 bytes (3 objects)

Objects retaining the most memory:
Thread[0x7f83e107cd78]: 1049592 bytes (7 objects)
Hash[0x7f83e107c850][size=1]: 192 bytes (1 objects)
Proc[0x7f83e107ccb0]: 120 bytes (2 objects)
...: 160 bytes (4 objects)

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
