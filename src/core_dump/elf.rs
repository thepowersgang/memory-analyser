//! Load a linux (ELF wrapepd) core dump file
//! 
//! Uses PT_LOAD segments to contain the data/flags

pub struct CoreDump {
    modules: Vec<super::ReferencedFile>,
    memory_ranges: Vec<MemoryRange>,
    threads: Vec<crate::CpuState>,
    fp: ::std::sync::Mutex< ::std::io::BufReader< ::std::fs::File > >,
}

impl CoreDump {
    pub fn open(fp: std::fs::File) -> Result<CoreDump, Box<dyn ::std::error::Error>> {
        let mut fp = ::std::io::BufReader::new(fp);
        let mut elf = ::elf::ElfStream::<::elf::endian::NativeEndian,_>::open_stream(&mut fp)?;
        let c = elf.ehdr.class;

        let mut memory_ranges = Vec::new();
        for s in elf.segments() {
            if s.p_type == ::elf::abi::PT_LOAD {
                memory_ranges.push(MemoryRange {
                    v_start: s.p_vaddr,
                    size: s.p_memsz,
                    dump_file_ofs: if s.p_offset != 0 {
                        RangeSource::Dump { offset: s.p_offset }
                    } else {
                        todo!("Reference data from outside the core dump")
                    },
                    is_anon: true,
                });
            }
        }

        let mut threads = Vec::new();
        let mut modules: Vec<super::ReferencedFile> = Vec::new();

        // Get the PT_NOTE section
        let note_segments: Vec<_> = elf.segments().iter().filter(|p| p.p_type == ::elf::abi::PT_NOTE).copied().collect();
        for mut s_note in note_segments {
            s_note.p_align = 4; // Force alignment, as this GDB core dump doesn't set it.
            for note in elf.segment_data_as_notes(&s_note)? {
                //println!("{:?}", note);
                let note = match note {
                    elf::note::Note::GnuAbiTag(..) => continue,
                    elf::note::Note::GnuBuildId(..) => continue,
                    elf::note::Note::Unknown(note_any) => note_any,
                };
                if note.name != b"CORE\0" {
                    continue;
                }
                match note.n_type {
                ::elf::abi::NT_PRPSINFO => {},  // don't care: Generic process information (pid, gid, filename, args, ...)
                ::elf::abi::NT_PRSTATUS => {    // Thread status, including GPRs
                    let mut slice = note.desc;
                    // REF: https://elixir.bootlin.com/linux/v4.7/source/include/uapi/linux/elfcore.h#L36
                    // REF: https://elixir.bootlin.com/linux/v4.7/source/arch/x86/include/asm/user_64.h#L68
                    let _ = [(); 14].map(|_| get_u64(&mut slice));
                    let r15 = get_u64(&mut slice);
                    let r14 = get_u64(&mut slice);
                    let r13 = get_u64(&mut slice);
                    let r12 = get_u64(&mut slice);
                    let bp = get_u64(&mut slice);
                    let bx = get_u64(&mut slice);
                    let r11 = get_u64(&mut slice);
                    let r10 = get_u64(&mut slice);
                    let r9 = get_u64(&mut slice);
                    let r8 = get_u64(&mut slice);
                    let ax = get_u64(&mut slice);
                    let cx = get_u64(&mut slice);
                    let dx = get_u64(&mut slice);
                    let si = get_u64(&mut slice);
                    let di = get_u64(&mut slice);
                    let _orig_ax = get_u64(&mut slice);
                    let ip = get_u64(&mut slice);
                    let _cs = get_u64(&mut slice);
                    let _flags = get_u64(&mut slice);
                    let sp = get_u64(&mut slice);
                    threads.push(crate::CpuState {
                        pc: ip,
                        gprs: [
                            ax, dx, cx, bx, si, di, bp, sp,
                            r8, r9, r10, r11, r12, r13, r14, r15,
                        ],
                    });
                },
                ::elf::abi::NT_FPREGSET => {},  // don't care: Floating point registers
                ::elf::abi::NT_SIGINFO => {},   // siginfo_t

                ::elf::abi::NT_AUXV => {},  // Auxillary vector (has extra things not covered by PRPSTATUS)
                ::elf::abi::NT_FILE => {    // List of mapped files
                    let mut slice = note.desc;
                    struct FileEnt<'a> {
                        v_start: u64,
                        #[allow(dead_code)]
                        v_end: u64,
                        ofs: u64,
                        path: &'a ::std::path::Path,
                    }
                    let n_files = get_s(&mut slice, c) as usize;
                    let _page_size = get_s(&mut slice, c);   // GDB seems to set this to `1`
                    let mut files = Vec::with_capacity(n_files);
                    for _i in 0 .. n_files {
                        files.push(FileEnt {
                            v_start: get_s(&mut slice, c),
                            v_end: get_s(&mut slice, c),
                            ofs: get_s(&mut slice, c), // Is this u64 instead?
                            path: "".as_ref(),
                        });
                    }
                    for i in  0 .. n_files {
                        let s = get_nul_str(&mut slice);
                        files[i].path = ::std::str::from_utf8(s).expect("Malformed UTF-8 in path, unsupported").as_ref();
                    }

                    // Find the lowest `v_start` for each file
                    for f in files {
                        if let Some(v) = modules.iter_mut().find(|v| v.path == f.path) {
                            if v.virt_base > f.v_start {
                                v.virt_base = f.v_start;
                                v.file_base = f.ofs;
                            }
                        }
                        else {
                            modules.push(super::ReferencedFile {
                                file_base: f.ofs,
                                virt_base: f.v_start,
                                path: f.path.to_owned(),
                            });
                        }

                        for r in memory_ranges.iter_mut() {
                            if r.v_start == f.v_start {
                                r.is_anon = false;
                            }
                        }
                    }
                    },
                _ => todo!("Note {:?} = ty={:#x} {:?}", crate::ByteStr(note.name), note.n_type, crate::ByteStr(note.desc)),
                }
            }
        }
        Ok(CoreDump {
            modules,
            memory_ranges,
            threads,
            fp: ::std::sync::Mutex::new(fp),
        })
    }

    pub fn anon_size(&self) -> usize {
        self.memory_ranges.iter().map(|v| if v.is_anon { v.size } else { 0 }).sum::<u64>() as usize
    }
    pub fn modules(&self) -> &[super::ReferencedFile] {
        &self.modules
    }
    pub fn get_thread(&self, index: usize) -> &crate::CpuState {
        &self.threads[index]
    }
    pub fn read_bytes(&self, addr: u64, dst: &mut [u8]) {
        for r in &self.memory_ranges {
            if r.v_start <= addr && addr < r.v_start + r.size {
                assert!( (addr - r.v_start) + dst.len() as u64 <= r.size, "Reading across segment boundaries" );
                match r.dump_file_ofs {
                RangeSource::Dump { offset } => {
                    use ::std::io::{Read,Seek};
                    let o = offset + (addr - r.v_start);
                    let r = {
                        let mut fp = self.fp.lock().unwrap();
                        if let Err(e) = fp.seek(::std::io::SeekFrom::Start(o)) {
                            Err(e)
                        }
                        else if let Err(e) = fp.read_exact(dst) {
                            Err(e)
                        }
                        else {
                            Ok(())
                        }
                    };
                    match r {
                    Err(e) => panic!("Failure reading {:#x}+{}: {:?}", addr, dst.len(), e),
                    Ok(()) => return,
                    }
                },
                RangeSource::External { .. } => todo!("External file"),
                }
            }
        }
        todo!("Not covered? {:#x}", addr)
    }
}

