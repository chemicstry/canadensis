extern crate canadensis_bit_length_set;
extern crate canadensis_dsdl_frontend;
extern crate heck;

mod impl_constants;
mod impl_data_type;
mod impl_deserialize;
mod impl_serialize;
mod module_tree;
mod size_bits;

use canadensis_bit_length_set::BitLengthSet;
use heck::{CamelCase, SnakeCase};
use std::collections::BTreeMap;
use std::iter;

use crate::module_tree::ModuleTree;
use canadensis_dsdl_frontend::compiled::package::CompiledPackage;
use canadensis_dsdl_frontend::compiled::{
    CompiledDsdl, DsdlKind, Extent, FieldKind, Message, MessageKind, Struct, Union,
};
use canadensis_dsdl_frontend::constants::Constants;
use canadensis_dsdl_frontend::types::{PrimitiveType, ResolvedScalarType, ResolvedType};
use canadensis_dsdl_frontend::TypeKey;

/// Returns a Cargo.toml fragment with the packages that the generated code depends on
pub fn generated_code_dependencies() -> String {
    String::from(
        r#"[dependencies]
half = { version = "1.8", features = ["zerocopy"] }
heapless = "0.7.7"
zerocopy = "0.6.0"
canadensis_core = "0.2.0"
canadensis_encoding = "0.2.0"
[dev-dependencies]
memoffset = "0.6.4"
"#,
    )
}

/// Generates a Rust module from the provided package of DSDL
///
/// `external_packages` is a map from DSDL package names to Rust module paths. A DSDL type in
/// one of these packages (or any subpackage) will not have Rust code generated. Instead, any
/// references to that type will refer to external Rust code in the corresponding module.
pub fn generate_code(
    package: &CompiledPackage,
    external_packages: &BTreeMap<Vec<String>, Vec<String>>,
) -> GeneratedModule {
    let mut generated_types = Vec::new();

    for (key, dsdl) in package {
        if external_module(key.name().path(), &external_packages).is_none() {
            // Generate a non-external type
            generate_from_dsdl(key, dsdl, &external_packages, &mut generated_types);
        }
    }
    let tree: ModuleTree = generated_types.into_iter().collect();
    GeneratedModule { tree }
}

/// If the provided key matches an external package, this function returns the Rust module path
/// that contains the already-generated type(s).
fn external_module(
    package: &[String],
    external_packages: &BTreeMap<Vec<String>, Vec<String>>,
) -> Option<Vec<String>> {
    // Check for the path and all prefixes
    for i in (1..=package.len()).rev() {
        // Split path_segments into a start segment, which matches an external package name,
        // and an end segment, which will get appended to the Rust module path
        let (start, end) = package.split_at(i);
        if let Some(rust_module) = external_packages.get(start) {
            // Convert the end segments into Rust module path segments and append
            let mut full_module = Vec::with_capacity(end.len() + rust_module.len());
            full_module.extend(rust_module.iter().cloned());
            full_module.extend(end.iter().cloned());

            return Some(full_module);
        }
    }
    None
}

fn generate_from_dsdl(
    key: &TypeKey,
    dsdl: &CompiledDsdl,
    external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    items: &mut Vec<GeneratedItem>,
) {
    match &dsdl.kind {
        DsdlKind::Message(message) => {
            let rust_type = RustTypeName::for_message_type(key, external_packages);

            if let Some(subject_id) = dsdl.fixed_port_id {
                // Add a module-level constant with the subject ID
                let constant_name = RustTypeName {
                    internal: true,
                    path: rust_type.path.clone(),
                    type_name: "SUBJECT".into(),
                };
                items.push(GeneratedItem::Constant {
                    name: constant_name,
                    ty: "::canadensis_core::SubjectId".into(),
                    value: format!(
                        "::canadensis_core::SubjectId::from_truncating({})",
                        subject_id
                    ),
                });
            }

            items.push(GeneratedItem::Type(generate_rust_type(
                key,
                message,
                &rust_type,
                message.extent().clone(),
                MessageRole::Message,
                external_packages,
            )));
        }
        DsdlKind::Service { request, response } => {
            let rust_type = ServiceTypeNames::for_service_type(key, external_packages);

            if let Some(service_id) = dsdl.fixed_port_id {
                // Add a module-level constant with the service ID
                let constant_name = RustTypeName {
                    internal: true,
                    path: rust_type.request.path.clone(),
                    type_name: "SERVICE".into(),
                };
                items.push(GeneratedItem::Constant {
                    name: constant_name,
                    ty: "::canadensis_core::ServiceId".into(),
                    value: format!(
                        "::canadensis_core::ServiceId::from_truncating({})",
                        service_id
                    ),
                });
            }

            items.push(GeneratedItem::Type(generate_rust_type(
                key,
                request,
                &rust_type.request,
                request.extent().clone(),
                MessageRole::Request,
                external_packages,
            )));
            items.push(GeneratedItem::Type(generate_rust_type(
                key,
                response,
                &rust_type.response,
                response.extent().clone(),
                MessageRole::Response,
                external_packages,
            )));
        }
    }
}

