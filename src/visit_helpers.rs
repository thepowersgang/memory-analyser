//use super::core_dump::CoreDump;
use super::debug_info::{DebugPool, Type, TypeRef};

/// A path through type visiting
pub struct Path<'a> {
    parent: Option<&'a Path<'a>>,
    node: PathNode<'a>,
}
impl<'a> Path<'a> {
    pub fn root() -> Self {
        Path {
            parent: None,
            node: PathNode::Null,
        }
    }
    fn add<'r>(&'r self, node: PathNode<'r>) -> Path<'r> {
        Path {
            parent: if let PathNode::Null = self.node { None } else { Some(self) },
            node,
        }
    }

    pub fn len(&self) -> usize {
        match self.parent {
        Some(p) => 1 + p.len(),
        None => match self.node
            {
            PathNode::Null => 0,
            _ => 1,
            },
        }
    }
    pub fn get_parent(&self) -> Option<&'a Path<'a>> {
        self.parent
    }
    pub fn get_prefix(&'a self, len: usize) -> &'a Path<'a> {
        let l = self.len();
        let n_pop = l.saturating_sub(len);
        let mut v = self;
        for _ in 0 .. n_pop {
            v = v.parent.unwrap();
        }
        v
    }

    pub fn is_root_or_deref(&self) -> bool {
        match self.node {
        PathNode::Null => true,
        PathNode::Deref => true,
        _ => false,
        }
    }

    /// Indexing of an array/vector or pointer
    pub fn index(&self, index: usize) -> Path<'_> {
        self.add(PathNode::Index(index))
    }
    /// Parent of a class
    pub fn parent(&self, index: usize) -> Path<'_> {
        self.add(PathNode::Parent(index))
    }
    /// Type field
    pub fn field<'r>(&'r self, name: &'r str) -> Path<'r> {
        self.add(PathNode::Field(name))
    }
    /// Dereference a (maybe smart) pointer
    pub fn deref(&self) -> Path<'_> {
        self.add(PathNode::Deref)
    }
}
impl<'a> ::std::fmt::Display for Path<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(v) = self.parent {
            v.fmt(f)?;
        }
        match self.node {
        PathNode::Null => Ok(()),
        PathNode::Field(name) => write!(f, ".{}", name),
        PathNode::Parent(idx) => write!(f, "#{}", idx),
        PathNode::Index(idx) => write!(f, "[{}]", idx),
        PathNode::Deref => write!(f, ".*"),
        }
    }
}
enum PathNode<'a> {
    Null,
    Field(&'a str),
    Parent(usize),
    Index(usize),
    Deref,
}

/// Get the offset and type of a named field (evaluating a path)
pub fn get_field(debug: &DebugPool, ty: &Type, path: &Path) -> (u64, TypeRef) {
    //println!("get_field: {} in {}", path, debug.fmt_type(ty));
    let (base, ty)  = if let Some(p) = path.parent {
        let (base,ty) = get_field(debug, ty, p);
        (base, debug.get_type(&ty))
    }
    else {
        (0, ty)
    };
    let ty = resolve_alias_chain(debug, ty);
    match ty {
    Type::Struct(composite_type) =>
        match path.node {
        PathNode::Field(name) => {
            let Some(f) = composite_type.iter_fields().find(|f| f.name == name) else {
                panic!("Failed to find {:?} in {} ({})", name, composite_type.name(), path);
            };
            (base + f.offset, f.ty)
        },
        PathNode::Parent(index) => {
            let Some((ofs, ty)) = composite_type.parents().nth(index) else {
                panic!("Failed to parent #{} in {} ({})", index, composite_type.name(), path);
            };
            (base + ofs, *ty)
        },
        PathNode::Null|PathNode::Index(_)|PathNode::Deref => panic!("Unexpected path node for `struct` in {}", path),
        },
    Type::Union(composite_type) => 
        match path.node {
        PathNode::Field(name) => {
            let f = composite_type.iter_fields().find(|f| f.name == name).unwrap();
            (base + f.offset, f.ty)
        },
        PathNode::Null|
        PathNode::Parent(_)|PathNode::Index(_)|PathNode::Deref => panic!("Unexpected path node for `union`"),
        },
    Type::Array(_, _) => todo!("array"),
    Type::Primtive(_) => panic!("Getting field of a primitive"),
    Type::Pointer(..) => todo!("Pointer"),
    Type::Alias(_) => panic!("Alias should be resolved"),
    Type::Enum(_) => panic!("Getting field of an enum"),
    }
}

/// Undo a chain of type alises (e.g. typedefs)
pub fn resolve_alias_chain<'a>(debug: &'a DebugPool, mut ty: &'a Type) -> &'a Type {
    while let Type::Alias(tr) = ty {
        ty = debug.get_type(tr);
    }
    ty
}