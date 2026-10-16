//! Compile-only parity checks between `include/orrery.h` and the Rust C ABI.
//!
//! Rust declarations are read with `syn`, not copied into a second hand-written
//! inventory. The C compiler checks field types, function pointer signatures,
//! constants, and repr(C) offsets/sizes for the current target.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use cc::{Build, Tool};
use regex::Regex;
use syn::punctuated::Punctuated;
use syn::{
    Expr, Fields, FnArg, Item, Lit, Meta, Pat, ReturnType, Token, Type, UnOp, UseTree, Visibility,
};

const TARGET: &str = env!("ORR_FFI_TARGET");

#[derive(Clone, Debug, Eq, PartialEq)]
struct AbiType {
    base: String,
    /// Rust raw pointers from outermost to innermost. `true` means `*const`.
    pointers: Vec<bool>,
    array_lengths: Vec<usize>,
}

impl AbiType {
    fn c_declaration(&self, declarator: &str) -> String {
        assert!(
            self.array_lengths.is_empty(),
            "arrays in the public ABI are unsupported"
        );
        let mut name = declarator.to_owned();
        for (index, _) in self.pointers.iter().enumerate() {
            let pointee_is_const = index > 0 && self.pointers[index - 1];
            let qualifier = if pointee_is_const { " const" } else { "" };
            name = format!("*{qualifier} {name}");
        }
        let base_is_const = self.pointers.last().copied().unwrap_or(false);
        let base = if base_is_const {
            format!("const {}", self.base)
        } else {
            self.base.clone()
        };
        format!("{base} {name}").trim().to_owned()
    }

    fn c_type(&self) -> String {
        self.c_declaration("")
    }

    fn layout(
        &self,
        primitive: &BTreeMap<String, (usize, usize)>,
    ) -> Result<(usize, usize), String> {
        if !self.pointers.is_empty() {
            return primitive
                .get("*const u8")
                .copied()
                .ok_or_else(|| "missing target pointer layout".to_owned());
        }
        let (size, align) = primitive
            .get(&self.base)
            .copied()
            .ok_or_else(|| format!("unsupported by-value ABI type {}", self.base))?;
        let count = self
            .array_lengths
            .iter()
            .try_fold(1usize, |n, len| n.checked_mul(*len))
            .ok_or_else(|| "array layout overflow".to_owned())?;
        Ok((
            size.checked_mul(count)
                .ok_or_else(|| "array layout overflow".to_owned())?,
            align,
        ))
    }
}

#[derive(Clone, Debug)]
struct FieldModel {
    name: String,
    ty: AbiType,
}

#[derive(Clone, Debug)]
struct StructModel {
    name: String,
    fields: Vec<FieldModel>,
}

#[derive(Clone, Debug)]
struct ConstantModel {
    name: String,
    ty: AbiType,
    value: i128,
}

#[derive(Clone, Debug)]
struct FunctionModel {
    name: String,
    result: AbiType,
    args: Vec<AbiType>,
}

#[derive(Clone, Debug)]
struct AbiModel {
    structs: Vec<StructModel>,
    constants: Vec<ConstantModel>,
    functions: Vec<FunctionModel>,
}

#[derive(Clone, Debug)]
struct StructLayout {
    size: usize,
    align: usize,
    offsets: Vec<usize>,
}

fn ffi_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")
}

fn ffi_header() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/orrery.h")
}

fn is_public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn repr_c(attrs: &[syn::Attribute], owner: &str) -> Result<bool, String> {
    let reprs: Vec<_> = attrs
        .iter()
        .filter(|attr| attr.path().is_ident("repr"))
        .collect();
    if reprs.len() > 1 {
        return Err(format!(
            "multiple repr attributes on {owner} are unsupported"
        ));
    }
    let Some(attr) = reprs.first() else {
        return Ok(false);
    };
    let args = attr
        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .map_err(|error| format!("cannot parse repr on {owner}: {error}"))?;
    let has_c = args
        .iter()
        .any(|meta| matches!(meta, Meta::Path(path) if path.is_ident("C")));
    if has_c
        && (args.len() != 1
            || !matches!(args.first(), Some(Meta::Path(path)) if path.is_ident("C")))
    {
        return Err(format!(
            "unsupported repr modifier on {owner}; only repr(C) is accepted"
        ));
    }
    Ok(has_c)
}

fn reject_conditional_attrs(attrs: &[syn::Attribute], owner: &str) -> Result<(), String> {
    for attr in attrs {
        if attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr") {
            return Err(format!(
                "conditional ABI declaration on {owner} is unsupported"
            ));
        }
    }
    Ok(())
}

fn use_tree_aliases(
    tree: &UseTree,
    prefix: &str,
    aliases: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    match tree {
        UseTree::Path(path) => {
            let next = if prefix.is_empty() {
                path.ident.to_string()
            } else {
                format!("{prefix}::{}", path.ident)
            };
            use_tree_aliases(&path.tree, &next, aliases)
        }
        UseTree::Name(name) => {
            let ident = name.ident.to_string();
            let full_path = if prefix.is_empty() {
                ident.clone()
            } else {
                format!("{prefix}::{ident}")
            };
            if aliases.insert(ident.clone(), full_path.clone()).is_some() {
                return Err(format!("duplicate imported ABI type name {ident}"));
            }
            Ok(())
        }
        UseTree::Rename(rename) => {
            let ident = rename.rename.to_string();
            let full_path = if prefix.is_empty() {
                rename.ident.to_string()
            } else {
                format!("{prefix}::{}", rename.ident)
            };
            if aliases.insert(ident.clone(), full_path.clone()).is_some() {
                return Err(format!("duplicate imported ABI type name {ident}"));
            }
            Ok(())
        }
        UseTree::Group(group) => {
            for item in &group.items {
                use_tree_aliases(item, prefix, aliases)?;
            }
            Ok(())
        }
        UseTree::Glob(_) => Err("glob imports in the ABI root are unsupported".to_owned()),
    }
}

