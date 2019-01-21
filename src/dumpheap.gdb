define ruby_eval
  call((void) rb_p((unsigned long) rb_eval_string_protect($arg0,(int*)0)))
end

ruby_eval("require 'objspace'")
ruby_eval("ObjectSpace.dump_all(output: open('/tmp/heap.json','w'))")
