
pub struct CoreDump {
    modules: Vec<ReferencedFile>,
    memory_ranges: Vec<MemoryRange>,
    
    // NOTE: The virual memory is broken into chunks of `chunk_size`, with empty chunks not stored
    /// Size of a compressed memory chunk, nominally 1MiB
    chunk_size: u64,
    threads: Vec<super::CpuState>,

    /// Offset of the start of each chunk
    file_chunks: Vec<u64>,
    chunk_cache: ChunkCache,
}
struct ReferencedFile {
    base: u64,
    file_base: u64,
    path: ::std::path::PathBuf,
}
struct MemoryRange {
    /// Location of backing data in the core dump (logical position, starting from the first compressed chunk)
    /// - The data is stored at offset `v_start % chunk_size` in this chunk
    first_chunk: usize,
    /// Virtual memory address of start of this range
    v_start: u64,
    /// Size of this range in bytes
    size: u64,
    // /// Memory flags
    // flags: u8,
    /// Source data offset in named file
    _file_ofs: u64,
    ///// Source file
    //file_path: String,

    /// Indicates that this is an anonoymous binding (i.e. no backing file, i.e. it's dynamic memory)
    is_anon: bool,
}
impl CoreDump {
    pub fn new_stub() -> CoreDump {
        CoreDump {
            chunk_cache: ChunkCache::new(::std::io::BufReader::new(::std::fs::File::open("/dev/null").unwrap()), 0),
            modules: vec![
                ReferencedFile {
                    base: 0,
                    file_base: 0,
                    path: "/home/tpg/Projects/mrustc/bin/mrustc".into(),
                }
            ],
            memory_ranges: Vec::new(),
            chunk_size: 1 << 20,
            file_chunks: Vec::new(),
            threads: vec![
                super::CpuState::stub()
            ]
        }
    }
    pub fn open(path: &std::path::Path) -> ::std::io::Result<CoreDump> {
        let mut fp = ::std::io::BufReader::new(::std::fs::File::open(path)?);
        // Header
        let header = raw::FileHeader::from_reader(&mut fp)?;
        header.check_magic()?;
        println!("{:?}", header);
        // Memory ranges
        let mut memory_ranges = Vec::with_capacity(header.n_ranges as usize);
        let mut modules = Vec::<ReferencedFile>::new();
        let mut n_chunks = 0;
        let mut last_v_chunk = 0;
        let mut last_end = 0;
        for _ in 0 .. header.n_ranges {
            let hdr = raw::MemoryRangeHeader::from_reader(&mut fp)?;
            let name = {
                let mut b = vec![0; hdr.name_length as usize];
                ::std::io::Read::read_exact(&mut fp, &mut b)?;
                String::from_utf8(b).unwrap()
            };
            let this_chunk = hdr.v_start / header.chunk_size as u64;
            let this_end = hdr.v_start + hdr.size;
            let next_chunk = this_end / header.chunk_size as u64;
            if last_v_chunk != this_chunk {
                // There's been a gap in chunks, so fix alignment
                if last_end % header.chunk_size as u64 != 0 {
                    n_chunks += 1;
                }
            }
            println!("C{}/{} {:#x} + {:#x} from {:#x} flags={:#x}: {:?}", n_chunks, header.n_chunks, hdr.v_start, hdr.size, hdr.file_ofs, hdr._flags, name);
            last_end = this_end;
            last_v_chunk = next_chunk;

            memory_ranges.push(MemoryRange {
                _file_ofs: hdr.file_ofs,
                first_chunk: n_chunks,
                size: hdr.size,
                v_start: hdr.v_start,
                is_anon: name.is_empty(),
            });
            n_chunks += (next_chunk - this_chunk) as usize;

            if name != "" && name.starts_with("/") {
                // Add the module
                if let Some(v) = modules.iter_mut().find(|v| v.path == name) {
                    if hdr.v_start < v.base {
                        v.base = hdr.v_start;
                        v.file_base = hdr.file_ofs;
                    }
                }
                else {
                    modules.push(ReferencedFile { base: hdr.v_start, file_base: hdr.file_ofs, path: name.into() });
                }
            }
        }
        // There's been a gap in chunks, so fix alignment
        if last_end % header.chunk_size as u64 != 0 {
            n_chunks += 1;
        }
        assert!(n_chunks == header.n_chunks as usize);
        // Chunks (decompress, but don't save)
        let file_chunks = {
            let mut chunks = Vec::with_capacity(header.n_chunks as usize);
            let mut empty_chunk = vec![0; header.chunk_size as usize];
            for _i in 0 .. header.n_chunks {
                use ::std::io::Seek;
                chunks.push(fp.seek(::std::io::SeekFrom::Current(0))?);
                println!("Chunk {} @ {}", _i, chunks.last().unwrap());
                let _chunk_base = raw::read_chunk(&mut fp, &mut empty_chunk)?;
            }
            chunks
        };
        // Current thread register dump
        let mut threads = Vec::new();
        for _ in 0 .. 1 {
            threads.push(super::CpuState {
                pc: raw::read_u64(&mut fp)?,
                gprs: [
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                    raw::read_u64(&mut fp)?,
                ]
            });
        }

        Ok(CoreDump {
            chunk_cache: ChunkCache::new(fp, header.chunk_size as usize),
            modules,
            memory_ranges,
            chunk_size: header.chunk_size as u64,
            file_chunks,
            threads,
        })
    }