/// A module of generated Rust code
pub struct GeneratedModule {
    tree: ModuleTree,
}

fn generate_rust_type(
    key: &TypeKey,
    message: &Message,
    rust_type: &RustTypeName,
    extent: Extent,
    role: MessageRole,
    external_packages: &BTreeMap<Vec<String>, Vec<String>>,
) -> GeneratedType {
    let length = message.bit_length().clone();
    match message.kind() {
        MessageKind::Struct(uavcan_struct) => GeneratedType::new_struct(
            key,
            rust_type.clone(),
            length,
            extent,
            role,
            uavcan_struct,
            message.constants().clone(),
            external_packages,
        ),
        MessageKind::Union(uavcan_union) => GeneratedType::new_enum(
            key,
            rust_type.clone(),
            length,
            extent,
            role,
            uavcan_union,
            message.constants().clone(),
            external_packages,
        ),
    }
}

enum GeneratedItem {
    Type(GeneratedType),
    Constant {
        name: RustTypeName,
        ty: String,
        value: String,
    },
}

impl GeneratedItem {
    pub fn name(&self) -> &RustTypeName {
        match self {
            GeneratedItem::Type(ty) => &ty.name,
            GeneratedItem::Constant { name, .. } => name,
        }
    }
}

struct GeneratedType {
    uavcan_name: String,
    name: RustTypeName,
    size: BitLengthSet,
    extent: Extent,
    role: MessageRole,
    kind: GeneratedTypeKind,
    constants: Constants,
}

enum GeneratedTypeKind {
    Struct(GeneratedStruct),
    Enum(GeneratedEnum),
}

impl GeneratedType {
    pub fn new_struct(
        key: &TypeKey,
        name: RustTypeName,
        size: BitLengthSet,
        extent: Extent,
        role: MessageRole,
        uavcan_struct: &Struct,
        constants: Constants,
        external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    ) -> GeneratedType {
        let fields = uavcan_struct
            .fields
            .iter()
            .cloned()
            .map(|field| match field.kind() {
                FieldKind::Padding(bits) => GeneratedField::Padding(*bits),
                FieldKind::Data { ty, name } => GeneratedField::data(
                    ty.clone(),
                    name.clone(),
                    field.always_aligned(),
                    external_packages,
                ),
            })
            .collect();
        GeneratedType::new(
            key,
            name,
            size,
            extent,
            role,
            GeneratedTypeKind::Struct(GeneratedStruct { fields }),
            constants,
        )
    }
    pub fn new_enum(
        key: &TypeKey,
        name: RustTypeName,
        size: BitLengthSet,
        extent: Extent,
        role: MessageRole,
        uavcan_union: &Union,
        constants: Constants,
        external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    ) -> GeneratedType {
        let variants = uavcan_union
            .variants
            .iter()
            .cloned()
            .map(|variant| {
                GeneratedVariant::new(variant.ty.clone(), variant.name, external_packages)
            })
            .collect();
        GeneratedType::new(
            key,
            name,
            size,
            extent,
            role,
            GeneratedTypeKind::Enum(GeneratedEnum {
                discriminant_bits: uavcan_union.discriminant_bits,
                variants,
            }),
            constants,
        )
    }

