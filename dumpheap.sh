#!/bin/bash

gdb --batch --command=src/dumpheap.gdb --se $1 --pid $2
