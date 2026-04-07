# Memory Analyser
Offshoot of mrustc (a linux/ELF copy of the MSVC version in mrustc's source)

Loads:
- A custom full-memory core dump (because I can't find a good way) to get a standard core dump on demand.
- DWARF debug data

Then walks memory tree from a given variable, and extracts statistics from it (TODO)

# Proposed statisics
- tagged union variant counts
- memory usage owned by variables/members
- memory fragmentation (amount of memory touched during walk vs memory allocated)