    /// Sum the size of all anon (no backing file) mappings
    pub fn anon_size(&self) -> usize {
        self.memory_ranges.iter().map(|v| if v.is_anon { v.size } else { 0 }).sum::<u64>() as usize
    }

    pub fn modules(&self) -> impl Iterator<Item=(::std::path::PathBuf,u64,u64)> {
        self.modules.iter().map(|v| (v.path.clone(), v.base, v.file_base))
    }

    pub fn get_thread(&self, index: usize) -> &crate::CpuState {
        &self.threads[index]
    }

    pub fn read_bytes(&self, addr: u64, dst: &mut [u8]) {
        assert!(dst.len() <= 16);
        for r in &self.memory_ranges {
            if r.v_start <= addr && addr < r.v_start + r.size {
                // Correct range, now get the chunk
                let ofs = addr - r.v_start;
                let chunk_idx = r.first_chunk + (ofs / self.chunk_size) as usize;
                let chunk_ofs = (addr % self.chunk_size) as usize;
                //println!("read_bytes: {:#x} -> {:#x}+{:#x} -> C{}+{:#x}", addr, r.v_start, ofs, chunk_idx, chunk_ofs);
                self.with_chunk(chunk_idx, |chunk| {
                    let l = dst.len();
                    dst.copy_from_slice(&chunk[chunk_ofs..][..l]);
                });
                return
            }
        }
        panic!("Out-of-bounds read: {:#x}+{}", addr, dst.len());
    }

    fn with_chunk(&self, index: usize, cb: impl FnOnce(&[u8])) {
        self.chunk_cache.with_chunk(self.file_chunks[index], cb);
    }
}

impl CoreDump {
    pub fn read_ptr(&self, addr: u64) -> u64 {
        let mut v = [0; 8];
        self.read_bytes(addr, &mut v);
        u64::from_le_bytes(v)
    }
    pub fn read_u32(&self, addr: u64) -> u32 {
        let mut v = [0; 4];
        self.read_bytes(addr, &mut v);
        u32::from_le_bytes(v)
    }
}

struct ChunkCache {
    fp: ::std::cell::RefCell<::std::io::BufReader<::std::fs::File>>,
    uses: ::std::cell::Cell<usize>,
    ents: Vec<::std::cell::RefCell<ChunkChacheEnt>>,
}
struct ChunkChacheEnt {
    ofs: u64,
    last_use: usize,
    data: Vec<u8>,
}
impl ChunkCache {
    fn new(fp: ::std::io::BufReader<::std::fs::File>, chunk_size: usize) -> Self {
        ChunkCache {
            fp: ::std::cell::RefCell::new(fp),
            uses: Default::default(),
            ents: (0 .. 16).map(|_| ::std::cell::RefCell::new(ChunkChacheEnt { ofs: 0, last_use: 0, data: vec![0; chunk_size] })).collect(),
        }
    }
    fn with_chunk(&self, ofs: u64, cb: impl FnOnce(&[u8])) {
        self.uses.set( self.uses.get() + 1 );
        let mut e = 'a: {
            for e in &self.ents {
                let e = e.borrow_mut();
                if e.ofs == ofs {
                    break 'a e;
                }
            }
            let oldest = self.ents.iter().min_by_key(|v| v.borrow().last_use).unwrap();
            let mut e = oldest.borrow_mut();
            let mut fp = self.fp.borrow_mut();
            use std::io::Seek;
            fp.seek(::std::io::SeekFrom::Start(ofs)).expect("Seek fail");
            let _chunk_base = raw::read_chunk(&mut *fp, &mut e.data).expect("Decompression failed, it passed earlier");
            e.ofs = ofs;
            e
        };
        e.last_use = self.uses.get();
        cb(&e.data);
    }
}