    fn new(
        key: &TypeKey,
        name: RustTypeName,
        size: BitLengthSet,
        extent: Extent,
        role: MessageRole,
        kind: GeneratedTypeKind,
        constants: Constants,
    ) -> Self {
        GeneratedType {
            uavcan_name: key.to_string(),
            name,
            size,
            extent,
            role,
            kind,
            constants,
        }
    }

    /// Returns true if this type supports zero-copy serialization and deserialization
    fn supports_zero_copy(&self) -> bool {
        match &self.kind {
            GeneratedTypeKind::Struct(gstruct) => {
                // Things that disqualify a struct from zero-copy:
                // * Non-fixed length
                // * Padding fields
                // * Any field that does not support zero-copy
                // * Padding in the Rust in-memory representation (how do we check that?)

                if !self.size.is_fixed_size() {
                    return false;
                }
                for field in &gstruct.fields {
                    match field {
                        GeneratedField::Data(field) => {
                            if !field.supports_zero_copy() {
                                return false;
                            }
                        }
                        GeneratedField::Padding(_) => return false,
                    }
                }

                true
            }
            GeneratedTypeKind::Enum(_) => false,
        }
    }
}

struct GeneratedStruct {
    fields: Vec<GeneratedField>,
}

enum GeneratedField {
    Data(GeneratedDataField),
    /// A padding field
    ///
    /// The enclosed value is the number of bits
    Padding(u8),
}

struct GeneratedDataField {
    name: String,
    ty: String,
    uavcan_ty: ResolvedType,
    always_aligned: bool,
}

impl GeneratedDataField {
    pub fn supports_zero_copy(&self) -> bool {
        type_supports_zero_copy(&self.uavcan_ty)
    }
}

fn type_supports_zero_copy(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Scalar(scalar) => scalar_supports_zero_copy(scalar),
        ResolvedType::FixedArray { inner, .. } => scalar_supports_zero_copy(inner),
        ResolvedType::VariableArray { .. } => false,
    }
}

fn scalar_supports_zero_copy(scalar: &ResolvedScalarType) -> bool {
    match scalar {
        ResolvedScalarType::Composite { inner, .. } => message_supports_zero_copy(&*inner),
        ResolvedScalarType::Primitive(primitive) => match primitive {
            PrimitiveType::Boolean => false,
            PrimitiveType::Int { bits } | PrimitiveType::UInt { bits, .. } => match bits {
                8 | 16 | 32 | 64 => true,
                _ => false,
            },
            PrimitiveType::Float16 { .. }
            | PrimitiveType::Float32 { .. }
            | PrimitiveType::Float64 { .. } => true,
        },
        ResolvedScalarType::Void { .. } => false,
    }
}

fn message_supports_zero_copy(message: &Message) -> bool {
    if !message.bit_length().is_fixed_size() {
        return false;
    }
    match message.kind() {
        MessageKind::Struct(mstruct) => {
            for field in &mstruct.fields {
                if !field.always_aligned() {
                    return false;
                }
                match field.kind() {
                    FieldKind::Padding(_) => return false,
                    FieldKind::Data { ty, .. } => {
                        if !type_supports_zero_copy(ty) {
                            return false;
                        }
                    }
                }
            }
            true
        }
        MessageKind::Union(_) => false,
    }
}

impl GeneratedField {
    pub fn data(
        ty: ResolvedType,
        name: String,
        always_aligned: bool,
        external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    ) -> Self {
        GeneratedField::Data(GeneratedDataField {
            name: make_rust_identifier(name),
            ty: to_rust_type(&ty, external_packages),
            uavcan_ty: ty,
            always_aligned,
        })
    }
}

struct GeneratedEnum {
    /// The number of bits used for the discriminant, which identifies the active variant
    discriminant_bits: u8,
    /// The enum variants
    variants: Vec<GeneratedVariant>,
}

struct GeneratedVariant {
    name: String,
    ty: String,
    uavcan_ty: ResolvedType,
}

impl GeneratedVariant {
    pub fn new(
        ty: ResolvedType,
        name: String,
        external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    ) -> Self {
        GeneratedVariant {
            name: make_rust_identifier(name).to_camel_case(),
            ty: to_rust_type(&ty, external_packages),
            uavcan_ty: ty,
        }
    }
}

