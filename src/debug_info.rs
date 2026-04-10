
mod dwarf;

type Inner = dwarf::DebugPool;
pub struct DebugPool(Inner);
impl DebugPool {
    pub fn new() -> Self {
        DebugPool(dwarf::DebugPool::new())
    }
    pub fn add_file(&mut self, path: &::std::path::Path, base: u64, file_base: u64) -> Result<(), Box<dyn ::std::error::Error>> {
        self.0.add_file(path, base, file_base)
    }
    pub fn index_types(&mut self) {
        self.0.index_types();
    }
    pub fn get_caller(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump) -> crate::CpuState {
        self.0.get_caller(state, memory)
    }
    pub fn get_variable(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump, name: &str) -> (u64, TypeRef) {
        self.0.get_variable(state, memory, name)
    }
    pub fn get_type(&self, ty: &TypeRef) -> &Type {
        self.0.get_type(ty)
    }
    pub fn size_of(&self, ty: &Type) -> usize {
        self.0.size_of(ty)
    }

    pub fn fmt_type<'a>(&'a self, ty: &'a Type) -> impl ::std::fmt::Display + 'a {
        self.0.fmt_type(ty)
    }
    pub fn fmt_type_ref<'a>(&'a self, ty: &TypeRef) -> impl ::std::fmt::Display + 'a {
        self.0.fmt_type_ref(ty)
    }

    pub fn find_type_by_vtable(&self, addr: u64) -> Option<&Type> {
        self.0.find_type_by_vtable(addr)
    }
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

/// Reference to a type in the debug tree
#[derive(Clone, Copy, Hash, Eq, PartialEq)]
#[derive(Debug)]
pub struct TypeRef(usize);

#[derive(Debug)]
pub enum Type {
    Struct(CompositeType),
    Union(CompositeType),
    Primtive(PrimitiveType),
    Pointer(TypeRef, PointerClass),
    Alias(TypeRef),
    Enum(String),
    Array(TypeRef, usize),
}

#[derive(Debug)]
pub enum PointerClass {
    // Bog standard pointer
    Bare,
    // C++ `Foo&` type
    Reference,
    // C++ `Foo&&` type
    RValueReference,
}
#[derive(Debug)]
pub struct PrimitiveType {
    name: String,
    bits: u32,
}
#[derive(Debug)]
pub struct CompositeType {
    name: String,
    size: usize,
    pub fields: Vec<CompositeField>,
    parents: Vec<(u64, TypeRef)>,
    pub sub_types: ::std::collections::HashMap<String,TypeRef>,
}
impl CompositeType {
    fn new(name: String, size: usize) -> Self {
        CompositeType {
            name,
            size,
            fields: Default::default(),
            parents: Default::default(),
            sub_types: Default::default()
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parents(&self) -> impl Iterator<Item=&(u64, TypeRef)> {
        self.parents.iter()
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