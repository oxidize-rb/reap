# reap

A tool for parsing Ruby heap dumps.

# Use 

Just pass the heap dump as the first argument.

For example, if `/tmp/heap.json` is a heap dump produced by `ObjectSpace.dump_all` (see below),
you can run the following from the root of this repository:

```sh
RUSTFLAGS='-C target-cpu=native' cargo run --release /tmp/heap.json
```

(`target-cpu=native` isn't necessary, but might help performance.)

# Getting a heap dump

If you have `rbtrace` installed, and required in the process you're planning to trace, you can run:

```sh
rbtrace -p $PID -e "Thread.new{require 'objspace';io=open('/tmp/heap.json', 'w');ObjectSpace.dump_all(output: io);io.close}"
```

Otherwise, you can connect to the Ruby process with `gdb`, then run:

```gdb
call rb_eval_string_protect("Thread.new{require 'objspace';io=open('/tmp/heap.json', 'w');ObjectSpace.dump_all(output: io);io.close}", 0)
```