/// The role of a generated message type
enum MessageRole {
    /// A message (not service-related)
    Message,
    /// A service request
    Request,
    /// A service response
    Response,
}

fn to_rust_type(
    ty: &ResolvedType,
    external_packages: &BTreeMap<Vec<String>, Vec<String>>,
) -> String {
    match ty {
        ResolvedType::Scalar(scalar) => scalar_to_rust_type(scalar, external_packages),
        ResolvedType::FixedArray {
            inner: ResolvedScalarType::Primitive(PrimitiveType::Boolean),
            len,
        }
        | ResolvedType::VariableArray {
            inner: ResolvedScalarType::Primitive(PrimitiveType::Boolean),
            max_len: len,
        } => {
            // Use a BitArray
            // Convert from bits to bytes and round up
            let bytes = (*len + 7) / 8;
            format!("::canadensis_encoding::bits::BitArray<{}>", bytes)
        }
        ResolvedType::FixedArray { inner, len } => {
            format!(
                "[{}; {}]",
                scalar_to_rust_type(inner, external_packages),
                len
            )
        }
        ResolvedType::VariableArray { inner, max_len } => {
            format!(
                "::heapless::Vec<{}, {}>",
                scalar_to_rust_type(inner, external_packages),
                max_len
            )
        }
    }
}

fn scalar_to_rust_type(
    scalar: &ResolvedScalarType,
    external_packages: &BTreeMap<Vec<String>, Vec<String>>,
) -> String {
    match scalar {
        ResolvedScalarType::Composite { key, .. } => {
            RustTypeName::for_message_type(key, external_packages).to_string()
        }
        ResolvedScalarType::Primitive(primitive) => match primitive {
            PrimitiveType::Boolean => "bool".to_owned(),
            PrimitiveType::Int { bits, .. } => format!("i{}", round_up_integer_size(*bits)),
            PrimitiveType::UInt { bits, .. } => format!("u{}", round_up_integer_size(*bits)),
            PrimitiveType::Float16 { .. } => "::half::f16".to_owned(),
            PrimitiveType::Float32 { .. } => "f32".to_owned(),
            PrimitiveType::Float64 { .. } => "f64".to_owned(),
        },
        ResolvedScalarType::Void { .. } => "()".to_owned(),
    }
}

fn round_up_integer_size(bits: u8) -> u8 {
    match bits {
        0..=8 => 8,
        9..=16 => 16,
        17..=32 => 32,
        33..=64 => 64,
        65..=u8::MAX => panic!("Integer too large"),
    }
}

/// The path and name of a Rust type
#[derive(Debug, Clone)]
struct RustTypeName {
    /// True for internal types (path begins with `crate::`), false for external types
    /// (path begins with `::`)
    internal: bool,
    path: Vec<String>,
    type_name: String,
}

