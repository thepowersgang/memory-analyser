use crate::Input;
use crate::visit_helpers::Path;
use crate::debug_info::Type;

pub struct AllocRc<'d> {
    pub inner_ty: &'d Type,
    pub addr: u64,
}
impl<'d> AllocRc<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("alloc::rc::Rc<") {
            return None;
        }
        let (ptr_o, ptr_ty) = input.get_field(ty, Path::root().field("ptr").field("pointer"));
        let inner_ty = {
            let i = input.resolve_alias_chain_tr(&ptr_ty);
            let Type::Pointer(i, _) = i else { panic!("Expected pointer") };
            input.resolve_alias_chain_tr(i)
        };

        Some(Self {
            inner_ty,
            addr: input.dump.read_ptr(addr + ptr_o),
            })
    }
}

pub struct AllocString
{
    pub ptr: u64,
    pub cap: u64,
    pub len: u64,
}
impl AllocString
{
    pub fn opt_read(input: &Input, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if composite_type.name() != "alloc::string::String" {
            return None;
        }
        if false {
            print!("alloc String: "); crate::dump_type_fields(input.debug, ty, 0); println!("");
        }
        let (ptr_o,_) = input.get_field(ty, Path::root().field("vec").field("buf").field("inner").field("ptr"));
        let (cap_o,_) = input.get_field(ty, Path::root().field("vec").field("buf").field("inner").field("cap"));
        let (len_o,_) = input.get_field(ty, Path::root().field("vec").field("len"));
        let ptr = input.dump.read_ptr(addr + ptr_o);
        let cap = input.dump.read_ptr(addr + cap_o);
        let len = input.dump.read_ptr(addr + len_o);
        Some(Self { ptr, cap, len })
    }
}
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
    pub item_ty: &'d Type,
    // Sequence of `item_ty`
    data_end: u64,
    // `u8` sequence, with the top bit indicating "empty"
    ctrl_start: u64,
    // Number of buckets
    n_buckets: u64,
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
        if false {
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
        let _item_size = input.debug.size_of(item_ty);
        let ctrl = input.dump.read_ptr(addr + input.get_field(ty, Path::root().field("table").field("table").field("ctrl")).0);
        let _items = input.dump.read_ptr(addr + input.get_field(ty, Path::root().field("table").field("table").field("items")).0);
        let bucket_mask = input.dump.read_ptr(addr + input.get_field(ty, Path::root().field("table").field("table").field("bucket_mask")).0);
        let n_buckets = bucket_mask + 1;
        //println!("ctrl={ctrl:#x}, n_buckets={}, item_size={_item_size} ({_item_size:#x}) items={_items}", n_buckets);
        Some(Self {
            item_ty,
            data_end: ctrl,
            ctrl_start: ctrl,
            n_buckets,
        })
    }

    pub fn next(&mut self, input: &Input) -> Option<u64> {
        while self.n_buckets > 0 {
            let tag = input.dump.read_u8(self.ctrl_start);
            self.n_buckets -= 1;
            self.ctrl_start += 1;
            self.data_end -= input.debug.size_of(self.item_ty) as u64;
            let cur = self.data_end;
            let is_populated = tag & 0x80 == 0;
            if is_populated {
                //println!("Use {cur:#x} (tag={:#x})", tag);
                return Some(cur);
            }
            //println!("Skip {cur:#x} (tag={tag:#x})");
        }
        None
    }
}