fn abi_imports(items: &[Item]) -> Result<BTreeMap<String, String>, String> {
    let mut aliases = BTreeMap::new();
    for item in items {
        if let Item::Use(item) = item {
            use_tree_aliases(&item.tree, "", &mut aliases)?;
        }
    }
    for (name, path) in &aliases {
        let valid_c_alias = match name.as_str() {
            "c_char" => matches!(
                path.as_str(),
                "std::ffi::c_char" | "core::ffi::c_char" | "std::os::raw::c_char"
            ),
            "c_int" => matches!(
                path.as_str(),
                "std::ffi::c_int" | "core::ffi::c_int" | "std::os::raw::c_int"
            ),
            _ => false,
        };
        if matches!(
            name.as_str(),
            "u8" | "u32"
                | "u64"
                | "i32"
                | "i64"
                | "usize"
                | "OrrHost"
                | "OrrHostConfig"
                | "OrrClientConfig"
                | "OrrSessionStatus"
                | "std"
                | "core"
        ) || (matches!(name.as_str(), "c_char" | "c_int") && !valid_c_alias)
        {
            return Err(format!(
                "import shadows or aliases a supported ABI type: {name} from {path}"
            ));
        }
    }
    Ok(aliases)
}

fn abi_type(ty: &Type, imports: &BTreeMap<String, String>) -> Result<AbiType, String> {
    match ty {
        Type::Ptr(pointer) => {
            let mut inner = abi_type(&pointer.elem, imports)?;
            if !inner.array_lengths.is_empty() {
                return Err("pointer to an array in the public ABI is unsupported".to_owned());
            }
            inner.pointers.insert(0, pointer.const_token.is_some());
            Ok(inner)
        }
        Type::Array(_) => Err("arrays in the public C ABI are unsupported".to_owned()),
        Type::Path(path) if path.qself.is_none() => {
            let segment = path
                .path
                .segments
                .last()
                .ok_or_else(|| "empty type path".to_owned())?;
            if !matches!(segment.arguments, syn::PathArguments::None) {
                return Err("generic ABI types are unsupported".to_owned());
            }
            let name = segment.ident.to_string();
            let qualified = path
                .path
                .segments
                .iter()
                .map(|part| part.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            let primitive = match qualified.as_str() {
                "u8" | "core::primitive::u8" | "std::primitive::u8" => Some("uint8_t"),
                "u32" | "core::primitive::u32" | "std::primitive::u32" => Some("uint32_t"),
                "u64" | "core::primitive::u64" | "std::primitive::u64" => Some("uint64_t"),
                "i32" | "core::primitive::i32" | "std::primitive::i32" => Some("int32_t"),
                "i64" | "core::primitive::i64" | "std::primitive::i64" => Some("int64_t"),
                "usize" | "core::primitive::usize" | "std::primitive::usize" => Some("size_t"),
                "c_char"
                    if imports.get("c_char").is_some_and(|p| {
                        matches!(
                            p.as_str(),
                            "std::ffi::c_char" | "core::ffi::c_char" | "std::os::raw::c_char"
                        )
                    }) =>
                {
                    Some("char")
                }
                "std::ffi::c_char" | "core::ffi::c_char" | "std::os::raw::c_char" => Some("char"),
                "c_int"
                    if imports.get("c_int").is_some_and(|p| {
                        matches!(
                            p.as_str(),
                            "std::ffi::c_int" | "core::ffi::c_int" | "std::os::raw::c_int"
                        )
                    }) =>
                {
                    Some("int")
                }
                "std::ffi::c_int" | "core::ffi::c_int" | "std::os::raw::c_int" => Some("int"),
                _ => None,
            };
            let base = match primitive {
                Some(base) => base,
                None if qualified == name
                    && matches!(
                        name.as_str(),
                        "OrrHost" | "OrrHostConfig" | "OrrClientConfig" | "OrrSessionStatus"
                    ) =>
                {
                    name.as_str()
                }
                _ => return Err(format!("unsupported public ABI type {name}")),
            };
            Ok(AbiType {
                base: base.to_owned(),
                pointers: Vec::new(),
                array_lengths: Vec::new(),
            })
        }
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(AbiType {
            base: "void".to_owned(),
            pointers: Vec::new(),
            array_lengths: Vec::new(),
        }),
        _ => Err("unsupported public ABI syntax".to_owned()),
    }
}

fn constant_value(expr: &Expr) -> Result<i128, String> {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Int(value) => value
                .base10_parse::<i128>()
                .map_err(|error| error.to_string()),
            _ => Err("ABI constants must be integer literals".to_owned()),
        },
        Expr::Unary(unary) if matches!(unary.op, UnOp::Neg(_)) => {
            constant_value(&unary.expr).map(|value| -value)
        }
        Expr::Paren(paren) => constant_value(&paren.expr),
        Expr::Group(group) => constant_value(&group.expr),
        _ => Err("unsupported ABI constant expression".to_owned()),
    }
}

fn has_no_mangle(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr_name(attr, "no_mangle"))
}

fn has_export_name(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr_name(attr, "export_name"))
}

fn attr_name(attr: &syn::Attribute, name: &str) -> bool {
    if attr.path().is_ident(name) {
        return true;
    }
    if attr.path().is_ident("unsafe") {
        if let Ok(items) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) {
            return items.iter().any(|item| match item {
                Meta::Path(path) | Meta::NameValue(syn::MetaNameValue { path, .. }) => {
                    path.is_ident(name)
                }
                Meta::List(list) => list.path.is_ident(name),
            });
        }
    }
    false
}