impl RustTypeName {
    pub fn for_message_type(
        key: &TypeKey,
        external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    ) -> Self {
        let version_module = format!(
            "{}_{}_{}",
            key.name().name().to_snake_case(),
            key.version().major,
            key.version().minor
        );
        let type_name = make_rust_identifier(key.name().name().to_owned());
        match external_module(key.name().path(), external_packages) {
            Some(mut external_module) => {
                // For external types:
                // [UAVCAN package path]::[snake case type name][version]::[type name]

                external_module.push(version_module);
                RustTypeName {
                    internal: false,
                    path: external_module,
                    type_name,
                }
            }
            None => {
                // For internal types:
                // crate::[UAVCAN package path]::[snake case type name][version]::[type name]

                let path = key
                    .name()
                    .path()
                    .iter()
                    .cloned()
                    .map(make_rust_identifier)
                    .chain(iter::once(version_module))
                    .collect();
                RustTypeName {
                    internal: true,
                    path,
                    type_name,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ServiceTypeNames {
    request: RustTypeName,
    response: RustTypeName,
}

impl ServiceTypeNames {
    pub fn for_service_type(
        key: &TypeKey,
        external_packages: &BTreeMap<Vec<String>, Vec<String>>,
    ) -> Self {
        // For service types:
        // [UAVCAN package path]::[snake case type name][version]::[type name][Request/Response]

        let base = RustTypeName::for_message_type(key, external_packages);
        let mut request = base.clone();
        request.type_name.push_str("Request");
        let mut response = base;
        response.type_name.push_str("Response");

        ServiceTypeNames { request, response }
    }
}

fn make_rust_identifier(mut identifier: String) -> String {
    if identifier == "_" {
        // Becomes _0
        identifier.push('0');
        identifier
    } else {
        identifier
    }
}

mod fmt_impl {
    use super::{GeneratedField, GeneratedType, RustTypeName};
    use crate::impl_constants::ImplementConstants;
    use crate::impl_data_type::ImplementDataType;
    use crate::impl_deserialize::ImplementDeserialize;
    use crate::impl_serialize::ImplementSerialize;
    use crate::{GeneratedItem, GeneratedModule, GeneratedTypeKind, GeneratedVariant};
    use std::convert::TryFrom;
    use std::fmt::{Display, Formatter, Result};

    impl Display for RustTypeName {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            if self.internal {
                write!(f, "crate::")?;
            } else {
                write!(f, "::")?;
            }
            for segment in &self.path {
                write!(f, "{}::", segment)?;
            }
            write!(f, "{}", self.type_name)
        }
    }

    impl Display for GeneratedType {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            writeln!(f, "/// `{}`\n///", self.uavcan_name)?;
            let min_size = self.size.min_value();
            let max_size = self.size.max_value();
            if min_size == max_size {
                writeln!(f, "/// Fixed size {} bytes", min_size / 8)?;
            } else {
                writeln!(
                    f,
                    "/// Size ranges from {} to {} bytes",
                    min_size / 8,
                    max_size / 8
                )?;
            }

            // Derive zerocopy traits if possible
            let supports_zero_copy = self.supports_zero_copy();
            if supports_zero_copy {
                writeln!(f, "#[derive(::zerocopy::FromBytes, ::zerocopy::AsBytes)]")?;
                writeln!(f, "#[repr(C, packed)]")?;
            }

            match &self.kind {
                GeneratedTypeKind::Struct(inner) => {
                    writeln!(f, "pub struct {} {{", self.name.type_name)?;
                    for field in &inner.fields {
                        field.fmt(f)?;
                    }
                    writeln!(f, "}}")?;
                }
                GeneratedTypeKind::Enum(inner) => {
                    writeln!(f, "pub enum {} {{", self.name.type_name)?;
                    for variant in &inner.variants {
                        variant.fmt(f)?;
                    }
                    writeln!(f, "}}")?;
                }
            }

            Display::fmt(&ImplementDataType(self), f)?;
            Display::fmt(&ImplementConstants(self), f)?;

            Display::fmt(
                &ImplementSerialize {
                    ty: self,
                    zero_copy: supports_zero_copy,
                },
                f,
            )?;

            Display::fmt(
                &ImplementDeserialize {
                    ty: self,
                    zero_copy: supports_zero_copy,
                },
                f,
            )?;

            if supports_zero_copy {
                // Add some static assertions about the type size and field layout
                writeln!(f, "#[test] fn test_layout() {{")?;
                // Check total size
                writeln!(
                    f,
                    "assert_eq!(::core::mem::size_of::<{}>() * 8, {});",
                    self.name.type_name, min_size
                )?;
                match &self.kind {
                    GeneratedTypeKind::Struct(gstruct) => {
                        let mut expected_offset_bits = 0usize;
                        for field in &gstruct.fields {
                            match field {
                                GeneratedField::Data(field) => {
                                    writeln!(
                                        f,
                                        "assert_eq!(::memoffset::offset_of!({}, {}) * 8, {});",
                                        self.name.type_name, field.name, expected_offset_bits
                                    )?;

                                    // Update expected offset for the next field
                                    let field_size = field.uavcan_ty.size();
                                    let field_size_min = field_size.min_value();
                                    let field_size_max = field_size.max_value();
                                    assert_eq!(field_size_min, field_size_max);
                                    expected_offset_bits +=
                                        usize::try_from(field_size_min).unwrap();
                                }
                                GeneratedField::Padding(bits) => {
                                    expected_offset_bits += usize::from(*bits);
                                }
                            }
                        }
                    }
                    GeneratedTypeKind::Enum(_) => unreachable!("Enums can't be zero-copy"),
                }

                writeln!(f, "}}")?;
            }

            Ok(())
        }
    }

    impl Display for GeneratedField {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            match self {
                GeneratedField::Data(data) => {
                    writeln!(f, "/// `{}`\n///", data.uavcan_ty)?;
                    if data.always_aligned {
                        writeln!(f, "/// Always aligned")?;
                    } else {
                        writeln!(f, "/// Not always aligned")?;
                    }
                    let size = data.uavcan_ty.size();
                    let size_min = size.min_value();
                    let size_max = size.max_value();
                    if size_min == size_max {
                        writeln!(f, "/// Size {} bits", size_min)?;
                    } else {
                        writeln!(f, "/// Size ranges from {} to {} bits", size_min, size_max)?;
                    }
                    writeln!(f, "pub {}: {},", data.name, data.ty)
                }
                GeneratedField::Padding(bits) => {
                    writeln!(f, "// {} bits of padding", *bits)
                }
            }
        }
    }

    impl Display for GeneratedVariant {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            writeln!(f, "// {}", self.uavcan_ty)?;
            writeln!(f, "{}({}),", self.name, self.ty)
        }
    }

    impl Display for GeneratedModule {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            writeln!(
                f,
                r#"#[cfg(not(target_endian = "little"))] compile_error!("Zero-copy serialization requires a little-endian target");"#
            )?;
            assert!(
                self.tree.items.is_empty(),
                "Top-level types are not allowed"
            );
            for (sub_name, submodule) in &self.tree.children {
                // Adjust lints for every top-level module
                writeln!(
                    f,
                    "#[allow(unused_variables, unused_braces, unused_parens)]"
                )?;
                writeln!(f, "#[deny(unaligned_references)]")?;

                writeln!(f, "pub mod {} {{", sub_name)?;
                Display::fmt(submodule, f)?;
                writeln!(f, "}}")?;
            }

            Ok(())
        }
    }

