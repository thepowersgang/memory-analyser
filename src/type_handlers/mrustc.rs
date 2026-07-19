use crate::Input;
use crate::debug_info::Type;
use crate::visit_helpers::Path;
use crate::core_dump::ReadError;

/// A mrustc `TAGGED_UNION` structure
pub struct TaggedUnion<'d> {
    /// Offset to the data, relative to the passed `addr`
    pub data_ofs: u64,
    /// Name and data type of the current variant
    pub variant: Option<(&'d str, &'d Type)>,
    /// Other data fields on the union (`TAGGED_UNION_EX` can add methods and data)
    pub other_fields: &'d [crate::debug_info::CompositeField],
    pub data_union: &'d crate::debug_info::CompositeType,
}
impl<'d> TaggedUnion<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &'d Type, addr: u64) -> Result<Option<Self>,()> {
        let Type::Struct(composite_type) = ty else {
            return Ok(None);
        };
        if !(composite_type.fields.len() >= 2
            && composite_type.fields[0].name == "m_tag"
            && composite_type.fields[1].name == "m_data"
            ) {
            return Ok(None);
        }
        let d_ty = input.resolve_alias_chain_tr(&composite_type.fields[1].ty);
        let Type::Union(d_u) = d_ty else { return Ok(None) };
        let t_o = composite_type.fields[0].offset;
        let d_o = composite_type.fields[1].offset;
        let tag = input.dump.read_u32(addr + t_o)? as usize;
        let variant = if tag == 0 {
                None
            }
            else if tag > d_u.fields.len() {
                println!("Invalid tagged union: tag out of range {:#x} > {}", tag, d_u.fields.len());
                None
            }
            else {
                let f = &d_u.fields[tag-1];
                let ty = input.debug.get_type(&f.ty);
                Some((&f.name[..], ty))
            };
        Ok(Some(Self {
            data_ofs: d_o,
            variant,
            other_fields: &composite_type.fields[2..],
            data_union: d_u,
        }))
    }
}

pub struct RcString {
    pub data_addr: u64,
    pub string_ptr: u64,
    pub string_len: u64,
    //pub refcount: u32,
}
impl RcString {
    pub fn opt_read<'d>(input: &Input<'d>, ty: &'d Type, addr: u64) -> Result<Option<Self>,ReadError> {
        let Type::Struct(composite_type) = ty else {
            return Ok(None);
        };
        if composite_type.name() != "RcString" {
            return Ok(None);
        }
        let Type::Pointer(inner_ty, ..) = input.debug.get_type(&composite_type.fields[0].ty) else { panic!() };
        let inner_ty = input.debug.get_type(inner_ty);
        if false {
            print!("RcString: "); crate::dump_type_fields(input.debug, inner_ty, 0); println!("");
        }
        let (data_ofs, _) = input.get_field(inner_ty, Path::root().field("data"));
        let (size_ofs, _) = input.get_field(inner_ty, Path::root().field("size"));
        let ptr = input.dump.read_ptr(addr)?;
        if ptr != 0 && !input.dump.is_valid(ptr + size_ofs, 8) {
            //return Ok(Some(MrustcRcString { data_addr: 0, string_ptr: 0, string_len: 0 }))
            eprintln!("Invalid pointer in RcString @{:#x}: {:#x}", addr, ptr);
            println!("Invalid pointer in RcString: {:#x}", ptr);
            return Err(());
        }
        Ok(Some(Self {
            data_addr: ptr,
            string_ptr: if ptr == 0 { 0 } else { ptr + data_ofs },
            string_len: if ptr == 0 { 0 } else { input.dump.read_u32(ptr + size_ofs)? as u64 },
        }))
    }
}

pub struct ThinVector<'a> {
    pub data_ptr: u64,
    pub len: u64,
    pub cap: u64,
    pub inner_ty: &'a Type,
}
impl<'d> ThinVector<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &'d Type, addr: u64) -> Result<Option<Self>,ReadError> {
        let Type::Struct(composite_type) = ty else {
            return Ok(None);
        };
        if !composite_type.name().starts_with("ThinVector<") {
            return Ok(None);
        }
        let Type::Pointer(inner_ty, ..) = input.debug.get_type(&composite_type.fields[0].ty) else { panic!() };
        let inner_ty = input.debug.get_type(inner_ty);
        let p = input.dump.read_ptr(addr)?;
        Ok(Some(if p != 0 {
            let inner_size = input.debug.size_of(inner_ty) as u64;
            let metadata_len = (16 + (inner_size-1)) / inner_size;
            let meta_addr = p - metadata_len * inner_size;
            ThinVector {
                data_ptr: p,
                len: input.dump.read_ptr(meta_addr + 0)?,
                cap: input.dump.read_ptr(meta_addr + 8)?,
                inner_ty,
            }
        }
        else {
            ThinVector {
                data_ptr: 0,
                len: 0,
                cap: 0,
                inner_ty,
            }
        }))
    }
}