fn parse_model(source: &str) -> Result<AbiModel, String> {
    let file = syn::parse_file(source)
        .map_err(|error| format!("cannot parse FFI Rust source: {error}"))?;
    let imports = abi_imports(&file.items)?;
    let mut model = AbiModel {
        structs: Vec::new(),
        constants: Vec::new(),
        functions: Vec::new(),
    };

    for item in file.items {
        let type_namespace_name = match &item {
            Item::Struct(item) => Some(item.ident.to_string()),
            Item::Enum(item) => Some(item.ident.to_string()),
            Item::Union(item) => Some(item.ident.to_string()),
            Item::Type(item) => Some(item.ident.to_string()),
            Item::Mod(item) => Some(item.ident.to_string()),
            _ => None,
        };
        if let Some(name) = type_namespace_name {
            let primitive_or_namespace = matches!(
                name.as_str(),
                "u8" | "u32"
                    | "u64"
                    | "i32"
                    | "i64"
                    | "usize"
                    | "c_char"
                    | "c_int"
                    | "std"
                    | "core"
            );
            let shadowed_orr_type = matches!(
                name.as_str(),
                "OrrHost" | "OrrHostConfig" | "OrrClientConfig" | "OrrSessionStatus"
            ) && !matches!(&item, Item::Struct(_));
            if primitive_or_namespace || shadowed_orr_type {
                return Err(format!(
                    "type namespace shadows a supported ABI path: {name}"
                ));
            }
        }
        match item {
            Item::Struct(item) if is_public(&item.vis) => {
                if item
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("cfg_attr"))
                {
                    return Err(format!(
                        "cfg_attr on public ABI candidate {} is unsupported",
                        item.ident
                    ));
                }
                if !repr_c(&item.attrs, &item.ident.to_string())? {
                    continue;
                }
                reject_conditional_attrs(&item.attrs, &item.ident.to_string())?;
                if !item.generics.params.is_empty() {
                    return Err(format!(
                        "generic repr(C) type {} is unsupported",
                        item.ident
                    ));
                }
                let Fields::Named(fields) = item.fields else {
                    return Err(format!(
                        "repr(C) type {} must have named fields",
                        item.ident
                    ));
                };
                let mut field_models = Vec::new();
                for field in fields.named {
                    let name = field
                        .ident
                        .ok_or_else(|| format!("unnamed field in {}", item.ident))?
                        .to_string();
                    reject_conditional_attrs(&field.attrs, &format!("{}.{}", item.ident, name))?;
                    if !is_public(&field.vis) {
                        return Err(format!(
                            "private field in public repr(C) type {} is unsupported",
                            item.ident
                        ));
                    }
                    field_models.push(FieldModel {
                        name,
                        ty: abi_type(&field.ty, &imports)?,
                    });
                }
                model.structs.push(StructModel {
                    name: item.ident.to_string(),
                    fields: field_models,
                });
            }
            Item::Enum(item)
                if is_public(&item.vis) && repr_c(&item.attrs, &item.ident.to_string())? =>
            {
                reject_conditional_attrs(&item.attrs, &item.ident.to_string())?;
                return Err(format!("public repr(C) enum {} is unsupported", item.ident));
            }
            Item::Union(item)
                if is_public(&item.vis) && repr_c(&item.attrs, &item.ident.to_string())? =>
            {
                reject_conditional_attrs(&item.attrs, &item.ident.to_string())?;
                return Err(format!(
                    "public repr(C) union {} is unsupported",
                    item.ident
                ));
            }
            Item::Type(item)
                if matches!(
                    item.ident.to_string().as_str(),
                    "u8" | "u32"
                        | "u64"
                        | "i32"
                        | "i64"
                        | "usize"
                        | "c_char"
                        | "c_int"
                        | "OrrHost"
                        | "OrrHostConfig"
                        | "OrrClientConfig"
                        | "OrrSessionStatus"
                ) =>
            {
                return Err(format!(
                    "type alias shadows a supported ABI type: {}",
                    item.ident
                ));
            }
            Item::Const(item)
                if is_public(&item.vis) && item.ident.to_string().starts_with("ORR_") =>
            {
                reject_conditional_attrs(&item.attrs, &item.ident.to_string())?;
                model.constants.push(ConstantModel {
                    name: item.ident.to_string(),
                    ty: abi_type(&item.ty, &imports)?,
                    value: constant_value(&item.expr)?,
                });
            }
            Item::Fn(item) => {
                let name = item.sig.ident.to_string();
                if item
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("cfg_attr"))
                {
                    return Err(format!(
                        "cfg_attr on ABI-root function {name} is unsupported"
                    ));
                }
                if item.attrs.iter().any(|attr| {
                    attr.path().is_ident("unsafe")
                        && (has_no_mangle(std::slice::from_ref(attr))
                            || has_export_name(std::slice::from_ref(attr)))
                }) {
                    return Err(format!(
                        "unsafe-wrapped symbol attributes on {name} are unsupported"
                    ));
                }
                if has_export_name(&item.attrs) {
                    return Err(format!("export_name on {name} is unsupported"));
                }
                let has_mangle = has_no_mangle(&item.attrs);
                let abi = item
                    .sig
                    .abi
                    .as_ref()
                    .and_then(|abi| abi.name.as_ref())
                    .map(|name| name.value())
                    .unwrap_or_default();
                if has_mangle && !is_public(&item.vis) {
                    return Err(format!("private no_mangle export {name} is unsupported"));
                }
                if is_public(&item.vis) && abi == "C" && !has_mangle {
                    return Err(format!(
                        "public extern C function {name} must use no_mangle"
                    ));
                }
                if !has_mangle {
                    continue;
                }
                reject_conditional_attrs(&item.attrs, &name)?;
                if abi != "C" || item.sig.variadic.is_some() || !item.sig.generics.params.is_empty()
                {
                    return Err(format!("unsupported ABI signature for {name}"));
                }
                let mut args = Vec::new();
                for input in &item.sig.inputs {
                    let FnArg::Typed(arg) = input else {
                        return Err(format!("receiver in exported function {name}"));
                    };
                    if !matches!(arg.pat.as_ref(), Pat::Ident(_)) {
                        return Err(format!("unsupported argument pattern in {}", name));
                    }
                    reject_conditional_attrs(&arg.attrs, &format!("{name} argument"))?;
                    args.push(abi_type(&arg.ty, &imports)?);
                }
                let result = match &item.sig.output {
                    ReturnType::Default => AbiType {
                        base: "void".to_owned(),
                        pointers: Vec::new(),
                        array_lengths: Vec::new(),
                    },
                    ReturnType::Type(_, ty) => abi_type(ty, &imports)?,
                };
                model.functions.push(FunctionModel { name, result, args });
            }
            Item::Macro(item) if !item.mac.path.is_ident("thread_local") => {
                return Err(format!(
                    "top-level macro {} in the ABI root is unsupported",
                    item.mac
                        .path
                        .segments
                        .iter()
                        .map(|segment| segment.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::")
                ))
            }
            _ => {}
        }
    }

    model.structs.sort_by(|a, b| a.name.cmp(&b.name));
    model.constants.sort_by(|a, b| a.name.cmp(&b.name));
    model.functions.sort_by(|a, b| a.name.cmp(&b.name));
    if model.structs.is_empty() || model.constants.is_empty() || model.functions.is_empty() {
        return Err("Rust ABI inventory unexpectedly empty".to_owned());
    }
    Ok(model)
}

