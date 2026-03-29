
mod dwarf_parse;

#[derive(Default)]
pub struct DebugPool {
    functions: ::std::collections::HashMap<String,FunctionRecord>,
    type_lookup: ::std::collections::HashMap< (usize, ::gimli::UnitOffset), TypeRef >,
    backtrace_data: Vec<(u64, BacktraceType,)>,
    types: Vec<Option<Type>>,
    next_unit_index: usize,
}
impl DebugPool {
    pub fn new() -> Self {
        Default::default()
    }
    pub fn add_file(&mut self, path: &::std::path::Path, base: u64) -> Result<(), Box<dyn ::std::error::Error>>
    {
        let tmp_path = path.with_added_extension("debug");
        let path = if tmp_path.exists() {
            tmp_path.as_path()
        }
        else {
            path
        };
        // Load with ELF loader
        let bytes = ::std::fs::read(path)?;
        let elf_file = ::elf::ElfBytes::<::elf::endian::LittleEndian>::minimal_parse(&bytes)?;

        if let Some(debug_frame) = elf_file.section_header_by_name(".debug_frame")?
        {
            let debug_frame: ::std::rc::Rc<[u8]> = elf_file.section_data(&debug_frame)?.0.into();
            let debug_frame = ::gimli::EndianRcSlice::new(debug_frame, ::gimli::LittleEndian);
            self.backtrace_data.push((base, BacktraceType::Debug(::gimli::DebugFrame::from(debug_frame)),));
        }
        else if let Some(eh_frame) = elf_file.section_header_by_name(".eh_frame")?
        {
            let eh_frame: ::std::rc::Rc<[u8]> = elf_file.section_data(&eh_frame)?.0.into();
            let eh_frame = ::gimli::EndianRcSlice::new(eh_frame, ::gimli::LittleEndian);
            self.backtrace_data.push((base, BacktraceType::Eh(::gimli::EhFrame::from(eh_frame)),));
        }
        let debug_info = ::gimli::Dwarf::load::<_,::elf::ParseError>(|section| {
            let s = elf_file.section_header_by_name(section.name())?;
            let section_data = match s {
                Some(s) => elf_file.section_data(&s)?.0,
                None => b"",
            };
            Ok(::gimli::EndianSlice::new(section_data, ::gimli::LittleEndian))
        })?;
        self.add_variables_types_from_dwarf(&debug_info);
        Ok( () )
    }

    pub fn get_caller(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump) -> crate::CpuState {
        let mut context = ::gimli::UnwindContext::new();
        for (base, info) in &self.backtrace_data {
            let bases = ::gimli::BaseAddresses::default().set_text(*base);
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
                todo!("unwind: {:?}", i);
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
        todo!()
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