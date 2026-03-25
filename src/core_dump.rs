pub struct CoreDump {
    modules: Vec<ReferencedFile>,
    memory_ranges: Vec<MemoryRange>,
    /// Size of a compressed memory chunk, nominally 1MiB
    chunk_size: u64,
    /// Offset of the start of each chunk
    file_chunks: Vec<u64>,

    // TODO: thread state
}
struct ReferencedFile {
    base: u64,
    path: ::std::path::PathBuf,
}
struct MemoryRange {
    /// Location of backing data in the core dump (logical position, starting from the first compressed chunk)
    first_chunk: u64,
    /// Virtual memory address of start of this range
    v_start: u64,
    /// Size of this range in bytes
    size: u64,
    // /// Memory flags
    // flags: u8,
    /// Source data offset in named file
    file_ofs: u64,
    /// Source file
    file_path: String,
}
impl CoreDump {
    pub fn open(path: &std::path::Path) -> CoreDump {
        // Header:
        // - Magic text
        // - Number of ranges
        // - Number of chunks
        // - Chunk size (bytes)
        
        // Memory ranges
        // Chunks (decompress, but don't save)
        // Current thread register dump
        CoreDump {
            modules: vec![
                ReferencedFile { base: 0, path: "/home/tpg/Projects/mrustc/bin/mrustc".into() }
            ],
            memory_ranges: Vec::new(),
            chunk_size: 1<<20,
            file_chunks: Vec::new()
        }
    }

    pub fn modules(&self) -> impl Iterator<Item=(::std::path::PathBuf,u64)> {
        self.modules.iter().map(|v| (v.path.clone(), v.base))
    }

    pub fn get_thread(&self, index: usize) -> &crate::CpuState {
        todo!("get_thread")
    }

    pub fn read_bytes(&self, addr: u64, dst: &mut [u8]) {
        assert!(dst.len() < 16);
        for r in &self.memory_ranges {
            if r.v_start <= addr && addr < r.v_start + r.size {
                // Correct range, now get the chunk
                let ofs = addr - r.v_start;
                let chunk_idx = (r.first_chunk + ofs / self.chunk_size) as usize;
                let chunk_ofs = (ofs % self.chunk_size) as usize;
                self.with_chunk(chunk_idx, |chunk| {
                    let l = dst.len();
                    dst.copy_from_slice(&chunk[chunk_ofs..][..l]);
                });
                return
            }
        }
    }

    fn with_chunk(&self, index: usize, cb: impl FnOnce(&[u8])) {
    }
}