fn function_abi_is_c(sig: &syn::Signature) -> bool {
    sig.abi
        .as_ref()
        .and_then(|abi| abi.name.as_ref())
        .is_some_and(|name| name.value() == "C")
}

fn resolve_module_file(
    module: &syn::ItemMod,
    parent_file: &Path,
    is_root: bool,
) -> Result<PathBuf, String> {
    if module
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg_attr"))
    {
        return Err(format!(
            "#[cfg_attr] module {} is unsupported by ABI inventory",
            module.ident
        ));
    }
    if module.attrs.iter().any(|attr| attr.path().is_ident("path")) {
        return Err(format!(
            "#[path] module {} is unsupported by ABI inventory",
            module.ident
        ));
    }
    let parent = parent_file
        .parent()
        .ok_or_else(|| "module has no parent directory".to_owned())?;
    let base = if is_root || parent_file.file_name().is_some_and(|name| name == "mod.rs") {
        parent.to_path_buf()
    } else {
        parent.join(
            parent_file
                .file_stem()
                .ok_or_else(|| "module has no file stem".to_owned())?,
        )
    };
    let flat = base.join(format!("{}.rs", module.ident));
    let nested = base.join(module.ident.to_string()).join("mod.rs");
    match (flat.is_file(), nested.is_file()) {
        (true, false) => Ok(flat),
        (false, true) => Ok(nested),
        (true, true) => Err(format!("ambiguous Rust module files for {}", module.ident)),
        (false, false) => Err(format!("module file for {} was not found", module.ident)),
    }
}

