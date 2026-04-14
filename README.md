# Memory Analyser
Takes a core dump of a process, and walks memory (starting from command-line specified variables) to attribute
memory usage to locations in the program's data structures.

A generalised linux port of mrustc's `memory_analyser` tool (which is MSVC-only)

# Usage
1. Get a core dump, e.g. by running `generate-core-file` in `gdb`
    ```
    gdb --args ./target/debug/rcc ./samples/while_loop.c
    (gdb) break main.rs:67
    Breakpoint 1 at 0x3fdfac: main.rs:67. (2 locations)
    (gdb) run
    Breakpoint 1.1, rcc::main () at src/main.rs:78
    78		for (name,ty,sym) in program.iter_symbols()
    (gdb) generate-core-file
    warning: Memory read failed for corefile section, 4096 bytes at 0xffffffffff600000.
    Saved corefile core.2376326
    ```
2. Run memory-analyser (suggest piping stdout to a file, as that's all of the detailed/debug data)
   - e.g. `cargo run --release ~/Projects/rust-cc/core.2376326 _RNvCsiGz8dyu0YhW_3rcc4main/program > 1.txt`
   - The mangled function names can be found in the output file

# Output statistics
- Tagged union variant counts (e.g. rust's enums, or mrustc's `TAGGED_UNION` types)
- Memory usage associated to variables/members (similar to how the `du` command works for filesystems)
- Total memory usage (visited and size of anonymous mappings)

# Future work
- Improved output (sort by counts, include variant/type sizes in output)
- Duplicate string detection
- Heap walking (determine the amount of allocated memory by inspecting the heap directly)
- Memory fragmentation estimate (free/allocated/used space counts)
