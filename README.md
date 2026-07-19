# Memory Analyser
Takes a core dump of a process, and walks memory (starting from command-line specified variables) to attribute
memory usage to locations in the program's data structures.

Example output showing memory efficiency in mrustc's "hello, world" test:
```
$ cargo run --release ~/Projects/mrustc/mrustc-1-Expanded.dmp main/crate --output output.txt > /dev/null
   Compiling memory_analyser_linux v0.1.0 (/home/tpg/Projects/mrustc/tools/memory_analyser_linux)
    Finished `release` profile [optimized + debuginfo] target(s) in 9.10s
     Running `target/release/memory_analyser_linux /home/tpg/Projects/mrustc/mrustc-1-Expanded.dmp main/crate --output output.txt`
Type not populated: TypeRef(26715) = Some(((6, UnitOffset(152)), TypeRef(26715)))
91.0% memory visited (303 MiB / 332 MiB)
```

```
$ less output.txt

enum counts: {
  ...
  "HIR::ValueItem": [368 B * 285978, max 360 B, 85 MiB wasted 85.6%] {
    "StructConstant": 63 (* 8 = 504 B, 22176 B waste),
    "StructConstructor": 353 (* 8 = 2824 B, 121 KiB waste),
    "Static": 4140 (* 360 = 1455 KiB, 0 B waste),
    "Constant": 9955 (* 280 = 2722 KiB, 777 KiB waste),
    "Function": 12669 (* 352 = 4354 KiB, 98 KiB waste),
    "Import": 258798 (* 16 = 4043 KiB, 84 MiB waste),
  }
}
top-level type counts: {
  [...]
  "MIR::BasicBlock": 148976 (* 192 = 27933 KiB),
  "Ident::Hygiene::Inner": 174543 (* 40 = 6818 KiB),
  "HIR::VisEnt<HIR::ValueItem>": 285978 (* 384 = 104 MiB),
  "std::pair<const RcString, std::unique_ptr<HIR::VisEnt<HIR::ValueItem>, std::default_delete<HIR::VisEnt<HIR::ValueItem> > > >": 285978 (* 16 = 4468 KiB),
}
annotated usage: {
  "" = 273 MiB,
  ".crate" = 273 MiB,
  ".crate.m_extern_crates" = 273 MiB,
  ".crate.m_extern_crates[3]" = 199 MiB,
  ".crate.m_extern_crates[3].second" = 199 MiB,
  ".crate.m_extern_crates[10]" = 22453 KiB,
  ".crate.m_extern_crates[10].second" = 22453 KiB,
  ".crate.m_extern_crates[5]" = 15183 KiB,
  ".crate.m_extern_crates[5].second" = 15183 KiB,
  ".crate.m_extern_crates[0]" = 14787 KiB,
  ...
  ".crate.m_root_module" = 10472 B,
  ".crate.m_root_module.m_items" = 9016 B,
  ".crate.m_root_module.m_items[0]" = 7080 B,
  ".crate.m_root_module.m_items[2]" = 1000 B,
  ".crate.m_root_module.m_items[1]" = 936 B,
}
```

This shows that:
- The `HIR::ValueItem` type wastes 84MiB because normally it only stores `Import` (i.e. a `use`, 16 bytes), but has space for `Static` (360 bytes)
- The third loaded crate (TODO: have map keys be rendered nicely) is nearly 200MB of the total ~270MB usage for the `crate` variable


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
- Duplicate string detection
- Heap walking (determine the amount of allocated memory by inspecting the heap directly)
- Memory fragmentation estimate (free/allocated/used space counts)
