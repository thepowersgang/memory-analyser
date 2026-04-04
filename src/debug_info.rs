
mod dwarf_parse;

struct ElfFiles<'a> {
    // TODO: Rewrite this again to use `::elf::ElfStream`, but that has some interactions with lifetimes in the closure passed to `Dwarf::load`
    // - Doing so would avoid needing to load the entire executable (including the .text section).
    // - For now, it's not a huge cost
    file_main: ::elf::ElfBytes<'a, ::elf::endian::NativeEndian>,
    file_debug: Option<::elf::ElfBytes<'a, ::elf::endian::NativeEndian>>,
}
impl<'s> ElfFiles<'s> {
    pub fn open(path: &::std::path::Path, storage: &'s mut (Vec<u8>,Vec<u8>)) -> Result<Self, Box<dyn ::std::error::Error>> {
        let path_debug = path.with_added_extension("debug");
        Ok(ElfFiles {
            file_main: {
                storage.0 = ::std::fs::read(path)?;
                ::elf::ElfBytes::minimal_parse(&storage.0)?
            },
            file_debug: if path_debug.exists() {
                storage.1 = ::std::fs::read(path)?;
                Some(::elf::ElfBytes::minimal_parse(&storage.1)?)
            } else {
                None
            },
        })
    }
    pub fn section_headers(&self) -> ::elf::section::SectionHeaderTable<'s, ::elf::endian::LittleEndian> {
        self.file_main.section_headers().unwrap()
    }
    pub fn section_data_opt(&self, name: &str) -> Result<Option<(::elf::section::SectionHeader, &'s [u8])>,::elf::ParseError> {
        Ok(match self.file_main.section_header_by_name(name)?
        {
        Some(v) => Some((v, self.file_main.section_data(&v)?.0.into())),
        None => match self.file_debug
            {
            Some(ref f) => match f.section_header_by_name(name)?
                {
                Some(v) => Some((v, f.section_data(&v)?.0.into())),
                None => None,
                },
            None => None,
            },
        })
    }
}

#[derive(Default)]
pub struct DebugPool {
    functions: ::std::collections::HashMap<String,FunctionRecord>,
    type_lookup: ::std::collections::HashMap< (usize, ::gimli::UnitOffset), TypeRef >,
    backtrace_data: Vec<(u64, u64, BacktraceType,)>,
    types: Vec<Option<Type>>,
    next_unit_index: usize,
}
impl DebugPool {
    pub fn new() -> Self {
        Default::default()
    }
    pub fn add_file(&mut self, path: &::std::path::Path, base: u64, file_base: u64) -> Result<(), Box<dyn ::std::error::Error>>
    {
        println!("{:?} @ {:#x} (from {:#x})", path, base, file_base);
        let mut elf_files = (Vec::new(), Vec::new());
        let elf_files = ElfFiles::open(path, &mut elf_files)?;
        // Load with ELF loade
        let mut lowest_load = !0;
        for s in elf_files.section_headers() {
            if s.sh_flags & ::elf::abi::SHF_ALLOC as u64 != 0 {
                lowest_load = s.sh_addr.min(lowest_load);
            }
        }
        println!("lowest_load={:#x}", lowest_load);

        if let Some((shdr,sdata)) = elf_files.section_data_opt(".debug_frame")?
        {
            let section_base = shdr.sh_addr;
            let debug_frame: ::std::rc::Rc<[u8]> = sdata.into();
            let debug_frame = ::gimli::EndianRcSlice::new(debug_frame, ::gimli::LittleEndian);
            self.backtrace_data.push((base, section_base, BacktraceType::Debug(::gimli::DebugFrame::from(debug_frame)),));
        }
        else if let Some((shdr,sdata)) = elf_files.section_data_opt(".eh_frame")?
        {
            let section_base = shdr.sh_addr;
            let eh_frame: ::std::rc::Rc<[u8]> = sdata.into();
            let eh_frame = ::gimli::EndianRcSlice::new(eh_frame, ::gimli::LittleEndian);
            self.backtrace_data.push((base, section_base, BacktraceType::Eh(::gimli::EhFrame::from(eh_frame)),));
        }
        let debug_info = ::gimli::Dwarf::load::<_,::elf::ParseError>(|section| {
            let s = elf_files.section_data_opt(section.name())?;
            let section_data = match s {
                Some((_,sdata)) => sdata,
                None => b"",
            };
            Ok(::gimli::EndianSlice::new(section_data, ::gimli::LittleEndian))
        })?;
        self.add_variables_types_from_dwarf(&debug_info);
        Ok( () )
    }