    impl Display for GeneratedItem {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            match self {
                GeneratedItem::Type(ty) => Display::fmt(ty, f),
                GeneratedItem::Constant { name, ty, value } => {
                    writeln!(f, "pub const {}: {} = {};", name.type_name, ty, value)
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::external_module;
    use std::collections::BTreeMap;

    fn string_vec(strings: &[&str]) -> Vec<String> {
        strings.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn external_module_basic() {
        let mut modules: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
        modules.insert(
            string_vec(&["uavcan"]),
            string_vec(&["canadensis_data_types", "uavcan"]),
        );
        modules.insert(
            string_vec(&["uavcan", "more_specific"]),
            string_vec(&["more_specific_uavcan_module"]),
        );
        modules.insert(
            string_vec(&["uavcan", "more_specific", "even_more"]),
            string_vec(&["even_more_specific_uavcan_module"]),
        );

        assert_eq!(
            None,
            external_module(&["someing_else".into(), "sub".into()], &modules)
        );
        assert_eq!(
            Some(string_vec(&["canadensis_data_types", "uavcan"])),
            external_module(&["uavcan".into()], &modules)
        );
        assert_eq!(
            Some(string_vec(&[
                "canadensis_data_types",
                "uavcan",
                "general_submodule"
            ])),
            external_module(&["uavcan".into(), "general_submodule".into()], &modules)
        );
        assert_eq!(
            Some(string_vec(&["more_specific_uavcan_module"])),
            external_module(&["uavcan".into(), "more_specific".into()], &modules)
        );
        assert_eq!(
            Some(string_vec(&["even_more_specific_uavcan_module"])),
            external_module(
                &["uavcan".into(), "more_specific".into(), "even_more".into()],
                &modules
            )
        );
        assert_eq!(
            Some(string_vec(&["even_more_specific_uavcan_module", "sub"])),
            external_module(
                &[
                    "uavcan".into(),
                    "more_specific".into(),
                    "even_more".into(),
                    "sub".into()
                ],
                &modules
            )
        );
    }
}