struct MemoryRange {
    /// Virtual memory address of start of this range
    v_start: u64,
    /// Size of this range in bytes
    size: u64,

    /// Offset in the dump file
    // TODO: Might be from an external file
    dump_file_ofs: RangeSource,

    /// Indicates that this is an anonoymous binding (i.e. no backing file, i.e. it's dynamic memory)
    is_anon: bool,
}
enum RangeSource {
    Dump { offset: u64 },
    #[allow(dead_code)]
    External { file_index: usize, offset: u64 }
}

fn get_u64(slice: &mut &[u8]) -> u64 {
    let (d,t) = slice.split_at(8);
    *slice = t;
    u64::from_ne_bytes(d.try_into().unwrap())
}
fn get_u32(slice: &mut &[u8]) -> u32 {
    let (d,t) = slice.split_at(4);
    *slice = t;
    u32::from_ne_bytes(d.try_into().unwrap())
}
fn get_s(slice: &mut &[u8], c: ::elf::file::Class) -> u64 {
    match c {
    elf::file::Class::ELF32 => get_u32(slice) as u64,
    elf::file::Class::ELF64 => get_u64(slice),
    }
}
fn get_nul_str<'a>(slice: &mut &'a [u8]) -> &'a [u8] {
    let Some(o) = slice.iter().position(|&v| v == 0) else { return ::std::mem::replace(slice, b"") };
    let rv = &slice[..o];
    *slice = &slice[o+1..];
    rv
}