#!/usr/bin/env ruby
#
# Script to generate bindings for all the Ruby versions we support.
#
# Currently assumes:
# - Presence of `rbenv` and (Rust) `bindgen` on PATH
# - 'subprocess' gem

require 'tmpdir'
require 'tempfile'
require 'subprocess'

class BindgenScript
  RUBY_VERSIONS = [
    [2,5,3],
    [2,4,1],
  ]

  WRAPPER_CONTENTS = <<~HEADER
    #include <vm_core.h>
    #include <iseq.h>
    #include <gc.h>
  HEADER

  RUBY_C_TYPES = %w{
    rb_iseq_constant_body
    rb_iseq_location_struct
    rb_thread_struct
    rb_thread_t
    rb_iseq_struct
    rb_control_frame_struct
    rb_thread_struct
    rb_execution_context_struct
    iseq_insn_info_entry
    RString
    RArray
    VALUE
  }

  OUT_DIR = File.expand_path('../src/bindings', __dir__)

  private def with_ruby_dir
    if ENV['RUBY_DIR']
      yield ENV['RUBY_DIR']
    else
      Dir.mktmpdir do |tmp|
        Subprocess.check_call(%w{git clone https://github.com/ruby/ruby.git}, cwd: tmp)
        yield File.join(tmp, 'ruby')
      end
    end
  end

  private def run_bindgen(ruby_dir, sys_headers_dir, out_file)
    wrapper_h = Tempfile.new(['wrapper', '.h'])
    File.write(wrapper_h, WRAPPER_CONTENTS)

    Subprocess.check_call([
      'bindgen',
      '--impl-debug',
      *RUBY_C_TYPES.flat_map {|t| ['--whitelist-type', t]},
      '-o', out_file,
      wrapper_h.to_path,
      '--',
      '-I', ruby_dir,
      '-I', File.join(ruby_dir, 'include'),
      '-I', sys_headers_dir,
    ], cwd: ruby_dir)
  end

  private def setup_ruby(version, ruby_dir)
    major, minor, patch = version
    branch = "v#{major}_#{minor}_#{patch}"
    version_name = "#{major}.#{minor}.#{patch}"

    Subprocess.check_call(%w{git checkout} << branch, cwd: ruby_dir)

    makefile = File.join(ruby_dir, 'Makefile')
    this_version_regex = /RUBY_PROGRAM_VERSION.*#{version_name}/
    if !File.exists?(makefile) || File.readlines(makefile).grep(this_version_regex).empty?
      Subprocess.check_call(%w{autoconf}, cwd: ruby_dir)
      Subprocess.check_call(%w{sh configure}, cwd: ruby_dir)
    end

    Subprocess.check_call(%w{make id.h}, cwd: ruby_dir)

    Subprocess.check_call(%w{rbenv install -s} << version_name)

    glob = File.join(
      ENV['RBENV_ROOT'],
      'versions',
      version_name,
      'include',
      "ruby-#{major}.#{minor}.0",
      "x86_64-*",
    )
    Dir[glob].first
  end

  def main
    with_ruby_dir do |ruby_dir|
      RUBY_VERSIONS.each do |version|
        out_file = File.join(OUT_DIR, version.join('_') + '.rs')
        $stderr.puts("Generating #{out_file} ...")
        sys_headers_dir = setup_ruby(version, ruby_dir)
        run_bindgen(ruby_dir, sys_headers_dir, out_file)
        $stderr.puts("Generated #{out_file}\n\n")
      end
    end
  end
end

if __FILE__ == $0
  BindgenScript.new.main()
end
