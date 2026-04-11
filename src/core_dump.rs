mod mrustc;

enum Inner {
    /// Custom mrustc core dump
    MRustc(mrustc::CoreDump),
}
pub struct CoreDump(Inner);
impl CoreDump {
    pub fn open(path: &std::path::Path) -> ::std::io::Result<CoreDump> {
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
        if prefix[..4] == *b"\x7FELF" {
            //return Ok(CoreDump(Inner::Elf(elf::CoreDump::open(fp)?)))
            todo!("elf core dumps")
        }
        else if prefix[..12] == *b"FullDump\x97\r\n\0" {
            Ok(CoreDump(Inner::MRustc(mrustc::CoreDump::open(fp)?)))
        }
        else {
            todo!("Unknown core dump format: first 16 bytes are {:#x?}", prefix);
        }
    }

    pub fn anon_size(&self) -> usize {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.anon_size(),
        }
    }

    pub fn modules(&self) -> impl Iterator<Item=(::std::path::PathBuf,u64,u64)> {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.modules(),
        }
    }

    pub fn get_thread(&self, index: usize) -> &crate::CpuState {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.get_thread(index),
        }
    }

    pub fn read_bytes(&self, addr: u64, dst: &mut [u8]) {
        match &self.0 {
        Inner::MRustc(core_dump) => core_dump.read_bytes(addr, dst),
        }
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