    pub fn get_caller(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump) -> crate::CpuState {
        println!("get_caller: {:#x}", state.get_pc());
        let mut context = ::gimli::UnwindContext::new();
        for (base, eh_base, info) in &self.backtrace_data {
            println!("get_caller: {:#x} + {:#x}", base, eh_base);
            let bases = ::gimli::BaseAddresses::default()
                .set_text(*base)
                .set_eh_frame(*base + *eh_base)
                ;
            use ::gimli::UnwindSection;
            match match info 
                {
                BacktraceType::Debug(debug_frame) =>
                    debug_frame.unwind_info_for_address(&bases, &mut context, state.get_pc(), ::gimli::DebugFrame::cie_from_offset),
                BacktraceType::Eh(eh_frame) => 
                    eh_frame.unwind_info_for_address(&bases, &mut context, state.get_pc(), ::gimli::EhFrame::cie_from_offset),
                }
            {
            Ok(i) => {
                fn get_register(state: &crate::CpuState, register: &::gimli::Register) -> u64 {
                    match register.0 {
                    i @ 0 .. 16 => state.gprs[i as usize],
                    16 => state.pc,
                    _ => todo!("get_register: {:?}", register),
                    }
                }
                let cfa = match i.cfa()
                    {
                    &gimli::CfaRule::RegisterAndOffset { register, offset } => get_register(state, &register).checked_add_signed(offset).unwrap(),
                    gimli::CfaRule::Expression(_unwind_expression) => todo!("CfaRule::Expression"),
                    };
                let mut rv = crate::CpuState::stub();
                for (r_name,rule) in i.registers() {
                    println!("{:?}: {:?}", r_name, rule);
                    let v = match rule
                        {
                        ::gimli::RegisterRule::Undefined => 0,
                        ::gimli::RegisterRule::SameValue => state.gprs[r_name.0 as usize],
                        ::gimli::RegisterRule::Offset(cfa_ofs) => memory.read_ptr(cfa.wrapping_add_signed(*cfa_ofs)),
                        ::gimli::RegisterRule::ValOffset(cfa_ofs) => cfa.wrapping_add_signed(*cfa_ofs),
                        ::gimli::RegisterRule::Register(register) => get_register(state, register),
                        ::gimli::RegisterRule::Expression(_unwind_expression) => todo!("RegisterRule::Expression"),
                        ::gimli::RegisterRule::ValExpression(_unwind_expression) => todo!("RegisterRule::ValExpression"),
                        ::gimli::RegisterRule::Architectural => todo!("RegisterRule::Architectural"),
                        ::gimli::RegisterRule::Constant(v) => *v,
                        };
                    match r_name.0 {
                    i @ 0 .. 16 => rv.gprs[i as usize] = v,
                    16 => rv.pc = v,
                    _ => {},
                    }
                }
                return rv;
            },
            Err(gimli::Error::NoUnwindInfoForAddress) => continue,
            Err(e) => todo!("Unwind error: {:?}", e),
            }
        }
        todo!("get_caller: no entry")
    }

    // Get the storage address of a variable
    pub fn get_variable(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump, name: &str) -> (u64, TypeRef) {
        let pc = state.get_pc();
        for (fcn_name, fcn_rec) in &self.functions {
            if fcn_rec.pc_range.contains(pc) {
                let var = &fcn_rec.variables[name];
                for r in &var.ranges {
                    if r.pc_range.contains(pc) {
                        match &r.position {
                        VariablePosition::OptimisedOut => todo!("Optimsed out variable"),
                        VariablePosition::Fixed(_) => todo!(),
                        VariablePosition::Expr(items) => todo!(),
                        }
                    }
                }
                todo!("Unable to find variable def in {} at PC {:#x}", fcn_name, pc);
            }
        }
        todo!("get_variable: {:?} - Failed to find function for PC={:#x}", name, pc)
    }
    pub fn get_type(&self, ty: &TypeRef) -> &Type {
        self.types[ty.0].as_ref().expect("Type not populated")
    }

    fn dwarf_type_ref(&mut self, unit_index: usize, ofs: ::gimli::UnitOffset) -> TypeRef {
        *self.type_lookup.entry((unit_index, ofs))
            .or_insert_with(|| {
                let rv = TypeRef(self.types.len());
                self.types.push(None);
                rv
            })
    }
}

#[derive(Debug)]
enum BacktraceType {
    Debug(::gimli::DebugFrame<::gimli::EndianRcSlice<::gimli::LittleEndian>>),
    Eh(::gimli::EhFrame<::gimli::EndianRcSlice<::gimli::LittleEndian>>),
}

#[derive(Clone, Copy)]
struct PcRange {
    start: u64,
    end: u64,
}
impl PcRange {
    fn contains(&self, pc: u64) -> bool {
        self.start <= pc && pc < self.end
    }
}
impl ::std::fmt::Debug for PcRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:x}--0x{:x}", self.start, self.end)
    }
}
#[derive(Debug,Clone)]
struct PcRanges {
    ranges: Vec<PcRange>,
}
impl PcRanges {
    fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
    fn contains(&self, pc: u64) -> bool {
        self.ranges.iter().any(|v| v.contains(pc))
    }
}
struct FunctionRecord {
    /// Range of PC values covered by this function
    pc_range: PcRanges,
    variables: ::std::collections::HashMap<String,VariableRecord>,
}
#[derive(Debug)]
struct VariableRecord {
    ty: TypeRef,
    ranges: Vec<VariableRange>,
}
#[derive(Debug)]
struct VariableRange {
    pc_range: PcRanges,
    position: VariablePosition,
}
#[derive(Debug)]
enum VariablePosition {
    OptimisedOut,
    Fixed(u64),
    Expr(Vec<u8>),  // see `gimli::read::Expression`
}

/// Reference to a type in the debug tree
#[derive(Clone, Copy)]
#[derive(Debug)]
pub struct TypeRef(usize);

pub enum Type {
    Struct(CompositeType),
    Union(CompositeType),
    Primtive(PrimitiveType),
    Pointer(TypeRef),
}
pub struct PrimitiveType {
    bits: u32,
}
pub struct CompositeType {
    name: String,
    fields: Vec<CompositeField>,
}
impl CompositeType {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn iter_fields(&self) -> impl Iterator<Item=&CompositeField> {
        self.fields.iter()
    }
}
#[derive(Debug)]
pub struct CompositeField {
    pub offset: u64,
    pub name: String,
    pub ty: TypeRef,
}