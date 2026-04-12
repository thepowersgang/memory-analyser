use crate::Input;
use crate::visit_helpers::Path;
use crate::debug_info::Type;

pub struct AllocVec<'d>
{
    pub begin: u64,
    pub end: u64,
    pub alloc_end: u64,
    pub item_ty: &'d Type,
}
impl<'d> AllocVec<'d>
{
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("alloc::vec::Vec<") {
            return None;
        }
        if false {
            print!("alloc Vec: "); crate::dump_type_fields(input.debug, ty, 0); println!("");
        }
        let (_, marker_ty) = input.get_field(ty, Path::root().field("buf").field("_marker"));
        let (ptr_o,_) = input.get_field(ty, Path::root().field("buf").field("inner").field("ptr"));
        let (cap_o,_) = input.get_field(ty, Path::root().field("buf").field("inner").field("cap"));
        let (len_o,_) = input.get_field(ty, Path::root().field("len"));
        let ptr = input.dump.read_ptr(addr + ptr_o);
        let cap = input.dump.read_ptr(addr + cap_o);
        let len = input.dump.read_ptr(addr + len_o);
        // Will need to parse the type name and look up in debug information, as Rust doesn't have the same type aliases as C++
        let item_ty = {
            let n = format!("{}", input.debug.fmt_type_ref(&marker_ty));
            let Some(n) = n.strip_prefix("core::marker::PhantomData<") else { panic!() };
            let Some(n) = n.strip_suffix(">") else { panic!() };
            match input.debug.find_type_by_name(n)
            {
            Some(t) => t,
            None => panic!("Failed to find type {:?}", n),
            }
        };
        let size = input.debug.size_of(item_ty);
        Some(Self {
            begin: ptr,
            end: ptr + len * size as u64,
            alloc_end: ptr + cap * size as u64,
            item_ty,
        })
    }
}

pub struct HashbrownMap<'d>
{
    item_ty: &'d Type,
}
impl<'d> HashbrownMap<'d>
{
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("hashbrown::map::HashMap<") {
            return None;
        }
        if true {
            print!("hashbrown HashMap: "); crate::dump_type_fields(input.debug, ty, 0); println!("");
        }
        // Get the item type by parsing the `marker` type's name
        let (_, marker_ty) = input.get_field(ty, Path::root().field("table").field("marker"));
        let item_ty = {
            let n = format!("{}", input.debug.fmt_type_ref(&marker_ty));
            let Some(n) = n.strip_prefix("core::marker::PhantomData<") else { panic!() };
            let Some(n) = n.strip_suffix(">") else { panic!() };
            match input.debug.find_type_by_name(n)
            {
            Some(t) => t,
            None => panic!("Failed to find type {:?}", n),
            }
        };
        // 
        todo!("hashbrown: {:?}", item_ty);
    }
}