mod raw {
    #[derive(Debug)]
    pub struct FileHeader {
        pub magic: [u8; 12],
        /// Number of file mappings/ranges
        pub n_ranges: u32,
        /// Number of memory chunks
        pub n_chunks: u32,
        /// Size of a memory dump chunk (in bytes)
        pub chunk_size: u32,
    }
    impl FileHeader {
        pub fn from_reader(fp: &mut impl ::std::io::Read) -> ::std::io::Result<Self> {
            let mut header = [0; 12+4+4+4];
            fp.read_exact(&mut header)?;
            Ok(FileHeader {
                magic: header[..12].try_into().unwrap(),
                n_ranges: u32::from_le_bytes(header[12..][..4].try_into().unwrap()),
                n_chunks: u32::from_le_bytes(header[16..][..4].try_into().unwrap()),
                chunk_size: u32::from_le_bytes(header[20..][..4].try_into().unwrap()),
            })
        }
        pub fn check_magic(&self) -> ::std::io::Result<()> {
            if self.magic != *b"FullDump\x97\r\n\0" {
                Err(::std::io::Error::other("Bad magic string"))
            }
            else {
                Ok(())
            }
        }
    }

    #[derive(Debug)]
    pub struct MemoryRangeHeader {
        /// Virtual memory address of start of this range
        pub v_start: u64,
        /// Size of this range in bytes
        pub size: u64,
        /// Source data offset in named file
        pub file_ofs: u64,

        /// Length of the source file name (following this structure)
        pub name_length: u16,
        /// Various flags, TODO
        pub _flags: u16,
        _pad: [u16; 2],
    }
    impl MemoryRangeHeader {
        pub fn from_reader(fp: &mut impl ::std::io::Read) -> ::std::io::Result<Self> {
            Ok(MemoryRangeHeader {
                v_start: u64::from_le_bytes(read_bytes(fp)?),
                size: u64::from_le_bytes(read_bytes(fp)?),
                file_ofs: u64::from_le_bytes(read_bytes(fp)?),
                name_length: u16::from_le_bytes(read_bytes(fp)?),
                _flags: u16::from_le_bytes(read_bytes(fp)?),
                _pad: [u16::from_le_bytes(read_bytes(fp)?), u16::from_le_bytes(read_bytes(fp)?)],
            })
        }
    }

    pub fn read_chunk(fp: &mut (impl ::std::io::BufRead + ::std::io::Seek), dst: &mut Vec<u8>) -> ::std::io::Result<u64> {
        let addr = {
            let mut buf = [0; 8];
            fp.read_exact(&mut buf)?;
            u64::from_ne_bytes(buf)
        };
        println!("read_chunk: addr={:#x}", addr);
        let mut stream = ::flate2::bufread::ZlibDecoder::new(fp);
        match ::std::io::Read::read_exact(&mut stream, dst) {
        Ok(_) => {},
        Err(e) => {
            println!("@{}\n", stream.into_inner().seek(::std::io::SeekFrom::Current(0))?);
            return Err(e.into())
        },
        }
        match ::std::io::Read::read(&mut stream, &mut [0]) {
        Ok(0) => {},
        Ok(n) => panic!("Tailing data after compresed chunk: {}", n),
        Err(e) => {
            println!("@{}\n", stream.into_inner().seek(::std::io::SeekFrom::Current(0))?);
            return Err(e.into())
        },
        }
        Ok(addr)
    }

    fn read_bytes<const N: usize>(fp: &mut impl ::std::io::Read) -> ::std::io::Result<[u8; N]> {
        let mut v = [0; N];
        fp.read_exact(&mut v)?;
        Ok(v)
    }

    pub fn read_u64(fp: &mut impl ::std::io::Read) -> ::std::io::Result<u64> {
        Ok(u64::from_le_bytes(read_bytes(fp)?))
    }
}