fn reject_module_abi_items(items: &[Item], file: &Path, is_root: bool) -> Result<(), String> {
    for item in items {
        match item {
            Item::Struct(item)
                if is_public(&item.vis)
                    && item
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("cfg_attr")) =>
            {
                return Err(format!(
                    "cfg_attr on public ABI candidate {} is unsupported",
                    item.ident
                ));
            }
            Item::Enum(item)
                if is_public(&item.vis)
                    && item
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("cfg_attr")) =>
            {
                return Err(format!(
                    "cfg_attr on public ABI candidate {} is unsupported",
                    item.ident
                ));
            }
            Item::Union(item)
                if is_public(&item.vis)
                    && item
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("cfg_attr")) =>
            {
                return Err(format!(
                    "cfg_attr on public ABI candidate {} is unsupported",
                    item.ident
                ));
            }
            Item::Fn(item)
                if item
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("cfg_attr")) =>
            {
                return Err(format!(
                    "cfg_attr on function {} is unsupported",
                    item.sig.ident
                ));
            }
            Item::Static(item)
                if item
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("cfg_attr")) =>
            {
                return Err(format!("cfg_attr on static {} is unsupported", item.ident));
            }
            Item::Struct(item)
                if !is_root
                    && is_public(&item.vis)
                    && repr_c(&item.attrs, &item.ident.to_string())? =>
            {
                return Err(format!(
                    "out-of-root repr(C) struct {} is unsupported",
                    item.ident
                ));
            }
            Item::Enum(item)
                if !is_root
                    && is_public(&item.vis)
                    && repr_c(&item.attrs, &item.ident.to_string())? =>
            {
                return Err(format!(
                    "out-of-root repr(C) enum {} is unsupported",
                    item.ident
                ));
            }
            Item::Union(item)
                if !is_root
                    && is_public(&item.vis)
                    && repr_c(&item.attrs, &item.ident.to_string())? =>
            {
                return Err(format!(
                    "out-of-root repr(C) union {} is unsupported",
                    item.ident
                ));
            }
            Item::Const(item)
                if !is_root
                    && is_public(&item.vis)
                    && item.ident.to_string().starts_with("ORR_") =>
            {
                return Err(format!(
                    "out-of-root ORR constant {} is unsupported",
                    item.ident
                ));
            }
            Item::Fn(item) => {
                if !is_root
                    && (has_no_mangle(&item.attrs)
                        || has_export_name(&item.attrs)
                        || (is_public(&item.vis) && function_abi_is_c(&item.sig)))
                {
                    return Err(format!(
                        "out-of-root exported C function {} is unsupported",
                        item.sig.ident
                    ));
                }
            }
            Item::Static(item) if has_no_mangle(&item.attrs) || has_export_name(&item.attrs) => {
                return Err(format!("exported static {} is unsupported", item.ident));
            }
            Item::Macro(item) if !(is_root && item.mac.path.is_ident("thread_local")) => {
                return Err(format!(
                    "macro items in ABI module {} are unsupported",
                    file.display()
                ))
            }
            Item::Mod(module) => {
                if module
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("cfg_attr"))
                {
                    return Err(format!(
                        "#[cfg_attr] module {} is unsupported by ABI inventory",
                        module.ident
                    ));
                }
                if let Some((_, inline_items)) = &module.content {
                    reject_module_abi_items(inline_items, file, false)?;
                } else {
                    if !is_root {
                        return Err(format!(
                            "nested out-of-line module {} is unsupported",
                            module.ident
                        ));
                    }
                    let path = resolve_module_file(module, file, true)?;
                    let source = fs::read_to_string(&path)
                        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
                    let parsed = syn::parse_file(&source)
                        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
                    reject_module_abi_items(&parsed.items, &path, false)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_module_abi(source_file: &Path, source: &str) -> Result<(), String> {
    let parsed = syn::parse_file(source)
        .map_err(|error| format!("cannot parse {}: {error}", source_file.display()))?;
    reject_module_abi_items(&parsed.items, source_file, true)
}

fn layout_for(
    ty: &AbiType,
    primitive: &BTreeMap<String, (usize, usize)>,
) -> Result<(usize, usize), String> {
    ty.layout(primitive)
}

fn align_up(value: usize, align: usize) -> Result<usize, String> {
    let mask = align
        .checked_sub(1)
        .ok_or_else(|| "zero ABI alignment".to_owned())?;
    value
        .checked_add(mask)
        .map(|n| n & !mask)
        .ok_or_else(|| "ABI layout overflow".to_owned())
}

fn struct_layout(
    item: &StructModel,
    primitive: &BTreeMap<String, (usize, usize)>,
) -> Result<StructLayout, String> {
    let mut offset = 0usize;
    let mut struct_align = 1usize;
    let mut offsets = Vec::new();
    for field in &item.fields {
        let (size, align) = layout_for(&field.ty, primitive)?;
        offset = align_up(offset, align)?;
        offsets.push(offset);
        offset = offset
            .checked_add(size)
            .ok_or_else(|| "ABI layout overflow".to_owned())?;
        struct_align = struct_align.max(align);
    }
    Ok(StructLayout {
        size: align_up(offset, struct_align)?,
        align: struct_align,
        offsets,
    })
}

fn primitive_layouts() -> BTreeMap<String, (usize, usize)> {
    let mut layouts = BTreeMap::new();
    macro_rules! add {
        ($key:literal, $type:ty) => {
            layouts.insert(
                $key.to_owned(),
                (std::mem::size_of::<$type>(), std::mem::align_of::<$type>()),
            );
        };
    }
    add!("uint8_t", u8);
    add!("uint32_t", u32);
    add!("uint64_t", u64);
    add!("int32_t", i32);
    add!("int64_t", i64);
    add!("size_t", usize);
    add!("char", std::ffi::c_char);
    add!("int", std::ffi::c_int);
    add!("*const u8", *const u8);
    layouts
}

fn rust_struct_layout(name: &str) -> Option<(usize, usize)> {
    macro_rules! layout {
        ($type:path) => {
            (std::mem::size_of::<$type>(), std::mem::align_of::<$type>())
        };
    }
    match name {
        "OrrHostConfig" => Some(layout!(orr_ffi::OrrHostConfig)),
        "OrrClientConfig" => Some(layout!(orr_ffi::OrrClientConfig)),
        "OrrSessionStatus" => Some(layout!(orr_ffi::OrrSessionStatus)),
        // Opaque handles have no repr(C) object layout exposed to callers.
        _ => None,
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Inventory {
    structs: BTreeMap<String, Vec<String>>,
    constants: BTreeSet<String>,
    functions: BTreeSet<String>,
}

fn rust_inventory(model: &AbiModel) -> Inventory {
    Inventory {
        structs: model
            .structs
            .iter()
            .map(|item| {
                (
                    item.name.clone(),
                    item.fields.iter().map(|field| field.name.clone()).collect(),
                )
            })
            .collect(),
        constants: model
            .constants
            .iter()
            .map(|item| item.name.clone())
            .collect(),
        functions: model
            .functions
            .iter()
            .map(|item| item.name.clone())
            .collect(),
    }
}

fn header_inventory(header: &str) -> Result<Inventory, String> {
    let comment_regex = Regex::new(r"(?s)/\*.*?\*/|//[^\n]*").unwrap();
    let uncommented = comment_regex.replace_all(header, " ");
    if Regex::new(r"\b(?:enum|union)\b")
        .unwrap()
        .is_match(&uncommented)
    {
        return Err("C enums and unions are unsupported in the ABI header".to_owned());
    }
    let functions: BTreeSet<String> =
        Regex::new(r"(?m)^\s*ORR_API\s+[^;\n]*?\b(orr_[A-Za-z0-9_]+)\s*\(")
            .unwrap()
            .captures_iter(&uncommented)
            .map(|capture| capture[1].to_owned())
            .collect();
    let all_functions: BTreeSet<String> = Regex::new(r"\b(orr_[A-Za-z0-9_]+)\s*\(")
        .unwrap()
        .captures_iter(&uncommented)
        .map(|capture| capture[1].to_owned())
        .collect();
    if all_functions != functions {
        return Err(format!(
            "C header has unrecognized or unexported function declarations: all={all_functions:?}, ORR_API={functions:?}"
        ));
    }
    let struct_regex = Regex::new(r"(?s)typedef\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{([^{}]*)\}\s*[A-Za-z_][A-Za-z0-9_]*\s*;").unwrap();
    // Keep this intentionally narrower than C declarators: one supported type
    // and one plain identifier per declaration. Comma declarators, arrays,
    // bitfields, unions, and nested types must not be guessed from a tail name.
    let field_regex = Regex::new(r"^(?:(?:const\s+)?char\s*\*|(?:u?int(?:8|32|64)_t|size_t|int))\s+([A-Za-z_][A-Za-z0-9_]*)$").unwrap();
    let mut structs = BTreeMap::new();
    for capture in struct_regex.captures_iter(&uncommented) {
        let name = capture[1].to_owned();
        let body = &capture[2];
        let mut fields = Vec::new();
        for declaration in body
            .split(';')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let declaration = declaration.split("//").next().unwrap_or(declaration).trim();
            let declaration = declaration.split("/*").next().unwrap_or(declaration).trim();
            let field = field_regex.captures(declaration).ok_or_else(|| {
                format!("unsupported C field declaration in {name}: {declaration}")
            })?[1]
                .to_owned();
            fields.push(field);
        }
        if structs.insert(name.clone(), fields).is_some() {
            return Err(format!("duplicate C struct declaration {name}"));
        }
    }
    let constants = Regex::new(r"(?m)^\s*#\s*define\s+(ORR_[A-Z0-9_]+)\b")
        .unwrap()
        .captures_iter(&uncommented)
        .map(|capture| capture[1].to_owned())
        .filter(|name| name != "ORR_API")
        .collect();
    let expected_definitions =
        Regex::new(r"(?m)^\s*typedef\s+struct\s+[A-Za-z_][A-Za-z0-9_]*\s*\{")
            .unwrap()
            .find_iter(&uncommented)
            .count();
    if expected_definitions != structs.len() {
        return Err("C struct uses unsupported nested/anonymous aggregate syntax".to_owned());
    }
    Ok(Inventory {
        structs,
        constants,
        functions,
    })
}

fn inventory_diff(
    expected: &BTreeSet<String>,
    found: &BTreeSet<String>,
    kind: &str,
) -> Result<(), String> {
    let missing: Vec<_> = expected.difference(found).cloned().collect();
    let extra: Vec<_> = found.difference(expected).cloned().collect();
    if missing.is_empty() && extra.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "C header {kind} inventory differs: missing={missing:?}, extra={extra:?}"
        ))
    }
}

