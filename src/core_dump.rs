mod mrustc;
mod elf;

enum Inner {
    /// Custom mrustc core dump
    MRustc(mrustc::CoreDump),
    /// Linux-standard ELF core dump
    Elf(elf::CoreDump),
}
pub type ReadError = ();

pub struct CoreDump(Inner);
impl CoreDump {
    pub fn open(path: &std::path::Path) -> Result<CoreDump,Box<dyn ::std::error::Error>> {
        if path.starts_with("/proc/") {
            // TODO: "core dump" from a running process
            //return Ok(CoreDump(Inner::LinuxProcFs(linux_proc_fs::CoreDump::open(fp)?)))
            todo!("procfs")
        }
        let mut fp = ::std::fs::File::open(path)?;
        let prefix = {
            use std::io::{Read,Seek};
            let mut prefix = [0; 16];
            fp.read_exact(&mut prefix)?;
            fp.seek(::std::io::SeekFrom::Start(0))?;
            prefix
        };
        // TODO: Detect ELF format dumps (elf prefix)
        Ok(CoreDump(if prefix[..4] == *b"\x7FELF" {
            Inner::Elf(elf::CoreDump::open(fp)?)
        }
        else if prefix[..12] == *b"FullDump\x97\r\n\0" {
            Inner::MRustc(mrustc::CoreDump::open(fp)?)
        }
        else {
            todo!("Unknown core dump format: first 16 bytes are {:#x?}", prefix);
        }))
    }

    pub fn anon_size(&self) -> usize {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.anon_size(),
        Inner::Elf(d) => d.anon_size(),
        }
    }

    pub fn modules(&self) -> &[ReferencedFile] {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.modules(),
        Inner::Elf(core_dump) => core_dump.modules(),
        }
    }

    pub fn get_thread(&self, index: usize) -> &crate::CpuState {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.get_thread(index),
        Inner::Elf(core_dump) => core_dump.get_thread(index),
        }
    }

    pub fn is_valid(&self, addr: u64, len: usize) -> bool {
        let v = match &self.0 {
            Inner::MRustc(core_dump) => core_dump.is_valid(addr, len),
            Inner::Elf(core_dump) => core_dump.is_valid(addr, len),
            };
        if !v && false {
            panic!("Oh noes! Out-of-bounds access {:#x}+{:#x}", addr, len);
        }
        v
    }
    pub fn read_bytes(&self, addr: u64, dst: &mut [u8]) -> Result<(),ReadError> {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.read_bytes(addr, dst),
        Inner::Elf(core_dump) => core_dump.read_bytes(addr, dst),
        }
    }
}


impl CoreDump {
    pub fn read_ptr(&self, addr: u64) -> Result<u64,()> {
        let mut v = [0; 8];
        self.read_bytes(addr, &mut v)?;
        Ok(u64::from_le_bytes(v))
    }
    pub fn read_u32(&self, addr: u64) -> Result<u32,()> {
        let mut v = [0; 4];
        self.read_bytes(addr, &mut v)?;
        Ok(u32::from_le_bytes(v))
    }
    pub fn read_u8(&self, addr: u64) -> Result<u8,()> {
        let mut v = [0; 1];
        self.read_bytes(addr, &mut v)?;
        Ok(v[0])
    }
}

/// Common representation of a mapped (named) file in the core dump
/// 
/// Used to get debug information
pub struct ReferencedFile {
    pub load_base: u64,
    pub file_base: u64,
    pub path: ::std::path::PathBuf,
}
