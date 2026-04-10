mod mrustc;

enum Inner {
    /// Custom mrustc core dump
    MRustc(mrustc::CoreDump),
}
pub struct CoreDump(Inner);
impl CoreDump {
    pub fn open(path: &std::path::Path) -> ::std::io::Result<CoreDump> {
        Ok(CoreDump(Inner::MRustc(mrustc::CoreDump::open(path)?)))
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