fn assert_inventory(model: &AbiModel, header: &str) -> Result<(), String> {
    let rust = rust_inventory(model);
    let c = header_inventory(header)?;
    inventory_diff(
        &rust.structs.keys().cloned().collect(),
        &c.structs.keys().cloned().collect(),
        "struct",
    )?;
    for (name, expected_fields) in &rust.structs {
        let actual_fields = c.structs.get(name).expect("struct inventory checked");
        if actual_fields != expected_fields {
            return Err(format!("C header field inventory differs for {name}: expected {expected_fields:?}, found {actual_fields:?}"));
        }
    }
    inventory_diff(&rust.constants, &c.constants, "constant")?;
    inventory_diff(&rust.functions, &c.functions, "function")
}

fn c_checks(model: &AbiModel, header: &str) -> Result<String, String> {
    assert_inventory(model, header)?;
    let primitive = primitive_layouts();
    let mut layouts = BTreeMap::new();
    for item in &model.structs {
        let layout = struct_layout(item, &primitive)?;
        let actual = rust_struct_layout(&item.name)
            .ok_or_else(|| format!("no Rust layout binding for repr(C) type {}", item.name))?;
        if (layout.size, layout.align) != actual {
            return Err(format!(
                "AST repr(C) layout for {} is ({}, {}), actual Rust layout is ({}, {})",
                item.name, layout.size, layout.align, actual.0, actual.1
            ));
        }
        layouts.insert(item.name.clone(), layout);
    }

    let mut out =
        String::from("#include <stddef.h>\n#include <stdint.h>\n#include \"orrery.h\"\n\n");
    for item in &model.structs {
        let layout = &layouts[&item.name];
        out.push_str(&format!(
            "_Static_assert(sizeof({}) == {}, \"struct size: {}\");\n",
            item.name, layout.size, item.name
        ));
        out.push_str(&format!(
            "_Static_assert(_Alignof({}) == {}, \"struct alignment: {}\");\n",
            item.name, layout.align, item.name
        ));
        for (index, field) in item.fields.iter().enumerate() {
            let typedef = format!("OrrExpected_{}_{}", item.name, field.name);
            out.push_str(&format!("typedef {};\n", field.ty.c_declaration(&typedef)));
            out.push_str(&format!(
                "_Static_assert(_Generic(&(({} *)0)->{}, {} *: 1, default: 0), \"field type: {}.{}\");\n",
                item.name, field.name, typedef, item.name, field.name
            ));
            out.push_str(&format!(
                "_Static_assert(offsetof({}, {}) == {}, \"field offset: {}.{}\");\n",
                item.name, field.name, layout.offsets[index], item.name, field.name
            ));
        }
        out.push('\n');
    }
    for constant in &model.constants {
        let c_ty = constant.ty.c_type();
        out.push_str(&format!(
            "_Static_assert(_Generic(({}), {}: 1, default: 0), \"constant type: {}\");\n",
            constant.name, c_ty, constant.name
        ));
        out.push_str(&format!(
            "_Static_assert(({}) == (({}){}), \"constant value: {}\");\n",
            constant.name, c_ty, constant.value, constant.name
        ));
    }
    out.push('\n');
    for function in &model.functions {
        let args = if function.args.is_empty() {
            "void".to_owned()
        } else {
            function
                .args
                .iter()
                .map(AbiType::c_type)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let signature = format!("OrrExpectedFn_{}", function.name);
        let return_declaration = function
            .result
            .c_declaration(&format!("(*{signature})({args})"));
        out.push_str(&format!("typedef {return_declaration};\n"));
        out.push_str(&format!(
            "_Static_assert(_Generic(&{}, {}: 1, default: 0), \"function signature: {}\");\n",
            function.name, signature, function.name
        ));
    }
    Ok(out)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "orr_ffi_header_parity_{}_{}_{}",
                std::process::id(),
                label,
                sequence
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compiler() -> Result<Tool, String> {
    Build::new()
        .target(TARGET)
        .host(TARGET)
        .std("c11")
        .opt_level(1)
        .debug(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .try_get_compiler()
        .map_err(|error| format!("no C compiler: {error}"))
}

fn compile_only(
    tool: &Tool,
    directory: &Path,
    header: &str,
    source: &str,
) -> Result<Output, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create C probe directory: {error}"))?;
    fs::write(directory.join("orrery.h"), header)
        .map_err(|error| format!("cannot write probe header: {error}"))?;
    fs::write(directory.join("parity.c"), source)
        .map_err(|error| format!("cannot write C probe: {error}"))?;
    let source_path = directory.join("parity.c");
    let object_path = directory.join(if tool.is_like_msvc() {
        "parity.obj"
    } else {
        "parity.o"
    });
    let mut command = tool.to_command();
    if tool.is_like_msvc() {
        command.args([
            OsStr::new("/nologo"),
            OsStr::new("/std:c11"),
            OsStr::new("/W4"),
            OsStr::new("/WX"),
            OsStr::new("/c"),
        ]);
        command
            .arg(&source_path)
            .arg(format!("/Fo{}", object_path.display()));
    } else {
        command.args([
            OsStr::new("-std=c11"),
            OsStr::new("-Wall"),
            OsStr::new("-Wextra"),
            OsStr::new("-Werror"),
            OsStr::new("-Werror=incompatible-pointer-types"),
            OsStr::new("-c"),
        ]);
        command.arg(&source_path).arg("-o").arg(&object_path);
    }
    if tool.is_like_msvc() {
        command.arg(format!("/I{}", directory.display()));
    } else {
        command.arg(format!("-I{}", directory.display()));
    }
    let output = command
        .output()
        .map_err(|error| format!("cannot run C compiler {}: {error}", tool.path().display()))?;
    Ok(output)
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn maybe_skip(why: &str) -> bool {
    if std::env::var("ORR_REQUIRE_C_COMPILER").is_ok_and(|value| value == "1") {
        panic!("{why} (ORR_REQUIRE_C_COMPILER=1)");
    }
    eprintln!("SKIPPED: {why} (set ORR_REQUIRE_C_COMPILER=1 to make this an error)");
    true
}

fn source_and_model() -> (String, String, AbiModel, String) {
    let source = fs::read_to_string(ffi_source()).expect("read orr_ffi/src/lib.rs");
    let header = fs::read_to_string(ffi_header()).expect("read include/orrery.h");
    validate_module_abi(&ffi_source(), &source)
        .unwrap_or_else(|error| panic!("unsupported ABI declaration in Rust module: {error}"));
    let model =
        parse_model(&source).unwrap_or_else(|error| panic!("invalid Rust C ABI model: {error}"));
    let checks = c_checks(&model, &header)
        .unwrap_or_else(|error| panic!("invalid C header ABI model: {error}"));
    (source, header, model, checks)
}

#[test]
fn checked_in_header_matches_rust_abi() {
    let (_, header, _, checks) = source_and_model();
    let tool = match compiler() {
        Ok(tool) => tool,
        Err(error) => {
            maybe_skip(&error);
            return;
        }
    };
    println!(
        "C ABI parity compiler: {} for {TARGET}",
        tool.path().display()
    );
    let temp = TempDir::new("canonical");
    let output = compile_only(&tool, &temp.0.join("canonical"), &header, &checks)
        .unwrap_or_else(|error| panic!("C ABI parity compile could not run: {error}"));
    assert!(
        output.status.success(),
        "C ABI parity translation unit failed to compile:\n{}",
        output_text(&output)
    );
}

#[test]
fn header_mutations_are_rejected_by_the_c_compiler() {
    let (_, header, _, checks) = source_and_model();
    let tool = match compiler() {
        Ok(tool) => tool,
        Err(error) => {
            maybe_skip(&error);
            return;
        }
    };
    let mutations = [
        (
            "field_order",
            "    uint32_t struct_size;  /* sizeof(OrrHostConfig): set it */\n    uint32_t flags;        /* ORR_HOST_* */",
            "    uint32_t flags;        /* ORR_HOST_* */\n    uint32_t struct_size;  /* sizeof(OrrHostConfig): set it */",
            "field offset: OrrHostConfig.struct_size",
        ),
        (
            "field_type",
            "uint32_t struct_size;  /* sizeof(OrrHostConfig): set it */",
            "int32_t struct_size;  /* sizeof(OrrHostConfig): set it */",
            "field type: OrrHostConfig.struct_size",
        ),
        (
            "pointer_constness",
            "const char* server;           /* \"host:port\" (UTF-8), required */",
            "char* server;                 /* \"host:port\" (UTF-8), required */",
            "field type: OrrClientConfig.server",
        ),
        (
            "constant",
            "#define ORR_ABI_VERSION 2u",
            "#define ORR_ABI_VERSION 3u",
            "constant value: ORR_ABI_VERSION",
        ),
        (
            "function_signature",
            "ORR_API int orr_control(OrrHost* host, int op, int64_t arg);",
            "ORR_API int orr_control(OrrHost* host, int op, uint64_t arg);",
            "function signature: orr_control",
        ),
    ];

    let root = TempDir::new("mutations");
    for (name, old, new, expected_message) in mutations {
        let count = header.matches(old).count();
        assert_eq!(
            count, 1,
            "mutation fixture {name} must match once (matched {count})"
        );
        let mutated = header.replacen(old, new, 1);
        let output =
            compile_only(&tool, &root.0.join(name), &mutated, &checks).unwrap_or_else(|error| {
                panic!("mutation fixture {name} compiler invocation failed: {error}")
            });
        let text = output_text(&output);
        assert!(
            !output.status.success(),
            "header mutation {name} unexpectedly compiled"
        );
        assert!(text.contains(expected_message), "header mutation {name} failed for an unrelated reason; expected `{expected_message}`:\n{text}");
        println!("PASS: {name} drift rejected by {}", tool.path().display());
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    fn source_with(extra: &str) -> String {
        format!(
            r#"
            use std::ffi::c_int;
            {extra}
            pub const ORR_TEST: c_int = 0;
            #[repr(C)] pub struct TestWire {{ pub value: u32 }}
            #[no_mangle] pub extern "C" fn orr_test(value: u32) -> u32 {{ value }}
            "#
        )
    }

    #[test]
    fn unsupported_type_paths_and_shadowing_fail_closed() {
        let qualified_alias = source_with("").replace("value: u32", "value: alias::u64");
        assert!(parse_model(&qualified_alias).is_err());

        let imported_alias = source_with("use alias::u64;");
        assert!(parse_model(&imported_alias).is_err());

        let primitive_shadow = source_with("type u32 = i32;");
        assert!(parse_model(&primitive_shadow).is_err());

        let namespace_shadow = source_with("mod std {}");
        assert!(parse_model(&namespace_shadow).is_err());

        let conditional_argument = source_with("").replace(
            "fn orr_test(value: u32)",
            "fn orr_test(#[cfg(any())] value: u32)",
        );
        let error = parse_model(&conditional_argument).unwrap_err();
        assert!(
            error.contains("conditional ABI declaration on orr_test argument"),
            "{error}"
        );
    }

    #[test]
    fn unsupported_repr_and_symbol_attributes_fail_closed() {
        assert!(parse_model(&source_with("")).is_ok());
        let generic_repr = source_with("").replace(
            "#[repr(C)] pub struct TestWire { pub value: u32 }",
            "#[repr(C)] pub struct TestWire<T> { pub value: u32, marker: core::marker::PhantomData<T> }",
        );
        assert!(parse_model(&generic_repr).is_err());

        let repeated_repr = source_with("").replace(
            "#[repr(C)] pub struct TestWire",
            "#[repr(C)] #[repr(C)] pub struct TestWire",
        );
        assert!(parse_model(&repeated_repr).is_err());

        let packed_repr = source_with("").replace(
            "#[repr(C)] pub struct TestWire",
            "#[repr(C, packed)] pub struct TestWire",
        );
        assert!(parse_model(&packed_repr).is_err());

        let private_export =
            source_with("").replace("#[no_mangle] pub extern", "#[no_mangle] extern");
        assert!(parse_model(&private_export).is_err());

        let unsafe_export =
            source_with("").replace("#[no_mangle] pub extern", "#[unsafe(no_mangle)] pub extern");
        assert!(parse_model(&unsafe_export).is_err());

        let conditional_export =
            source_with("#[cfg_attr(unix, no_mangle)] extern \"C\" fn orr_hidden() {}");
        assert!(parse_model(&conditional_export).is_err());

        let conditional_repr = source_with("").replace(
            "#[repr(C)] pub struct TestWire",
            "#[cfg_attr(unix, repr(C))] pub struct TestWire",
        );
        assert!(parse_model(&conditional_repr).is_err());
    }

    #[test]
    fn exported_statics_fail_closed_at_the_root() {
        let temp = TempDir::new("root_statics");
        let root = temp.0.join("lib.rs");
        assert!(validate_module_abi(&root, &source_with("")).is_ok());
        for declaration in [
            "#[no_mangle] static HIDDEN: u32 = 0;",
            "#[export_name = \"orr_hidden\"] pub static HIDDEN: u32 = 0;",
            "#[unsafe(export_name = \"orr_hidden\")] pub static HIDDEN: u32 = 0;",
            "#[unsafe(no_mangle)] pub static HIDDEN: u32 = 0;",
        ] {
            let error = validate_module_abi(&root, &source_with(declaration))
                .expect_err("an exported static must not disappear from the ABI inventory");
            assert!(error.contains("exported static"), "{error}");
        }
    }

    #[test]
    fn out_of_root_module_exports_fail_closed() {
        let temp = TempDir::new("module_fixture");
        let source_dir = temp.0.join("src");
        fs::create_dir_all(&source_dir).unwrap();
        let root = source_dir.join("lib.rs");
        fs::write(&root, "mod client;\n").unwrap();
        fs::write(
            source_dir.join("client.rs"),
            "#[no_mangle] pub extern \"C\" fn orr_hidden() {}\n",
        )
        .unwrap();
        assert!(validate_module_abi(&root, &fs::read_to_string(&root).unwrap()).is_err());

        fs::write(
            &root,
            "#[cfg_attr(all(), path = \"alternate.rs\")] mod client;\n",
        )
        .unwrap();
        fs::write(
            source_dir.join("client.rs"),
            "pub fn ordinary_helper() {}\n",
        )
        .unwrap();
        fs::write(
            source_dir.join("alternate.rs"),
            "#[no_mangle] pub extern \"C\" fn orr_alternate() {}\n",
        )
        .unwrap();
        let error = validate_module_abi(&root, &fs::read_to_string(&root).unwrap()).unwrap_err();
        assert!(error.contains("#[cfg_attr] module client"), "{error}");

        fs::write(&root, "mod client;\n").unwrap();
        fs::write(
            source_dir.join("client.rs"),
            "#[cfg_attr(unix, no_mangle)] extern \"C\" fn orr_hidden() {}\n",
        )
        .unwrap();
        let error = validate_module_abi(&root, &fs::read_to_string(&root).unwrap()).unwrap_err();
        assert!(error.contains("cfg_attr on function orr_hidden"), "{error}");
    }

    #[test]
    fn c_field_inventory_rejects_extra_fields_and_anonymous_unions() {
        let source = fs::read_to_string(ffi_source()).unwrap();
        let model = parse_model(&source).unwrap();
        let header = fs::read_to_string(ffi_header()).unwrap();

        let extra_field = header.replace(
            "uint32_t listen_port;  /* with ORR_HOST_LISTEN: port, 0 = any free port (see orr_host_url) */",
            "uint32_t listen_port;  /* with ORR_HOST_LISTEN: port, 0 = any free port (see orr_host_url) */\n    uint32_t extra;",
        );
        assert_ne!(extra_field, header);
        assert!(assert_inventory(&model, &extra_field).is_err());

        let anonymous_union = header.replace(
            "uint32_t flags;        /* ORR_HOST_* */",
            "union { uint32_t flags; uint32_t extra; };",
        );
        assert_ne!(anonymous_union, header);
        assert!(header_inventory(&anonymous_union).is_err());

        let comma_declarator = header.replace(
            "uint32_t flags;        /* ORR_HOST_* */",
            "uint32_t extra, flags; /* ORR_HOST_* */",
        );
        assert_ne!(comma_declarator, header);
        assert!(header_inventory(&comma_declarator).is_err());

        let extra_prototype = header.replace(
            "#ifdef __cplusplus\n}\n#endif",
            "ORR_API int orr_untracked(void);\n#ifdef __cplusplus\n}\n#endif",
        );
        assert_ne!(extra_prototype, header);
        assert!(assert_inventory(&model, &extra_prototype).is_err());

        let unexported_prototype = header.replace(
            "#ifdef __cplusplus\n}\n#endif",
            "int orr_untracked(void);\n#ifdef __cplusplus\n}\n#endif",
        );
        assert_ne!(unexported_prototype, header);
        assert!(header_inventory(&unexported_prototype).is_err());

        let extra_enum = header.replace(
            "#ifdef __cplusplus\n}\n#endif",
            "enum { ORR_UNEXPECTED = 0 };\n#ifdef __cplusplus\n}\n#endif",
        );
        assert_ne!(extra_enum, header);
        assert!(header_inventory(&extra_enum).is_err());
    }
}
